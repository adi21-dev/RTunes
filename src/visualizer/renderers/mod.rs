//! Concrete visualizer renderers (Phase 7+).

mod band_meter;
mod canvas;
pub mod oscilloscope;
mod particles;
mod phosphor;
mod pulse_rings;
pub mod rand;
pub mod spectrogram;
pub mod spectrum;
pub mod supernova;
pub mod vectorscope;

#[allow(unused_imports)] // Public re-exports for downstream / future phases
pub use canvas::{catmull_rom, glow_pass, gradient_color};
pub use oscilloscope::Oscilloscope;
#[allow(unused_imports)]
pub use phosphor::PhosphorBuffer;
#[allow(unused_imports)]
pub use spectrum::{bars_for_width, Spectrum};
pub use supernova::Supernova;

use crate::app::state::VisualizerMode;
use crate::config::VisualizerSettings;

use super::Visualizer;

use band_meter::BandMeter;
use particles::Particles;
use pulse_rings::PulseRings;
use spectrogram::Spectrogram;
use vectorscope::Vectorscope;

/// Stub renderer for modes not implemented yet.
pub struct NoopVisualizer;

impl Visualizer for NoopVisualizer {
    fn render(
        &mut self,
        _f: &mut ratatui::Frame<'_>,
        _area: ratatui::layout::Rect,
        _data: Option<&super::VisualizerData>,
        _t: f32,
        _ctx: &super::RendererCtx<'_>,
    ) {
    }
}

pub fn make_renderer(mode: VisualizerMode, viz: &VisualizerSettings) -> Box<dyn Visualizer> {
    match mode {
        VisualizerMode::Spectrum => Box::new(Spectrum::new()),
        VisualizerMode::Spectrogram => Box::new(Spectrogram::new()),
        VisualizerMode::Oscilloscope => Box::new(Oscilloscope::new()),
        VisualizerMode::Vectorscope => Box::new(Vectorscope::new()),
        VisualizerMode::Supernova => Box::new(Supernova::new()),
        VisualizerMode::PulseRings => Box::new(PulseRings::new()),
        VisualizerMode::BandMeter => Box::new(BandMeter::new()),
        VisualizerMode::Particles => Box::new(Particles::with_settings(viz)),
    }
}
