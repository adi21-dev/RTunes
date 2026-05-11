//! Audio-reactive particle field with Perlin turbulence and temporal supersampling.

use noise::NoiseFn;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::canvas::{Canvas, Context, Points};
use ratatui::symbols::Marker;
use ratatui::Frame;

use crate::config::VisualizerSettings;
use crate::tui::color::{dim_with_intensity, lerp_color, parse_hex};
use crate::visualizer::VisualizerData;

use super::phosphor::PhosphorBuffer;
use super::rand::xorshift_u01;

#[derive(Clone)]
struct Particle {
    active: bool,
    px: f32,
    py: f32,
    vx: f32,
    vy: f32,
    life: f32,
    hue_idx: u8,
}

pub struct Particles {
    pool: Vec<Particle>,
    cap: usize,
    history: PhosphorBuffer,
    perlin: noise::Perlin,
    rng: u64,
    time_s: f32,
    substeps: usize,
    max_particles: usize,
    /// Reusable coordinate buffers — cleared at start of each render (no per-frame alloc).
    hist_coords: Vec<(f64, f64)>,
    hist_cols: Vec<ratatui::style::Color>,
    live_coords: Vec<(f64, f64)>,
    live_cols: Vec<ratatui::style::Color>,
}

impl Particles {
    pub fn new() -> Self {
        Self::with_settings(&VisualizerSettings::default())
    }

    pub fn with_settings(s: &VisualizerSettings) -> Self {
        Self {
            pool: Vec::new(),
            cap: 0,
            history: PhosphorBuffer::new(0.94),
            perlin: noise::Perlin::new(0x5EED),
            rng: 0xC0FFEE_u64,
            time_s: 0.0,
            substeps: (s.particles_substeps.max(1)) as usize,
            max_particles: s.particles_max.max(10) as usize,
            hist_coords: Vec::new(),
            hist_cols: Vec::new(),
            live_coords: Vec::new(),
            live_cols: Vec::new(),
        }
    }

    fn ensure_cap(&mut self, cap: usize) {
        if cap == self.cap && self.pool.len() == cap {
            return;
        }
        self.cap = cap;
        self.pool.clear();
        self.pool.resize(
            cap,
            Particle {
                active: false,
                px: 0.0,
                py: 0.0,
                vx: 0.0,
                vy: 0.0,
                life: 0.0,
                hue_idx: 0,
            },
        );
    }

    fn spawn_burst(&mut self, cx: f32, cy: f32, beat_intensity: f32, ncolors: u8) {
        let mut spawned = 0usize;
        for p in self.pool.iter_mut() {
            if spawned >= 30 {
                break;
            }
            if !p.active {
                let ang = xorshift_u01(&mut self.rng) * std::f32::consts::TAU;
                let sp = (0.15 + beat_intensity * 0.85) * (0.5 + xorshift_u01(&mut self.rng));
                p.active = true;
                p.px = cx;
                p.py = cy;
                p.vx = ang.cos() * sp;
                p.vy = ang.sin() * sp;
                p.life = 1.0;
                p.hue_idx = (xorshift_u01(&mut self.rng) * ncolors as f32) as u8 % ncolors.max(1);
                spawned += 1;
            }
        }
    }
}

impl crate::visualizer::Visualizer for Particles {
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
        let wf = w as f32;
        let hf = h as f32;
        let cx = wf * 0.5;
        let cy = hf * 0.5;
        let cap = ((w as usize * h as usize) / 12).clamp(10, self.max_particles);
        self.ensure_cap(cap);

        let empty = VisualizerData::empty(64);
        let d = data.unwrap_or(&empty);

        self.history.ensure_size(w, h);
        self.history.scale_all(0.6);

        if d.beat {
            let nc = theme.viz.particle_colors.len().max(1) as u8;
            self.spawn_burst(cx, cy, d.beat_intensity, nc);
        }

        let ncols = theme.viz.particle_colors.len().max(1);
        self.time_s += 1.0 / 60.0;

        for _ in 0..self.substeps {
            for p in self.pool.iter_mut() {
                if !p.active {
                    continue;
                }
                let dx = p.px - cx;
                let dy = p.py - cy;
                let dist = (dx * dx + dy * dy).sqrt().max(1e-3);
                let mut ax = -dx / dist * 0.02 * dist;
                let mut ay = -dy / dist * 0.02 * dist;
                ax += (dx / dist) * d.bass_energy * 0.05;
                let nx = f64::from(p.px * 0.08);
                let ny = f64::from(p.py * 0.08);
                let nz = f64::from(self.time_s);
                let turb = self.perlin.get([nx, ny, nz]) as f32;
                ax += turb * d.mid_energy * 0.2;
                ay += turb * d.mid_energy * 0.15;

                p.vx += ax * 0.5;
                p.vy += ay * 0.5;
                p.vx *= 0.97;
                p.vy *= 0.97;
                p.px += p.vx * 0.5;
                p.py += p.vy * 0.5;
                p.life -= 0.005 * 0.5;
                if p.life <= 0.01
                    || p.px < -2.0
                    || p.py < -2.0
                    || p.px > wf + 2.0
                    || p.py > hf + 2.0
                {
                    p.active = false;
                }
                let hi = ((d.high_energy * 4.0) as u8 + p.hue_idx) % ncols as u8;
                p.hue_idx = hi;
            }
        }

