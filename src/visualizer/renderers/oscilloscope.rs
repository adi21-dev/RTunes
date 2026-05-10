//! Oscilloscope with phosphor persistence, zero-cross trigger, and Catmull–Rom densification.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::canvas::{Canvas, Context, Points};
use ratatui::symbols::Marker;
use ratatui::Frame;

use crate::tui::color::{dim_with_intensity, parse_hex};
use crate::visualizer::VisualizerData;

use super::canvas::{catmull_rom, glow_pass};
use super::phosphor::PhosphorBuffer;

/// First rising zero-crossing in the first half of `pcm`, or `0`.
pub fn zero_cross_start(pcm: &[f32]) -> usize {
    if pcm.len() < 2 {
        return 0;
    }
    let half = pcm.len() / 2;
    for i in 1..half {
        if pcm[i - 1] < 0.0 && pcm[i] >= 0.0 {
            return i;
        }
    }
    0
}

pub struct Oscilloscope {
    phosphor: PhosphorBuffer,
    spline: Vec<(f32, f32)>,
    /// Reusable coordinate buffers for canvas rendering.
    primary: Vec<(f64, f64)>,
    trail_pts: Vec<(f64, f64)>,
}

impl Oscilloscope {
    pub fn new() -> Self {
        Self {
            phosphor: PhosphorBuffer::new(0.85),
            spline: Vec::new(),
            primary: Vec::new(),
            trail_pts: Vec::new(),
        }
    }

    fn world_to_cell(wx: f64, wy: f64, w: u16, h: u16) -> (i32, i32) {
        let wf = w as f64;
        let hf = h as f64;
        if wf <= 0.0 || hf <= 0.0 {
            return (0, 0);
        }
        let gx = ((wx / wf) * (wf - 1.0)).round() as i32;
        let gy_from_bottom = ((wy / hf) * (hf - 1.0)).round() as i32;
        let iy = (h as i32 - 1) - gy_from_bottom;
        (gx.clamp(0, w as i32 - 1), iy.clamp(0, h as i32 - 1))
    }

    fn cell_to_world(ix: u16, iy: u16, h: u16) -> (f64, f64) {
        let wy = (h - 1 - iy) as f64 + 0.5;
        let wx = ix as f64 + 0.5;
        (wx, wy)
    }
}

impl crate::visualizer::Visualizer for Oscilloscope {
    fn render(
        &mut self,
        f: &mut Frame<'_>,
        area: Rect,
        data: Option<&VisualizerData>,
        _t: f32,
        rctx: &crate::visualizer::RendererCtx<'_>,
    ) {
        let theme = rctx.theme;
        let glow_on = rctx.glow;
        let w = area.width.max(1);
        let h = area.height.max(1);
        self.phosphor.ensure_size(w, h);
        self.phosphor.decay();

        let pcm = data.map(|d| d.pcm_mono.as_slice()).unwrap_or(&[]);
        if pcm.len() >= 2 {
            let start = zero_cross_start(pcm);
            let slice = &pcm[start..];
            if !slice.is_empty() {
                let n = slice.len().max(1);
                let wf = w as f32;
                let hf = h as f32;
                let mut pts: Vec<(f32, f32)> = Vec::with_capacity(n);
                for (i, &s) in slice.iter().enumerate() {
                    let y_norm = s.tanh() * 0.8;
                    let wx = if n <= 1 {
                        wf * 0.5
                    } else {
                        i as f32 * (wf - 1.0).max(1.0) / (n - 1) as f32
                    };
                    let wy = hf * 0.5 + y_norm * (hf * 0.5);
                    pts.push((wx, wy));
                }

                self.spline.clear();
                if pts.len() < 2 {
                    self.spline.extend_from_slice(&pts);
                } else if (pts.len() as u32) < u32::from(area.width).saturating_mul(2) {
                    self.spline = catmull_rom(&pts, 6);
                } else {
                    self.spline.clone_from(&pts);
                }

                for &(px, py) in &self.spline {
                    let (ix, iy) = Self::world_to_cell(f64::from(px), f64::from(py), w, h);
                    self.phosphor.paint(ix, iy, 1.0);
                }
            }
        }

        let bg = parse_hex(&theme.background);
        let vi = rctx.viz_intensity;
        let wave = dim_with_intensity(parse_hex(&theme.viz.wave_color), bg, vi);
        let trail = dim_with_intensity(parse_hex(&theme.viz.wave_trail), bg, vi);
        let wf = w as f64;
        let hf = h as f64;
        let theme_owned = theme.clone();

        // Reuse member buffers (clear preserves capacity — no heap alloc in steady state).
        self.primary.clear();
        self.trail_pts.clear();

        for (ix, iy, v) in self.phosphor.iter_lit(0.05) {
            let (wx, wy) = Self::cell_to_world(ix, iy, h);
            if v >= 0.35 {
                self.primary.push((wx, wy));
            } else {
                self.trail_pts.push((wx, wy));
            }
        }

        // Move coord slices into the closure (clone the small Vecs).
        let primary = self.primary.clone();
        let trail_pts = self.trail_pts.clone();

        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, wf])
            .y_bounds([0.0, hf])
            .background_color(bg)
            .paint(move |cctx: &mut Context<'_>| {
                if !trail_pts.is_empty() {
                    cctx.draw(&Points {
                        coords: &trail_pts,
                        color: trail,
                    });
                }
                if !primary.is_empty() {
                    cctx.draw(&Points {
                        coords: &primary,
                        color: wave,
                    });
                }
                if glow_on && !primary.is_empty() {
                    glow_pass(
                        cctx,
                        w,
                        h,
                        [0.0, wf],
                        [0.0, hf],
                        &primary,
                        &theme_owned,
                        wave,
                        glow_on,
                    );
                }
            });

        f.render_widget(
            canvas.block(Block::default().style(Style::default().bg(bg))),
            area,
        );
        crate::visualizer::maybe_draw_viz_baseline(f, area, rctx);
    }

    fn reset(&mut self) {
        self.spline.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    use crate::app::state::SpectrogramMode;
    use crate::config::theme;
    use crate::visualizer::RendererCtx;

    #[test]
    fn oscilloscope_no_panic_with_empty_pcm() {
        let mut osc = Oscilloscope::new();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        let mut data = VisualizerData::empty(64);
        data.pcm_mono.clear();
        let th = theme::builtin_themes()
            .get("synthwave")
            .cloned()
            .expect("synthwave builtin");
        let ctx = RendererCtx {
            theme: &th,
            fullscreen: false,
            glow: crate::config::theme::effective_glow(&th, true),
            spectrogram_mode: SpectrogramMode::Standard,
            viz_intensity: 1.0,
            baseline: false,
        };
        term
            .draw(|f| {
                crate::visualizer::Visualizer::render(&mut osc, f, area, Some(&data), 0.5, &ctx);
            })
            .unwrap();
    }

    #[test]
    fn oscilloscope_zero_crossing_finds_rising_edge() {
        let pcm = vec![-1.0, -0.5, 0.0, 0.5, 1.0, 0.5];
        assert_eq!(zero_cross_start(&pcm), 2);
    }

    #[test]
    fn oscilloscope_phosphor_decays_between_frames() {
        let mut p = PhosphorBuffer::new(0.85);
        p.ensure_size(10, 10);
        p.paint(5, 5, 1.0);
        p.decay();
        let v = p
            .iter_lit(0.0)
            .find(|(x, y, _)| *x == 5 && *y == 5)
            .map(|(_, _, i)| i)
            .unwrap_or(0.0);
        assert!((v - 0.85).abs() < 1e-5);
    }
}
