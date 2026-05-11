//! Scrolling spectrogram waterfall.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::state::SpectrogramMode;
use crate::tui::color::{dim_with_intensity, parse_hex};
use crate::visualizer::VisualizerData;

use super::canvas::gradient_color;

#[inline]
fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

fn bin_index(x: u16, w: u16, n: usize, mode: SpectrogramMode) -> usize {
    if w == 0 || n == 0 {
        return 0;
    }
    let n = n.max(1);
    match mode {
        SpectrogramMode::Standard => {
            let b = (x as usize * n / w as usize).min(n - 1);
            b
        }
        SpectrogramMode::Inverted => {
            let b = (((w - 1 - x) as usize) * n / w as usize).min(n - 1);
            b
        }
        SpectrogramMode::Mirrored => {
            let cx = (w - 1) as f32 * 0.5;
            let dist = ((x as f32 - cx).abs() / cx.max(0.5)) * ((n / 2).max(1)) as f32;
            (dist as usize).min(n - 1)
        }
    }
}

fn gaussian_blur_1d_row(inp: &[f32], out: &mut [f32]) {
    let n = inp.len();
    if n == 0 {
        return;
    }
    let k = [0.227027f32, 0.316216, 0.227027];
    for i in 0..n {
        let a = inp[i.saturating_sub(1)];
        let b = inp[i];
        let c = inp[(i + 1).min(n - 1)];
        out[i] = a * k[0] + b * k[1] + c * k[2];
    }
}

pub struct Spectrogram {
    prev_top: Vec<f32>,
    /// Cached per-column bin indices — recomputed only when width, nbin, or mode changes.
    cached_indices: Vec<usize>,
    cached_w: u16,
    cached_nbin: usize,
    cached_mode: SpectrogramMode,
    /// Scratch buffer for interpolated top row (length = nbin).
    scratch: Vec<f32>,
    /// Scratch buffer for Gaussian blur output (length = w).
    blurred: Vec<f32>,
    /// Per-row magnitudes buffer (length = w) — reused across rows.
    mags_buf: Vec<f32>,
}

impl Spectrogram {
    pub fn new() -> Self {
        Self {
            prev_top: Vec::new(),
            cached_indices: Vec::new(),
            cached_w: 0,
            cached_nbin: 0,
            cached_mode: SpectrogramMode::Standard,
            scratch: Vec::new(),
            blurred: Vec::new(),
            mags_buf: Vec::new(),
        }
    }

    /// Rebuild the column→bin index cache when size or mode changes.
    fn rebuild_cache(&mut self, w: usize, nbin: usize, mode: SpectrogramMode) {
        self.cached_indices.resize(w, 0);
        for x in 0..w {
            self.cached_indices[x] = bin_index(x as u16, w as u16, nbin, mode);
        }
        self.cached_w = w as u16;
        self.cached_nbin = nbin;
        self.cached_mode = mode;
    }
}

