//! Radial “supernova” spokes driven by spectrum bins and bass-reactive core.

use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::canvas::{Canvas, Context, Line, Points};
use ratatui::symbols::Marker;
use ratatui::Frame;

use crate::tui::color::{dim_with_intensity, parse_hex};
use crate::visualizer::smoothing::{
    apply_spectral_smoothing_with_scratch, treble_tilt, AutoGain, OneEuroFilter,
};
use crate::visualizer::VisualizerData;

use super::canvas::{glow_pass, gradient_color};

const SPOKES: usize = 32;

/// Inner breathing radius `r0` from bass (plan §8).
pub(crate) fn inner_radius_r0(bass: f32, w: u16, h: u16) -> f32 {
    let r_max = w.min(h) as f32 * 0.45;
    let r_min = r_max * 0.12;
    r_min + bass * (r_min * 0.5)
}

fn decimate_bins_32(src: &[f32], out: &mut Vec<f32>) {
    out.clear();
    let n = src.len();
    for k in 0..SPOKES {
        let i = k * 2;
        let v = if i + 1 < n {
            (src[i] + src[i + 1]) * 0.5
        } else if i < n {
            src[i]
        } else {
            0.0
        };
        out.push(v);
    }
}

pub struct Supernova {
    euro: OneEuroFilter,
    phase_deg: f32,
    cur32: Vec<f32>,
    prev32: Vec<f32>,
    auto_gain: AutoGain,
    /// Reusable coordinate buffers for canvas rendering.
    outer_pts: Vec<(f64, f64)>,
    core_pts: Vec<(f64, f64)>,
    glow_src: Vec<(f64, f64)>,
    /// Scratch buffer for zero-alloc spectral smoothing (reused each frame).
    smooth_scratch: Vec<f32>,
}

impl Supernova {
    pub fn new() -> Self {
        Self {
            euro: OneEuroFilter::new(30.0, 1.0, 0.007),
            phase_deg: 0.0,
            cur32: vec![0.0; SPOKES],
            prev32: vec![0.0; SPOKES],
            auto_gain: AutoGain::new(),
            outer_pts: Vec::with_capacity(SPOKES),
            core_pts: Vec::with_capacity(64),
            glow_src: Vec::with_capacity(SPOKES + 64),
            smooth_scratch: Vec::new(),
        }
    }

    /// Decimate, tilt, spectrally smooth, and update auto-gain from current bins.
    pub(crate) fn update_decimated_spokes(&mut self, d: &VisualizerData) -> f32 {
        decimate_bins_32(&d.bins_smoothed, &mut self.cur32);
        decimate_bins_32(&d.bins_prev, &mut self.prev32);
        treble_tilt(&mut self.cur32, 0.6);
        treble_tilt(&mut self.prev32, 0.6);
        apply_spectral_smoothing_with_scratch(&mut self.cur32, &mut self.smooth_scratch);
        apply_spectral_smoothing_with_scratch(&mut self.prev32, &mut self.smooth_scratch);
        self.auto_gain.apply(&self.cur32)
    }
}

