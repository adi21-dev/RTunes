//! FFT pipeline and visualizer data (Phase 6+).

#![allow(dead_code)] // Public API for Phase 7+

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::state::SpectrogramMode;
use crate::config::Theme;
use crate::tui::color::parse_hex;

pub mod data;
pub mod fft;
pub mod renderers;
pub mod smoothing;
pub mod thread;

pub use data::VisualizerData;
#[allow(unused_imports)]
pub use thread::{spawn_fft_thread, FftHandle};

/// Per-frame inputs shared by all visualizers (Phase 8).
pub struct RendererCtx<'a> {
    pub theme: &'a Theme,
    pub fullscreen: bool,
    /// Glow / halo enabled (neon toggle overrides theme default).
    pub glow: bool,
    pub spectrogram_mode: SpectrogramMode,
    /// `1.0` = full saturation; lower values blend toward background (chrome over viz).
    pub viz_intensity: f32,
    /// When true, draw a 1-row floor line at the bottom of the viz area.
    pub baseline: bool,
}

/// Dim 1-row floor at the bottom of the viz area when chrome sits above the transport.
pub fn maybe_draw_viz_baseline(f: &mut Frame<'_>, area: Rect, ctx: &RendererCtx<'_>) {
    if !ctx.baseline || area.height < 2 {
        return;
    }
    let bg = parse_hex(&ctx.theme.background);
    let dim = parse_hex(&ctx.theme.text_dim);
    let row = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    let line = "─".repeat(area.width as usize);
    f.render_widget(
        Paragraph::new(line).style(Style::default().fg(dim).bg(bg)),
        row,
    );
}

/// Pluggable full-screen / panel visualizer (Phase 7+).
pub trait Visualizer: Send {
    /// Render into `area`. `t` is sub-frame interpolation in `[0, 1]`. `data` may be `None`
    /// before the first FFT frame — visualizers must render an idle/blank state without panic.
    fn render(
        &mut self,
        f: &mut Frame<'_>,
        area: Rect,
        data: Option<&VisualizerData>,
        t: f32,
        ctx: &RendererCtx<'_>,
    );

    /// Reset internal buffers (resize / theme switch / viz switch in later phases).
    fn reset(&mut self) {}
}
