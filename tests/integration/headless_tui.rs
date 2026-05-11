//! Headless one-frame render smoke test.

use std::sync::{Arc, Mutex};

use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType};
use ratatui::Terminal;

use rtunes::app::state::{AppState, PanelFocus, VisualizerMode};
use rtunes::config::{resolve_active_theme, RtunesConfig};
use rtunes::tui::color::parse_hex;
use rtunes::tui::{build_snapshot, draw_frame, RenderScratch};
use rtunes::visualizer::renderers::{make_renderer, NoopVisualizer};

fn default_cfg() -> RtunesConfig {
    const YAML: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/default_config.yaml"
    ));
    serde_yaml::from_str(YAML).expect("default config")
}

#[test]
fn draw_frame_smoke_two_sizes_no_panic() {
    let cfg = default_cfg();
    let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
    let state = Arc::new(Mutex::new(AppState::new(&cfg, theme.clone())));
    let theme_arc = Arc::new(Mutex::new(theme));

    let mut scratch = RenderScratch::new();
    let mut viz = NoopVisualizer;

    let snap = build_snapshot(&state, &theme_arc, 8, 1.0, 1.0);
    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    term
        .draw(|f| draw_frame(f, &snap, None, 1.0, &mut viz, &mut scratch))
        .unwrap();

    let snap2 = build_snapshot(&state, &theme_arc, 8, 1.0, 1.0);
    let backend2 = TestBackend::new(40, 12);
    let mut term2 = Terminal::new(backend2).unwrap();
    term2
        .draw(|f| draw_frame(f, &snap2, None, 1.0, &mut viz, &mut scratch))
        .unwrap();
}

#[test]
fn redesigned_draw_normal_transport_only_fullscreen_no_panic() {
    let cfg = default_cfg();
    let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
    let state = Arc::new(Mutex::new(AppState::new(&cfg, theme.clone())));
    let theme_arc = Arc::new(Mutex::new(theme.clone()));

    let mut scratch = RenderScratch::new();
    let mut viz = NoopVisualizer;

    {
        let mut g = state.lock().unwrap();
        for i in 0..2 {
            g.library.push(rtunes::app::state::Track {
                id: format!("{i}"),
                filepath: std::path::PathBuf::from(format!("t{i}.mp3")),
                title: format!("Track {i}"),
                artist: None,
                album: None,
                duration_secs: 60,
            });
        }
        g.filtered_indices = vec![0, 1];
        g.selected_track = 0;
    }

    for (panel_focus, is_fullscreen) in [
        (PanelFocus::Normal, false),
        (PanelFocus::TransportOnly, false),
        (PanelFocus::Normal, true),
    ] {
        {
            let mut g = state.lock().unwrap();
            g.panel_focus = panel_focus;
            g.is_fullscreen = is_fullscreen;
        }
        let snap = build_snapshot(&state, &theme_arc, 6, 1.0, 1.0);
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        term
            .draw(|f| draw_frame(f, &snap, None, 1.0, &mut viz, &mut scratch))
            .unwrap();

        if panel_focus == PanelFocus::Normal && !is_fullscreen {
            let buf = term.backend().buffer();
            let full = Rect::new(0, 0, 80, 24);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(5),
                    Constraint::Length(1),
                    Constraint::Length(3),
                ])
                .split(full);
            let body = chunks[1];
            let card_w = ((body.width as u32 * 45 / 100).clamp(40, 80) as u16)
                .min(body.width)
                .min(body.width.saturating_sub(1).max(1));
            let card_h = body.height.saturating_sub(2).max(1);
            let card = Rect::new(body.x + 1, body.y + 1, card_w, card_h);
            let surf = parse_hex(&theme.surface);
            let pri = parse_hex(&theme.primary);
            let inner = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(pri))
                .style(Style::default().bg(surf))
                .inner(card);
            let list_area = Rect::new(
                inner.x.saturating_add(1),
                inner.y,
                inner.width.saturating_sub(1).max(1),
                inner.height,
            );
            let n = snap.library_rows.len().min(list_area.height as usize);
            assert!(list_area.height as usize > n + 1);
            for y in (list_area.y + n as u16)..(list_area.y + list_area.height) {
                for x in list_area.x..(list_area.x + list_area.width) {
                    assert_eq!(
                        buf[(x, y)].bg,
                        surf,
                        "library list dead zone should stay surface at ({x},{y})"
                    );
                }
            }
        }
    }

    for mode in [
        VisualizerMode::PulseRings,
        VisualizerMode::BandMeter,
    ] {
        {
            let mut g = state.lock().unwrap();
            g.panel_focus = PanelFocus::Normal;
            g.is_fullscreen = false;
            g.visualizer_mode = mode;
        }
        let snap = build_snapshot(&state, &theme_arc, 6, 1.0, 1.0);
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let mut r = make_renderer(mode, &rtunes::config::VisualizerSettings::default());
        term
            .draw(|f| draw_frame(f, &snap, None, 1.0, &mut *r, &mut scratch))
            .unwrap();
    }
}
