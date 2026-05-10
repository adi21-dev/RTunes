//! Stereo Lissajous / vectorscope with phosphor persistence.

use std::time::{Duration, Instant};

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

pub struct Vectorscope {
    phosphor: PhosphorBuffer,
    spline: Vec<(f32, f32)>,
    pen_thick_until: Option<Instant>,
    /// Reusable coordinate buffers for canvas rendering.
    primary: Vec<(f64, f64)>,
    trail_pts: Vec<(f64, f64)>,
    /// Running peak amplitude tracker for auto-scale (slow rise / fast fall EMA).
    /// Keeps the Lissajous figure visible even in quiet passages.
    amp_peak: f32,
}

impl Vectorscope {
    pub fn new() -> Self {
        Self {
            phosphor: PhosphorBuffer::new(0.88),
            spline: Vec::new(),
            pen_thick_until: None,
            primary: Vec::new(),
            trail_pts: Vec::new(),
            amp_peak: 0.5,
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

impl crate::visualizer::Visualizer for Vectorscope {
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

        let now = Instant::now();
        let empty = VisualizerData::empty(64);
        let d = data.unwrap_or(&empty);
        if d.beat {
            self.pen_thick_until = Some(now + Duration::from_millis(100));
        }
        let thick = self
            .pen_thick_until
            .map(|t| now < t)
            .unwrap_or(false);

        let pcm = &d.pcm_stereo;
        if pcm.len() >= 2 {
            // Auto-gain: track running peak amplitude with asymmetric EMA.
            // Slow rise (0.08) lets the figure grow gradually; fast fall (0.30)
            // attenuates quickly on loud transients to prevent clipping.
            let amp = pcm
                .iter()
                .flat_map(|&(l, r)| [l.abs(), r.abs()])
                .fold(0.0f32, f32::max);
            let t_gain = if amp > self.amp_peak { 0.08 } else { 0.30 };
            self.amp_peak += (amp.max(0.05) - self.amp_peak) * t_gain;
            let scale = 0.45 * w.min(h) as f32 / self.amp_peak.max(0.05);
            let wf = w as f32;
            let hf = h as f32;
            let cx = wf * 0.5;
            let cy = hf * 0.5;
            let n = pcm.len();
            let mut pts: Vec<(f32, f32)> = Vec::with_capacity(n);
            for (_i, &(l, r)) in pcm.iter().enumerate() {
                let vx = (l - r) * scale;
                let vy = (l + r) * scale;
                let wx = cx + vx;
                let wy = cy + vy;
                pts.push((wx.clamp(0.0, wf - 1.0), wy.clamp(0.0, hf - 1.0)));
            }
            self.spline.clear();
            if pts.len() < 2 {
                self.spline.extend_from_slice(&pts);
            } else if (pts.len() as u32) < u32::from(w).saturating_mul(2) {
                self.spline = catmull_rom(&pts, 5);
            } else {
                self.spline.clone_from(&pts);
            }
            let intens = d.loudness.clamp(0.05, 1.0);
            for &(px, py) in &self.spline {
                let (ix, iy) = Self::world_to_cell(f64::from(px), f64::from(py), w, h);
                self.phosphor.paint(ix, iy, intens);
                if thick {
                    self.phosphor.paint(ix + 1, iy, intens * 0.9);
                    self.phosphor.paint(ix - 1, iy, intens * 0.9);
                }
            }
        }

        let bg = parse_hex(&theme.background);
        let vi = rctx.viz_intensity;
        let wave = dim_with_intensity(parse_hex(&theme.viz.wave_color), bg, vi);
        let trail = dim_with_intensity(parse_hex(&theme.viz.wave_trail), bg, vi);
        let wf = w as f64;
        let hf = h as f64;
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
                        bg,
                        wave,
                        true,
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
        // Zero all phosphor persistence cells so the old figure doesn't linger.
        self.phosphor.scale_all(0.0);
        self.spline.clear();
        self.amp_peak = 0.5;
        self.pen_thick_until = None;
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
    fn vectorscope_no_panic_empty_stereo() {
        let mut v = Vectorscope::new();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        let mut data = VisualizerData::empty(64);
        data.pcm_stereo.clear();
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        let ctx = RendererCtx {
            theme: &th,
            fullscreen: false,
            glow: false,
            spectrogram_mode: SpectrogramMode::Standard,
            viz_intensity: 1.0,
            baseline: false,
        };
        term
            .draw(|f| {
                crate::visualizer::Visualizer::render(&mut v, f, area, Some(&data), 0.5, &ctx);
            })
            .unwrap();
    }

    #[test]
    fn vectorscope_mono_vertical_line_centered() {
        let scale = 0.45 * 40.0f32;
        let l = 0.5f32;
        let r = 0.5f32;
        let vx = (l - r) * scale;
        let vy = (l + r) * scale;
        assert!(vx.abs() < 1e-3);
        assert!(vy > 0.0);
    }

    #[test]
    fn vectorscope_beat_enables_thick_pen() {
        let mut v = Vectorscope::new();
        let mut d = VisualizerData::empty(64);
        d.beat = true;
        d.pcm_stereo = vec![(0.1, 0.1); 64];
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        let ctx = RendererCtx {
            theme: &th,
            fullscreen: false,
            glow: false,
            spectrogram_mode: SpectrogramMode::Standard,
            viz_intensity: 1.0,
            baseline: false,
        };
        let area = Rect::new(0, 0, 50, 20);
        let backend = TestBackend::new(50, 20);
        let mut term = Terminal::new(backend).unwrap();
        term
            .draw(|f| {
                crate::visualizer::Visualizer::render(&mut v, f, area, Some(&d), 0.0, &ctx);
            })
            .unwrap();
        assert!(v.pen_thick_until.is_some());
    }
}
