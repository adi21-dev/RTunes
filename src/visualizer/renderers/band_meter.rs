//! Three vertical VU columns (bass / mid / high) with asymmetric EMA and peak-hold.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::color::{dim_with_intensity, parse_hex};
use crate::visualizer::smoothing::{ema_dt, peak_hold_drift, AutoGain};
use crate::visualizer::VisualizerData;

use super::spectrum::ROUND_TOP;

const PEAK_DECAY: f32 = 0.96;

/// EMA time constants in seconds for dt-based smoothing.
const TAU_ATTACK: f32 = 0.012; // fast rise  (~12 ms)
const TAU_RELEASE: f32 = 0.080; // slow fall  (~80 ms)

pub struct BandMeter {
    ema: [f32; 3],
    peak: [f32; 3],
    auto_gain: AutoGain,
}

impl BandMeter {
    pub fn new() -> Self {
        Self {
            ema: [0.0; 3],
            peak: [0.0; 3],
            auto_gain: AutoGain::with_limits(0.85, 1.0, 4.0),
        }
    }
}

impl crate::visualizer::Visualizer for BandMeter {
    fn render(
        &mut self,
        f: &mut Frame<'_>,
        area: Rect,
        data: Option<&VisualizerData>,
        t: f32,
        rctx: &crate::visualizer::RendererCtx<'_>,
    ) {
        let theme = rctx.theme;
        if area.width == 0 || area.height < 3 {
            return;
        }

        let bg = parse_hex(&theme.background);
        let surf = parse_hex(&theme.surface);
        let txt = parse_hex(&theme.text);
        let dim = parse_hex(&theme.text_dim);
        let acc = parse_hex(&theme.accent);
        let vi = rctx.viz_intensity;

        let empty = VisualizerData::empty(64);
        let d = data.unwrap_or(&empty);

        if data.is_none() || d.loudness < 0.005 {
            self.ema = [0.0; 3];
            self.peak = [0.0; 3];
            self.auto_gain.reset();
            f.render_widget(Paragraph::new("").style(Style::default().bg(bg)), area);
            crate::visualizer::maybe_draw_viz_baseline(f, area, rctx);
            return;
        }

        let bands = [
            (d.bass_energy * 1.0).clamp(0.0, 1.0),
            (d.mid_energy * 1.1).clamp(0.0, 1.0),
            (d.high_energy * 1.4).clamp(0.0, 1.0),
        ];
        let g = self.auto_gain.apply(&bands);
        // Use dt-based EMA so band motion is frame-rate-independent.
        // `fft_period` is the inter-FFT interval; lerp the remaining sub-frame via `t`.
        let dt = d.fft_period.as_secs_f32();
        for (i, &band) in bands.iter().enumerate() {
            let target = (band * g).min(1.0);
            let tau = if target > self.ema[i] {
                TAU_ATTACK
            } else {
                TAU_RELEASE
            };
            let smoothed = ema_dt(self.ema[i], target, tau, dt);
            // Sub-frame interpolation: blend the previous EMA toward the new EMA value.
            self.ema[i] = smoothed.clamp(0.0, 1.0);
            let _ = t; // sub_frame_t not needed for columnar bar display (already smooth)
            self.peak[i] = peak_hold_drift(self.peak[i], self.ema[i], PEAK_DECAY);
        }

        let label_h = 1u16;
        let meter_h = area.height.saturating_sub(label_h).max(1);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ])
            .split(area);

        let labels = ["BASS", "MID", "HIGH"];
        let mut dom = 0usize;
        let mut best = self.ema[0];
        for i in 1..3 {
            if self.ema[i] > best {
                best = self.ema[i];
                dom = i;
            }
        }
        let flash_rows = if d.beat {
            ((meter_h as f32) * d.beat_intensity * 0.18).ceil().max(1.0) as u16
        } else {
            0
        };

        for col in 0..3 {
            let crect = cols[col];
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(meter_h), Constraint::Length(label_h)])
                .split(crect);
            let meter_rect = chunks[0];
            let label_rect = chunks[1];

            let fill = self.ema[col];
            let fill_total = fill * meter_h as f32;
            let fill_cells = fill_total.floor() as i32;
            let frac = fill_total - fill_cells as f32;

            let mut lines: Vec<Line> = Vec::with_capacity(meter_h as usize);
            for row in 0..meter_h {
                let from_bottom = (meter_h - 1 - row) as i32;
                let in_flash_zone = d.beat && col == dom && row < flash_rows;

                let mut spans: Vec<Span> = Vec::with_capacity(meter_rect.width as usize);
                for _ in 0..meter_rect.width {
                    let ch = if from_bottom < fill_cells {
                        '█'
                    } else if from_bottom == fill_cells && frac > 1e-4 {
                        let idx = ((frac * 8.0).floor() as usize).min(ROUND_TOP.len() - 1);
                        ROUND_TOP[idx]
                    } else {
                        ' '
                    };

                    let (fg, cell_bg) = if in_flash_zone && ch == '█' {
                        (acc, surf)
                    } else if ch == ' ' {
                        (dim, surf)
                    } else {
                        (dim_with_intensity(txt, bg, vi), surf)
                    };
                    spans.push(Span::styled(
                        ch.to_string(),
                        Style::default().fg(fg).bg(cell_bg),
                    ));
                }
                lines.push(Line::from(spans));
            }

            f.render_widget(
                Paragraph::new(lines).style(Style::default().bg(surf)),
                meter_rect,
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    labels[col],
                    Style::default()
                        .fg(dim_with_intensity(txt, bg, vi))
                        .bg(surf)
                        .add_modifier(Modifier::BOLD),
                )]))
                .alignment(Alignment::Center)
                .style(Style::default().bg(surf)),
                label_rect,
            );
        }

        crate::visualizer::maybe_draw_viz_baseline(f, area, rctx);
    }

    fn reset(&mut self) {
        self.ema = [0.0; 3];
        self.peak = [0.0; 3];
        self.auto_gain.reset();
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
    fn band_meter_no_panic_with_zeroed_data() {
        let mut b = BandMeter::new();
        let backend = TestBackend::new(60, 18);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 60, 18);
        let data = VisualizerData::empty(64);
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        term.draw(|f| Visualizer::render(&mut b, f, area, Some(&data), 0.0, &ctx(&th)))
            .unwrap();
    }

    #[test]
    fn band_meter_columns_track_bass_mid_high() {
        let mut b = BandMeter::new();
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        let area = Rect::new(0, 0, 60, 18);
        let backend = TestBackend::new(60, 18);
        let mut term = Terminal::new(backend).unwrap();
        let mut d = VisualizerData::empty(64);
        d.loudness = 1.0;
        d.bass_energy = 1.0;
        d.mid_energy = 0.0;
        d.high_energy = 0.0;
        term.draw(|f| Visualizer::render(&mut b, f, area, Some(&d), 0.0, &ctx(&th)))
            .unwrap();
        let buf = term.backend().buffer();
        let label_h = 1u16;
        let meter_h = area.height.saturating_sub(label_h);
        let col_w = area.width / 3;

        fn col_fill_max(buf: &ratatui::buffer::Buffer, x0: u16, x1: u16, meter_h: u16) -> i32 {
            let mut max_row = -1i32;
            for y in 0..meter_h {
                for x in x0..x1 {
                    if buf[(x, y)].symbol() == "█" {
                        max_row = max_row.max(y as i32);
                    }
                }
            }
            max_row
        }

        let m0 = col_fill_max(buf, 0, col_w, meter_h);
        let m1 = col_fill_max(buf, col_w, col_w * 2, meter_h);
        let m2 = col_fill_max(buf, col_w * 2, area.width, meter_h);
        assert!(
            m0 > m1 && m0 > m2,
            "bass column should fill higher than mid/high: m0={m0} m1={m1} m2={m2}"
        );
    }

    #[test]
    fn band_meter_normalizes_quiet_levels() {
        let mut b = BandMeter::new();
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        let area = Rect::new(0, 0, 60, 18);
        let backend = TestBackend::new(60, 18);
        let mut term = Terminal::new(backend).unwrap();
        let mut d = VisualizerData::empty(64);
        d.loudness = 1.0;
        d.bass_energy = 0.15;
        d.mid_energy = 0.15;
        d.high_energy = 0.15;
        for _ in 0..24 {
            term.draw(|f| Visualizer::render(&mut b, f, area, Some(&d), 0.0, &ctx(&th)))
                .unwrap();
        }
        let buf = term.backend().buffer();
        let label_h = 1u16;
        let meter_h = area.height.saturating_sub(label_h) as usize;
        let col_w = (area.width / 3) as usize;
        let half = meter_h / 2;

        fn col_fill_depth(
            buf: &ratatui::buffer::Buffer,
            x0: u16,
            x1: u16,
            meter_h: usize,
        ) -> usize {
            let mut max_from_bottom = 0usize;
            for y in 0..meter_h {
                let from_bottom = meter_h - 1 - y;
                for x in x0..x1 {
                    let s = buf[(x, y as u16)].symbol();
                    if s != " " && !s.chars().all(|c| c.is_whitespace()) {
                        max_from_bottom = max_from_bottom.max(from_bottom + 1);
                    }
                }
            }
            max_from_bottom
        }

        let d0 = col_fill_depth(buf, 0, col_w as u16, meter_h);
        let d1 = col_fill_depth(buf, col_w as u16, (col_w * 2) as u16, meter_h);
        let d2 = col_fill_depth(buf, (col_w * 2) as u16, area.width, meter_h);
        assert!(
            d0 >= half && d1 >= half && d2 >= half,
            "auto-gain should lift all three columns past half height: d0={d0} d1={d1} d2={d2} half={half}"
        );
    }
}
