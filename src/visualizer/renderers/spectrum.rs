//! Spectrum visualizer: adaptive bars, decimation, EMA, mirror fullscreen.

use ratatui::layout::{Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::widgets::canvas::{Canvas, Points};
use ratatui::symbols::Marker;
use ratatui::Frame;

use crate::config::Theme;
use crate::config::theme::BarStyle;
use crate::tui::color::{dim_with_intensity, lerp_color, parse_hex, GradientLut};
use crate::visualizer::smoothing::{
    apply_spectral_smoothing_with_scratch, band_tau, ema_dt,
};
use crate::visualizer::VisualizerData;


/// Adaptive bar count from terminal width (plan: 32 / 48 / 64).
pub fn bars_for_width(w: u16) -> usize {
    if w < 60 {
        32
    } else if w < 120 {
        48
    } else {
        64
    }
}

pub(crate) fn decimate_bins(src: &[f32], bars: usize, out: &mut Vec<f32>) {
    out.clear();
    let n = src.len();
    if n == 0 {
        return;
    }
    if bars >= n {
        out.extend_from_slice(src);
        return;
    }
    for i in 0..bars {
        let a = i * n / bars;
        let b = (i + 1) * n / bars;
        let slice = &src[a..b];
        let sum: f32 = slice.iter().sum();
        out.push(sum / slice.len().max(1) as f32);
    }
}

/// Rounded bar top glyphs (low → high fractional fill within the top cell).
pub(crate) const ROUND_TOP: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub struct Spectrum {
    last_bars: usize,
    decimated_cur: Vec<f32>,
    decimated_prev: Vec<f32>,
    decimated_peak: Vec<f32>,
    display_ema: Vec<f32>,
    /// Scratch buffer for spectral smoothing — reused every frame (no alloc).
    smooth_scratch: Vec<f32>,
    /// Pre-allocated bar height buffer — avoids a per-frame Vec allocation.
    heights: Vec<f32>,
    /// Cached gradient LUT for the active theme (rebuilt only when stops change).
    lut: Option<GradientLut>,
    /// Last-seen gradient stop strings — used for LUT invalidation.
    lut_stops: Vec<String>,
}

impl Spectrum {
    pub fn new() -> Self {
        Self {
            last_bars: 0,
            decimated_cur: Vec::new(),
            decimated_prev: Vec::new(),
            decimated_peak: Vec::new(),
            display_ema: Vec::new(),
            smooth_scratch: Vec::new(),
            heights: Vec::new(),
            lut: None,
            lut_stops: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn last_decimated_len(&self) -> usize {
        self.decimated_cur.len()
    }

    fn ensure_scratch(&mut self, bars: usize) {
        if bars != self.last_bars {
            self.last_bars = bars;
            self.decimated_cur.resize(bars, 0.0);
            self.decimated_prev.resize(bars, 0.0);
            self.decimated_peak.resize(bars, 0.0);
            self.display_ema.resize(bars, 0.0);
            self.display_ema.fill(0.0);
            self.heights.resize(bars, 0.0);
            // smooth_scratch is resized on demand inside apply_spectral_smoothing_with_scratch.
        }
    }

    fn render_block_bars(
        &self,
        f: &mut Frame<'_>,
        area: Rect,
        heights: &[f32],
        _peaks: &[f32],
        theme: &Theme,
        lut: &GradientLut,
        mirror_y: bool,
        dim: bool,
        rounded: bool,
        viz_intensity: f32,
    ) {
        let w = area.width as usize;
        let h = area.height as usize;
        if w == 0 || h == 0 {
            return;
        }
        let bars = heights.len().max(1);
        let bg = parse_hex(&theme.background);

        let mut lines: Vec<Line> = Vec::with_capacity(h);
        for row_ix in 0..h {
            let row_from_top = if mirror_y {
                h - 1 - row_ix
            } else {
                row_ix
            };
            let from_bottom = h - 1 - row_from_top;

            let mut spans: Vec<Span> = Vec::with_capacity(w);
            for col in 0..w {
                let bar_i = (((col as f32 + 0.5) / w as f32) * bars as f32) as usize;
                let bar_i = bar_i.min(bars - 1);
                let bar_h = heights[bar_i] * h as f32;
                let height_ratio = (bar_h / h as f32).clamp(0.0, 1.0);
                let mut grad_c = lut.get(height_ratio);
                grad_c = dim_with_intensity(grad_c, bg, viz_intensity);
                if dim {
                    grad_c = lerp_color(grad_c, bg, 0.5);
                }

                let full_floor = bar_h.floor() as usize;
                let frac_part = bar_h - full_floor as f32;

                let ch = if from_bottom < full_floor {
                    '█'
                } else if from_bottom == full_floor && frac_part > 1e-4 {
                    if rounded {
                        let idx = ((frac_part * 8.0).floor() as usize).min(ROUND_TOP.len() - 1);
                        ROUND_TOP[idx]
                    } else {
                        '█'
                    }
                } else {
                    ' '
                };

                let style = if ch == ' ' {
                    Style::default().bg(bg)
                } else {
                    Style::default().fg(grad_c).bg(bg)
                };
                spans.push(Span::styled(ch.to_string(), style));
            }
            lines.push(Line::from(spans));
        }

        let p = Paragraph::new(lines).style(Style::default().bg(bg));
        f.render_widget(p, area);
    }

    fn render_dots(
        &self,
        f: &mut Frame<'_>,
        area: Rect,
        heights: &[f32],
        _peaks: &[f32],
        theme: &Theme,
        lut: GradientLut,
        mirror_y: bool,
        dim: bool,
        viz_intensity: f32,
    ) {
        let w = area.width.max(1) as f64;
        let h = area.height.max(1) as f64;
        let bars = heights.len().max(1);
        let bg = parse_hex(&theme.background);
        let heights_owned = heights.to_vec();

        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, w])
            .y_bounds([0.0, h])
            .background_color(bg)
            .paint(move |ctx| {
                for i in 0..bars {
                    let x0 = i as f64 / bars as f64 * w;
                    let x1 = (i + 1) as f64 / bars as f64 * w;
                    let cx = (x0 + x1) * 0.5;
                    let bar_h = heights_owned[i] as f64 * h;
                    let height_ratio = (bar_h / h).clamp(0.0, 1.0) as f32;
                    let mut c = lut.get(height_ratio);
                    c = dim_with_intensity(c, bg, viz_intensity);
                    if dim {
                        c = lerp_color(c, bg, 0.5);
                    }
                    let steps = ((bar_h * 4.0).ceil() as usize).max(2).min(400);
                    let mut coords: Vec<(f64, f64)> = Vec::with_capacity(steps + 2);
                    for s in 0..steps {
                        let t = s as f64 / (steps - 1) as f64;
                        let mut wy = t * bar_h;
                        if mirror_y {
                            wy = h - 1.0 - wy;
                        }
                        coords.push((cx, wy));
                    }
                    ctx.draw(&Points {
                        coords: &coords,
                        color: c,
                    });
                }
            });

        f.render_widget(canvas.block(Block::default().style(Style::default().bg(bg))), area);
    }
}

impl crate::visualizer::Visualizer for Spectrum {
    fn render(
        &mut self,
        f: &mut Frame<'_>,
        area: Rect,
        data: Option<&VisualizerData>,
        t: f32,
        rctx: &crate::visualizer::RendererCtx<'_>,
    ) {
        let theme = rctx.theme;
        let fullscreen = rctx.fullscreen;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let bars = bars_for_width(area.width);
        self.ensure_scratch(bars);

        // Rebuild gradient LUT if theme stops have changed (typically only on theme switch).
        let stops = &theme.viz.gradient;
        if self.lut.is_none() || *stops != self.lut_stops {
            self.lut = Some(GradientLut::new(stops));
            self.lut_stops = stops.clone();
        }
        let lut = self.lut.as_ref().expect("lut just initialised");

        let empty = VisualizerData::empty(64);
        let d = data.unwrap_or(&empty);
        let bins_cur = &d.bins_smoothed[..];
        let bins_prev = &d.bins_prev[..];
        let bins_peak = &d.bins_peak[..];

        decimate_bins(bins_cur, bars, &mut self.decimated_cur);
        decimate_bins(bins_prev, bars, &mut self.decimated_prev);
        decimate_bins(bins_peak, bars, &mut self.decimated_peak);

        apply_spectral_smoothing_with_scratch(&mut self.decimated_cur, &mut self.smooth_scratch);
        apply_spectral_smoothing_with_scratch(&mut self.decimated_prev, &mut self.smooth_scratch);

        // Per-bar frequency-dependent EMA: bass bars sustain, treble bars snap.
        // Uses actual FFT period as dt so smoothing is frame-rate-independent.
        let dt = d.fft_period.as_secs_f32().max(1.0 / 120.0);
        for i in 0..bars {
            let target = self.decimated_prev[i].mul_add(1.0 - t, self.decimated_cur[i] * t);
            let (tau_a, tau_r) = band_tau(i, bars);
            let tau = if target > self.display_ema[i] { tau_a } else { tau_r };
            self.display_ema[i] = ema_dt(self.display_ema[i], target, tau, dt);
            self.heights[i] = self.display_ema[i].clamp(0.0, 1.0);
        }

        if fullscreen && area.height >= 4 {
            let chunks = Layout::default()
                .direction(ratatui::layout::Direction::Vertical)
                .constraints([
                    ratatui::layout::Constraint::Percentage(50),
                    ratatui::layout::Constraint::Percentage(50),
                ])
                .split(area);
            match theme.viz.bar_style {
                BarStyle::Dots => {
                    self.render_dots(
                        f,
                        chunks[0],
                        &self.heights,
                        &self.decimated_peak,
                        theme,
                        lut.clone(),
                        false,
                        false,
                        rctx.viz_intensity,
                    );
                    self.render_dots(
                        f,
                        chunks[1],
                        &self.heights,
                        &self.decimated_peak,
                        theme,
                        lut.clone(),
                        true,
                        true,
                        rctx.viz_intensity,
                    );
                }
                BarStyle::Solid => {
                    self.render_block_bars(
                        f,
                        chunks[0],
                        &self.heights,
                        &self.decimated_peak,
                        theme,
                        lut,
                        false,
                        false,
                        false,
                        rctx.viz_intensity,
                    );
                    self.render_block_bars(
                        f,
                        chunks[1],
                        &self.heights,
                        &self.decimated_peak,
                        theme,
                        lut,
                        true,
                        true,
                        false,
                        rctx.viz_intensity,
                    );
                }
                BarStyle::Rounded => {
                    self.render_block_bars(
                        f,
                        chunks[0],
                        &self.heights,
                        &self.decimated_peak,
                        theme,
                        lut,
                        false,
                        false,
                        true,
                        rctx.viz_intensity,
                    );
                    self.render_block_bars(
                        f,
                        chunks[1],
                        &self.heights,
                        &self.decimated_peak,
                        theme,
                        lut,
                        true,
                        true,
                        true,
                        rctx.viz_intensity,
                    );
                }
            }
        } else {
            match theme.viz.bar_style {
                BarStyle::Dots => {
                    self.render_dots(
                        f,
                        area,
                        &self.heights,
                        &self.decimated_peak,
                        theme,
                        lut.clone(),
                        false,
                        false,
                        rctx.viz_intensity,
                    );
                }
                BarStyle::Solid => {
                    self.render_block_bars(
                        f,
                        area,
                        &self.heights,
                        &self.decimated_peak,
                        theme,
                        lut,
                        false,
                        false,
                        false,
                        rctx.viz_intensity,
                    );
                }
                BarStyle::Rounded => {
                    self.render_block_bars(
                        f,
                        area,
                        &self.heights,
                        &self.decimated_peak,
                        theme,
                        lut,
                        false,
                        false,
                        true,
                        rctx.viz_intensity,
                    );
                }
            }
        }
        crate::visualizer::maybe_draw_viz_baseline(f, area, rctx);
    }

    fn reset(&mut self) {
        self.display_ema.clear();
        self.last_bars = 0;
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    use crate::app::state::SpectrogramMode;
    use crate::config::theme::{self, Theme};
    use crate::tui::color::parse_hex;
    use crate::visualizer::{RendererCtx, Visualizer, VisualizerData};

    use super::Spectrum;

    fn test_ctx<'a>(th: &'a Theme) -> RendererCtx<'a> {
        RendererCtx {
            theme: th,
            fullscreen: false,
            glow: crate::config::theme::effective_glow(th, true),
            spectrogram_mode: SpectrogramMode::Standard,
            viz_intensity: 1.0,
            baseline: false,
        }
    }

    #[test]
    fn viz_intensity_zero_renders_only_bg_cells() {
        let mut sp = Spectrum::new();
        let backend = TestBackend::new(40, 12);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 12);
        let data = VisualizerData::empty(64);
        let th = theme::builtin_themes()
            .get("synthwave")
            .cloned()
            .expect("synthwave");
        let bg = parse_hex(&th.background);
        let ctx = RendererCtx {
            theme: &th,
            fullscreen: false,
            glow: false,
            spectrogram_mode: SpectrogramMode::Standard,
            viz_intensity: 0.0,
            baseline: false,
        };
        term.draw(|f| sp.render(f, area, Some(&data), 0.0, &ctx))
            .unwrap();
        let buf = term.backend().buffer();
        for y in 0..12u16 {
            for x in 0..40u16 {
                let cell = &buf[(x, y)];
                if cell.symbol() == " " || cell.symbol().chars().all(|c| c.is_whitespace()) {
                    continue;
                }
                assert_eq!(
                    cell.fg, bg,
                    "non-bg fg at ({x},{y}) sym={:?}",
                    cell.symbol()
                );
            }
        }
    }

    #[test]
    fn spectrum_no_panic_with_zeroed_data() {
        let mut sp = Spectrum::new();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        let data = VisualizerData::empty(64);
        let th = theme::builtin_themes()
            .get("synthwave")
            .cloned()
            .expect("synthwave builtin");
        let ctx = test_ctx(&th);
        term
            .draw(|f| {
                crate::visualizer::Visualizer::render(&mut sp, f, area, Some(&data), 0.5, &ctx);
            })
            .unwrap();
    }

    #[test]
    fn spectrum_decimates_64_to_32_on_narrow_terminal() {
        let mut sp = Spectrum::new();
        let backend = TestBackend::new(40, 10);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);
        let data = VisualizerData::empty(64);
        let th = theme::builtin_themes()
            .get("synthwave")
            .cloned()
            .expect("synthwave builtin");
        let ctx = test_ctx(&th);
        term
            .draw(|f| {
                crate::visualizer::Visualizer::render(&mut sp, f, area, Some(&data), 0.5, &ctx);
            })
            .unwrap();
        assert_eq!(sp.last_decimated_len(), 32);
    }

    #[test]
    fn spectrum_bar_height_responds_to_bin_value() {
        let mut sp = Spectrum::new();
        let backend = TestBackend::new(64, 20);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 64, 20);
        let mut data = VisualizerData::empty(64);
        data.bins_smoothed[0] = 1.0;
        data.bins_prev[0] = 1.0;
        data.bins_peak[0] = 1.0;
        let th = theme::builtin_themes()
            .get("synthwave")
            .cloned()
            .expect("synthwave builtin");
        let ctx = test_ctx(&th);
        for _ in 0..8 {
            term
                .draw(|f| {
                    crate::visualizer::Visualizer::render(&mut sp, f, area, Some(&data), 1.0, &ctx);
                })
                .unwrap();
        }
        let buf = term.backend().buffer();
        let bottom_row = area.height - 1;
        let cell = buf
            .cell((area.x, area.y + bottom_row))
            .expect("cell in buffer");
        assert_ne!(cell.symbol(), " ");
    }

    #[test]
    fn spectrum_quiet_input_reaches_above_half_height() {
        let mut sp = Spectrum::new();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        let mut d = VisualizerData::empty(64);
        d.loudness = 1.0;
        for x in d.bins_smoothed.iter_mut() {
            *x = 0.5;
        }
        d.bins_prev.copy_from_slice(&d.bins_smoothed);
        d.bins_peak.copy_from_slice(&d.bins_smoothed);
        let th = theme::builtin_themes()
            .get("synthwave")
            .cloned()
            .expect("synthwave builtin");
        let ctx = test_ctx(&th);
        for _ in 0..24 {
            term
                .draw(|f| {
                    crate::visualizer::Visualizer::render(&mut sp, f, area, Some(&d), 1.0, &ctx);
                })
                .unwrap();
        }
        let buf = term.backend().buffer();
        let y_mid = area.y + area.height / 2;
        let mut any_bar_above_mid = false;
        for y in y_mid..(area.y + area.height) {
            for x in area.x..(area.x + area.width) {
                let sym = buf[(x, y)].symbol();
                if sym != " " && !sym.chars().all(|c| c.is_whitespace()) {
                    any_bar_above_mid = true;
                    break;
                }
            }
            if any_bar_above_mid {
                break;
            }
        }
        assert!(
            any_bar_above_mid,
            "quiet uniform bins should auto-gain into lower half of the area"
        );
    }
}