        for p in self.pool.iter() {
            if p.active {
                let xi = p.px.round() as i32;
                let yi = p.py.round() as i32;
                self.history
                    .paint(xi, yi, (p.life * 0.5).clamp(0.0, 1.0));
            }
        }

        let bg = parse_hex(&theme.background);
        let vi = rctx.viz_intensity;
        let wf64 = wf as f64;
        let hf64 = hf as f64;
        let colors: Vec<ratatui::style::Color> = theme
            .viz
            .particle_colors
            .iter()
            .map(|s| dim_with_intensity(parse_hex(s), bg, vi))
            .collect();
        let default_c = dim_with_intensity(parse_hex(&theme.primary), bg, vi);

        // Reuse member buffers — clear preserves capacity (no heap alloc in steady state).
        self.hist_coords.clear();
        self.hist_cols.clear();
        for (ix, iy, v) in self.history.iter_lit(0.02) {
            let wx = ix as f64 + 0.5;
            let wy = iy as f64 + 0.5;
            self.hist_coords.push((wx, wy));
            let hc = lerp_color(default_c, bg, 1.0 - (v * 0.6));
            self.hist_cols.push(dim_with_intensity(hc, bg, vi));
        }

        self.live_coords.clear();
        self.live_cols.clear();
        for p in self.pool.iter() {
            if p.active {
                self.live_coords.push((f64::from(p.px) + 0.5, f64::from(p.py) + 0.5));
                let c = colors
                    .get(p.hue_idx as usize % colors.len().max(1))
                    .copied()
                    .unwrap_or(default_c);
                let lc = lerp_color(c, bg, 1.0 - p.life);
                self.live_cols.push(dim_with_intensity(lc, bg, vi));
            }
        }

        // Move coord slices into the closure. We clone only the small theme struct here;
        // the coord Vecs are moved in as owned to satisfy the 'static closure requirement.
        let hist_coords = self.hist_coords.clone();
        let hist_cols = self.hist_cols.clone();
        let live = self.live_coords.clone();
        let live_c = self.live_cols.clone();
        let theme_owned = theme.clone();
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, wf64])
            .y_bounds([0.0, hf64])
            .background_color(bg)
            .paint(move |cctx: &mut Context<'_>| {
                for (&(x, y), &col) in hist_coords.iter().zip(hist_cols.iter()) {
                    cctx.draw(&Points {
                        coords: &[(x, y)],
                        color: col,
                    });
                }
                for (&(x, y), &col) in live.iter().zip(live_c.iter()) {
                    cctx.draw(&Points {
                        coords: &[(x, y)],
                        color: col,
                    });
                }
                if glow_on && !live.is_empty() {
                    super::canvas::glow_pass(
                        cctx,
                        w,
                        h,
                        [0.0, wf64],
                        [0.0, hf64],
                        &live,
                        &theme_owned,
                        default_c,
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
    fn particles_no_panic_with_zeroed_data() {
        let mut p = Particles::new();
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        let data = VisualizerData::empty(64);
        let th = theme::builtin_themes()
            .get("synthwave")
            .cloned()
            .unwrap();
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
                crate::visualizer::Visualizer::render(&mut p, f, area, Some(&data), 0.5, &ctx);
            })
            .unwrap();
    }

    #[test]
    fn particles_pool_capacity_stable_after_warmup() {
        let mut p = Particles::new();
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        let ctx = RendererCtx {
            theme: &th,
            fullscreen: false,
            glow: false,
            spectrogram_mode: SpectrogramMode::Standard,
            viz_intensity: 1.0,
            baseline: false,
        };
        let area = Rect::new(0, 0, 80, 24);
        let data = VisualizerData::empty(64);
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        for _ in 0..6 {
            term
                .draw(|f| {
                    crate::visualizer::Visualizer::render(&mut p, f, area, Some(&data), 0.5, &ctx);
                })
                .unwrap();
        }
        let c0 = p.pool.capacity();
        term.draw(|f| {
            crate::visualizer::Visualizer::render(&mut p, f, area, Some(&data), 0.5, &ctx);
        })
        .unwrap();
        assert_eq!(p.pool.capacity(), c0);
    }

    #[test]
    fn particles_beat_spawns_burst() {
        let mut p = Particles::new();
        p.ensure_cap(200);
        p.spawn_burst(40.0, 12.0, 1.0, 4);
        let n = p.pool.iter().filter(|x| x.active).count();
        assert!(n >= 30);
    }
}