impl crate::visualizer::Visualizer for Supernova {
    fn render(
        &mut self,
        f: &mut Frame<'_>,
        area: Rect,
        data: Option<&VisualizerData>,
        t: f32,
        rctx: &crate::visualizer::RendererCtx<'_>,
    ) {
        let theme = rctx.theme;
        let glow_on = rctx.glow;
        let w = area.width.max(1);
        let h = area.height.max(1);
        let wf = w as f64;
        let hf = h as f64;
        let bg = parse_hex(&theme.background);
        let vi = rctx.viz_intensity;
        let cx = wf * 0.5;
        let cy = hf * 0.5;

        let empty = VisualizerData::empty(64);
        let d = data.unwrap_or(&empty);
        let r_max = w.min(h) as f32 * 0.45;
        let r0 = inner_radius_r0(d.bass_energy, w, h) as f64;
        let spoke_scale = (f64::from(r_max) - r0).max(0.0);

        let now = Instant::now();
        let mid = self.euro.filter(d.mid_energy, now);
        self.phase_deg = (self.phase_deg + 0.3 + mid * 1.5).rem_euclid(360.0);

        let g = self.update_decimated_spokes(d);

        let core_color = dim_with_intensity(
            theme
                .viz
                .particle_colors
                .first()
                .map(|s| parse_hex(s))
                .filter(|c| *c != ratatui::style::Color::Reset)
                .unwrap_or_else(|| parse_hex(&theme.primary)),
            bg,
            vi,
        );

        let mut outer_pts: Vec<(f64, f64)> = Vec::with_capacity(SPOKES);
        let mut lines: Vec<Line> = Vec::with_capacity(SPOKES);

        for k in 0..SPOKES {
            let theta = (self.phase_deg + (k as f32) * (360.0 / SPOKES as f32)).to_radians();
            let (st, ct) = (f64::from(theta.sin()), f64::from(theta.cos()));
            let mix = self.prev32[k].mul_add(1.0 - t, self.cur32[k] * t);
            let len = (f64::from(mix) * f64::from(g) * spoke_scale).max(0.0);
            let x1 = cx + r0 * ct;
            let y1 = cy + r0 * st;
            let x2 = cx + (r0 + len) * ct;
            let y2 = cy + (r0 + len) * st;
            let col = dim_with_intensity(
                gradient_color(&theme.viz.gradient, k as f32 / SPOKES as f32),
                bg,
                vi,
            );
            lines.push(Line {
                x1,
                y1,
                x2,
                y2,
                color: col,
            });
            outer_pts.push((x2, y2));
        }

        let mut core_pts: Vec<(f64, f64)> = Vec::with_capacity(64);
        for i in 0..64 {
            let ang = (i as f64) * std::f64::consts::TAU / 64.0;
            core_pts.push((cx + r0 * ang.cos(), cy + r0 * ang.sin()));
        }

        let glow_src: Vec<(f64, f64)> = outer_pts.iter().chain(core_pts.iter()).copied().collect();

        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, wf])
            .y_bounds([0.0, hf])
            .background_color(bg)
            .paint(move |ctx: &mut Context<'_>| {
                ctx.draw(&Points {
                    coords: &core_pts,
                    color: core_color,
                });
                for ln in &lines {
                    ctx.draw(ln);
                }
                if glow_on {
                    glow_pass(
                        ctx,
                        w,
                        h,
                        [0.0, wf],
                        [0.0, hf],
                        &glow_src,
                        bg,
                        core_color,
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
        self.auto_gain.reset();
        self.cur32.fill(0.0);
        self.prev32.fill(0.0);
        self.phase_deg = 0.0;
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
    use crate::visualizer::{RendererCtx, VisualizerData};

    #[test]
    fn supernova_no_panic_with_zeroed_data() {
        let mut sn = Supernova::new();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        let data = VisualizerData::empty(64);
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
                crate::visualizer::Visualizer::render(&mut sn, f, area, Some(&data), 0.5, &ctx);
            })
            .unwrap();
    }

    #[test]
    fn supernova_inner_radius_grows_with_bass() {
        let r_lo = inner_radius_r0(0.0, 80, 24);
        let r_hi = inner_radius_r0(1.0, 80, 24);
        assert!(r_hi > r_lo);
    }

    #[test]
    fn supernova_quiet_bins_produce_visible_spokes() {
        let mut sn = Supernova::new();
        let w = 80u16;
        let h = 24u16;
        let t = 1.0f32;
        let mut d = VisualizerData::empty(64);
        d.loudness = 1.0;
        for x in d.bins_smoothed.iter_mut() {
            *x = 0.1;
        }
        d.bins_prev.copy_from_slice(&d.bins_smoothed);
        sn.update_decimated_spokes(&d);
        let g = sn.update_decimated_spokes(&d);
        let r_max = w.min(h) as f32 * 0.45;
        let r0 = inner_radius_r0(d.bass_energy, w, h) as f64;
        let spoke_scale = (f64::from(r_max) - r0).max(0.0);
        let mut visible = 0usize;
        for k in 0..SPOKES {
            let mix = sn.prev32[k].mul_add(1.0 - t, sn.cur32[k] * t);
            let len = (f64::from(mix) * f64::from(g) * spoke_scale).max(0.0);
            if len > 1e-2 {
                visible += 1;
            }
        }
        assert!(
            visible >= 16,
            "quiet uniform spectrum should normalize so most spokes extend: visible={visible}"
        );
    }
}