impl crate::visualizer::Visualizer for Spectrogram {
    fn render(
        &mut self,
        f: &mut Frame<'_>,
        area: Rect,
        data: Option<&VisualizerData>,
        t: f32,
        rctx: &crate::visualizer::RendererCtx<'_>,
    ) {
        let theme = rctx.theme;
        let mode = rctx.spectrogram_mode;
        let w = area.width as usize;
        let h = area.height as usize;
        if w == 0 || h == 0 {
            return;
        }
        let empty = VisualizerData::empty(64);
        let d = data.unwrap_or(&empty);
        let nbin = d.bins_smoothed.len().max(1);
        self.prev_top.resize(nbin, 0.0);

        // Rebuild column→bin index cache only on size/mode change (hot path: just a table lookup).
        if w != self.cached_w as usize || nbin != self.cached_nbin || mode != self.cached_mode {
            self.rebuild_cache(w, nbin, mode);
        }

        // Resize scratch buffers on demand (no alloc if size unchanged).
        if self.scratch.len() != nbin {
            self.scratch.resize(nbin, 0.0);
        }
        if self.blurred.len() != w {
            self.blurred.resize(w, 0.0);
        }
        if self.mags_buf.len() != w {
            self.mags_buf.resize(w, 0.0);
        }

        let rows = &d.spectrogram_rows;
        let bg = parse_hex(&theme.background);
        let threshold = 0.02f32;

        let mut lines: Vec<Line> = Vec::with_capacity(h);
        for row_ix in 0..h {
            let src_row = rows.get(row_ix).map(|v| v.as_slice()).unwrap_or(&[]);

            // Fill mags_buf using the pre-built index cache.
            for x in 0..w {
                let bi = self.cached_indices[x];
                self.mags_buf[x] = src_row.get(bi).copied().unwrap_or(0.0);
            }

            if row_ix == 0 && !src_row.is_empty() {
                // Interpolate top row for sub-frame smoothness.
                for i in 0..nbin.min(src_row.len()) {
                    self.scratch[i] = lerp_f32(self.prev_top[i], src_row[i], t);
                }
                for x in 0..w {
                    let bi = self.cached_indices[x];
                    self.mags_buf[x] = self.scratch.get(bi).copied().unwrap_or(0.0);
                }
            }

            if rctx.glow {
                gaussian_blur_1d_row(&self.mags_buf, &mut self.blurred);
                self.mags_buf.copy_from_slice(&self.blurred[..w]);
            }

            let mut spans = Vec::with_capacity(w);
            for x in 0..w {
                let mag = self.mags_buf[x];
                let fg = if mag > threshold {
                    let c = gradient_color(&theme.viz.gradient, mag.clamp(0.0, 1.0));
                    dim_with_intensity(c, bg, rctx.viz_intensity)
                } else {
                    bg
                };
                spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
            }
            lines.push(Line::from(spans));
        }

        if let Some(top) = rows.front() {
            let len = top.len().min(self.prev_top.len());
            self.prev_top[..len].copy_from_slice(&top[..len]);
        }

        let p = Paragraph::new(lines).style(Style::default().bg(bg));
        f.render_widget(p, area);
        crate::visualizer::maybe_draw_viz_baseline(f, area, rctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrogram_bin_mapping_modes() {
        assert_eq!(bin_index(0, 10, 64, SpectrogramMode::Standard), 0);
        assert_eq!(bin_index(9, 10, 64, SpectrogramMode::Inverted), 0);
        let m = bin_index(5, 10, 64, SpectrogramMode::Mirrored);
        assert!(m < 64);
    }

    #[test]
    fn spectrogram_wide_terminal_glow_no_panic() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        use crate::config::theme;
        use crate::visualizer::RendererCtx;

        let mut s = Spectrogram::new();
        let backend = TestBackend::new(120, 20);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 120, 20);
        let data = VisualizerData::empty(64);
        let th = theme::builtin_themes().get("synthwave").cloned().unwrap();
        let ctx = RendererCtx {
            theme: &th,
            fullscreen: false,
            glow: true,
            spectrogram_mode: SpectrogramMode::Standard,
            viz_intensity: 1.0,
            baseline: false,
        };
        term
            .draw(|f| {
                crate::visualizer::Visualizer::render(&mut s, f, area, Some(&data), 0.5, &ctx);
            })
            .unwrap();
    }

    #[test]
    fn spectrogram_no_panic_empty_rows() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        use crate::config::theme;
        use crate::visualizer::RendererCtx;

        let mut s = Spectrogram::new();
        let backend = TestBackend::new(40, 12);
        let mut term = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 12);
        let data = VisualizerData::empty(64);
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
                crate::visualizer::Visualizer::render(&mut s, f, area, Some(&data), 0.5, &ctx);
            })
            .unwrap();
    }

    #[test]
    fn spectrogram_top_row_lerp() {
        use std::sync::Arc;
        let mut s = Spectrogram::new();
        s.prev_top = vec![0.0f32; 64];
        let mut d = VisualizerData::empty(64);
        d.bins_smoothed[0] = 1.0;
        let rows = Arc::make_mut(&mut d.spectrogram_rows);
        rows.push_front(d.bins_smoothed.clone());
        let v = lerp_f32(0.0, 1.0, 0.5);
        assert!((v - 0.5).abs() < 1e-5);
    }
}
