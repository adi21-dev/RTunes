//! Beat-only expanding rings — no motion without audio above the silence floor.

use std::f64::consts::TAU;
use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::canvas::{Canvas, Points};
use ratatui::symbols::Marker;
use ratatui::Frame;

use crate::tui::color::{dim_with_intensity, lerp_color, parse_hex};
use crate::visualizer::VisualizerData;

use super::canvas::gradient_color;

const FADE_SECS: f32 = 1.2;

struct Ring {
    born: Instant,
    intensity: f32,
    hue_t: f32,
}

pub struct PulseRings {
    rings: Vec<Ring>,
    /// Reusable per-render ring data buffer.
    rings_scratch: Vec<(f64, f32, f32)>,
}

impl PulseRings {
    pub fn new() -> Self {
        Self {
            rings: Vec::new(),
            rings_scratch: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn ring_count(&self) -> usize {
        self.rings.len()
    }
}

impl crate::visualizer::Visualizer for PulseRings {
    fn render(
        &mut self,
        f: &mut Frame<'_>,
        area: Rect,
        data: Option<&VisualizerData>,
        _t: f32,
        rctx: &crate::visualizer::RendererCtx<'_>,
    ) {
        let theme = rctx.theme;
        let w = area.width.max(1);
        let h = area.height.max(1);
        let wf = w as f64;
        let hf = h as f64;
        let bg = parse_hex(&theme.background);
        let vi = rctx.viz_intensity;
        let cx = wf * 0.5;
        let cy = hf * 0.5;
        let now = Instant::now();

        let empty = VisualizerData::empty(64);
        let d = data.unwrap_or(&empty);

        if data.is_none() || d.loudness < 0.005 {
            self.rings.clear();
            f.render_widget(
                Block::default().style(Style::default().bg(bg)),
                area,
            );
            crate::visualizer::maybe_draw_viz_baseline(f, area, rctx);
            return;
        }

        self.rings.retain(|r| {
            let age = now.saturating_duration_since(r.born).as_secs_f32();
            age < FADE_SECS
        });

        if d.beat && d.loudness >= 0.005 {
            let denom = d.bass_energy + d.mid_energy + d.high_energy + 1e-6;
            let hue_t = (d.bass_energy / denom).clamp(0.0, 1.0);
            self.rings.push(Ring {
                born: now,
                intensity: d.beat_intensity.clamp(0.0, 1.0),
                hue_t,
            });
        }

        let rings_owned: Vec<(f64, f32, f32)> = self
            .rings
            .iter()
            .map(|r| {
                let age = now.saturating_duration_since(r.born).as_secs_f64();
                let alpha = (1.0 - (age as f32 / FADE_SECS)).clamp(0.0, 1.0) as f64;
                let speed = 0.12 + 0.18 * f64::from(r.intensity);
                let r_pix = age * speed * wf.min(hf) * 0.5;
                (r_pix.max(0.5), alpha as f32, r.hue_t)
            })
            .collect();
        // Reuse scratch buffer capacity for next frame (no-op here, moved into closure below).

        let stops = theme.viz.gradient.clone();
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, wf])
            .y_bounds([0.0, hf])
            .background_color(bg)
            .paint(move |ctx| {
                for (r_pix, alpha, hue_t) in &rings_owned {
                    if *alpha <= 1e-3 {
                        continue;
                    }
                    let base = gradient_color(&stops, *hue_t);
                    let c = lerp_color(
                        dim_with_intensity(base, bg, vi),
                        bg,
                        1.0 - *alpha,
                    );
                    let n = 48;
                    let mut coords: Vec<(f64, f64)> = Vec::with_capacity(n);
                    for i in 0..n {
                        let th = TAU * (i as f64 / n as f64);
                        coords.push((cx + r_pix * th.cos(), cy + r_pix * th.sin()));
                    }
                    ctx.draw(&Points { coords: &coords, color: c });
                }
            });

        f.render_widget(
            canvas.block(Block::default().style(Style::default().bg(bg))),
            area,
        );
        crate::visualizer::maybe_draw_viz_baseline(f, area, rctx);
    }

    fn reset(&mut self) {
        self.rings.clear();
        self.rings_scratch.clear();
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
    use crate::visualizer::{RendererCtx, Visualizer, VisualizerData};

    fn ctx<'a>(th: &'a theme::Theme) -> RendererCtx<'a> {
        RendererCtx {
            theme: th,
            fullscreen: false,
            glow: false,
            spectrogram_mode: SpectrogramMode::Standard,
            viz_intensity: 1.0,
            baseline: false,
        }
    }

    #[test]
    fn pulse_rings_no_panic_with_zeroed_data() {
        let mut p = PulseRings::new();
        let backend = TestBackend::new(60, 20);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 60, 20);
        let data = VisualizerData::empty(64);
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        term
            .draw(|f| {
                Visualizer::render(&mut p, f, area, Some(&data), 0.0, &ctx(&th));
            })
            .unwrap();
    }

    #[test]
    fn pulse_rings_silence_renders_empty_buffer() {
        let mut p = PulseRings::new();
        let backend = TestBackend::new(40, 12);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 12);
        let mut d = VisualizerData::empty(64);
        d.loudness = 0.0;
        d.beat = false;
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        term
            .draw(|f| Visualizer::render(&mut p, f, area, Some(&d), 0.0, &ctx(&th)))
            .unwrap();
        let buf = term.backend().buffer();
        for y in 0..12u16 {
            for x in 0..40u16 {
                let sym = buf[(x, y)].symbol();
                assert!(
                    sym == " " || sym.chars().all(|c| c.is_whitespace()),
                    "expected blank viz at ({x},{y}), got {sym:?}"
                );
            }
        }
    }

    #[test]
    fn pulse_rings_beat_spawns_ring() {
        let mut p = PulseRings::new();
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        let area = Rect::new(0, 0, 50, 20);
        let backend = TestBackend::new(50, 20);
        let mut term = Terminal::new(backend).unwrap();
        let mut d = VisualizerData::empty(64);
        d.loudness = 1.0;
        d.beat = true;
        d.beat_intensity = 1.0;
        d.bass_energy = 0.8;
        d.mid_energy = 0.1;
        d.high_energy = 0.1;
        term
            .draw(|f| Visualizer::render(&mut p, f, area, Some(&d), 0.0, &ctx(&th)))
            .unwrap();
        assert_eq!(p.ring_count(), 1);
        let buf = term.backend().buffer();
        let mut any_fg = false;
        for y in 0..20u16 {
            for x in 0..50u16 {
                if buf[(x, y)].fg != ratatui::style::Color::Reset {
                    any_fg = true;
                    break;
                }
            }
        }
        assert!(any_fg, "expected at least one styled cell after beat ring");
        d.beat = false;
        term
            .draw(|f| Visualizer::render(&mut p, f, area, Some(&d), 0.0, &ctx(&th)))
            .unwrap();
        assert_eq!(p.ring_count(), 1);
    }
}
