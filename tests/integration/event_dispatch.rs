//! Synthetic key events through the real TUI dispatch path.

use std::sync::{Arc, Mutex};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use rtunes::app::state::AppState;
use rtunes::config::{resolve_active_theme, RtunesConfig, Theme};
use rtunes::fetcher::{FetcherPool, MockFetcher};
use rtunes::tui::events::{handle_event, TuiDeps};

fn default_cfg() -> RtunesConfig {
    const YAML: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/default_config.yaml"
    ));
    serde_yaml::from_str(YAML).expect("default config")
}

fn deps(cfg: RtunesConfig) -> TuiDeps {
    let (tx, _rx) = crossbeam_channel::unbounded();
    let (picker_tx, _picker_rx) = crossbeam_channel::unbounded();
    let fetcher_settings = Arc::new(Mutex::new(cfg.fetcher.clone()));
    TuiDeps {
        config: Arc::new(Mutex::new(cfg)),
        config_path: std::env::temp_dir().join(format!("rtunes-ev-{}.yaml", std::process::id())),
        fetch_pool: Arc::new(FetcherPool::new(2, Arc::new(MockFetcher))),
        fetch_tx: tx,
        picker_tx,
        fetcher_settings,
    }
}

fn theme_arc(theme: Theme) -> Arc<Mutex<Theme>> {
    Arc::new(Mutex::new(theme))
}

#[test]
fn volume_plus_minus_and_theme_key() {
    let cfg = default_cfg();
    let d = deps(cfg.clone());
    let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
    let app = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
    let theme_arc = theme_arc(th);

    {
        let mut g = app.lock().unwrap();
        g.player.volume = 0.5;
    }

    handle_event(
        &app,
        &theme_arc,
        cfg.theme.custom.as_ref(),
        &d,
        &Event::Key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE)),
    )
    .unwrap();
    assert!(
        app.lock().unwrap().player.volume > 0.5,
        "volume should increase"
    );

    handle_event(
        &app,
        &theme_arc,
        cfg.theme.custom.as_ref(),
        &d,
        &Event::Key(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE)),
    )
    .unwrap();
    assert!(
        app.lock().unwrap().player.volume <= 0.5,
        "volume should decrease"
    );

    let before = theme_arc.lock().unwrap().name.clone();
    handle_event(
        &app,
        &theme_arc,
        cfg.theme.custom.as_ref(),
        &d,
        &Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
    )
    .unwrap();
    let after = theme_arc.lock().unwrap().name.clone();
    assert_ne!(before, after, "theme should change on 't'");
}

#[test]
fn search_slash_type_enter_filters() {
    let cfg = default_cfg();
    let d = deps(cfg.clone());
    let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
    let app = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
    let theme_arc = theme_arc(th);

    {
        let mut g = app.lock().unwrap();
        g.library.push(rtunes::app::state::Track {
            id: "x".into(),
            filepath: std::path::PathBuf::from("a.mp3"),
            title: "UniqueZebraTitle".into(),
            artist: None,
            album: None,
            duration_secs: 1,
        });
    }

    handle_event(
        &app,
        &theme_arc,
        cfg.theme.custom.as_ref(),
        &d,
        &Event::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
    )
    .unwrap();
    assert_eq!(app.lock().unwrap().input_mode, rtunes::app::state::InputMode::Search);

    for ch in "Zebra".chars() {
        handle_event(
            &app,
            &theme_arc,
            cfg.theme.custom.as_ref(),
            &d,
            &Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)),
        )
        .unwrap();
    }
    handle_event(
        &app,
        &theme_arc,
        cfg.theme.custom.as_ref(),
        &d,
        &Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    )
    .unwrap();

    let g = app.lock().unwrap();
    assert_eq!(g.input_mode, rtunes::app::state::InputMode::Normal);
    assert_eq!(g.filtered_indices.len(), 1);
}

#[test]
fn space_toggles_play_flag_when_track_selected() {
    let cfg = default_cfg();
    let d = deps(cfg.clone());
    let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
    let app = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
    let theme_arc = theme_arc(th);

    {
        let mut g = app.lock().unwrap();
        g.library.push(rtunes::app::state::Track {
            id: "y".into(),
            filepath: std::path::PathBuf::from("b.mp3"),
            title: "T".into(),
            artist: None,
            album: None,
            duration_secs: 1,
        });
        g.player.current_index = Some(0);
        g.player.is_playing = false;
    }

    handle_event(
        &app,
        &theme_arc,
        cfg.theme.custom.as_ref(),
        &d,
        &Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
    )
    .unwrap();
    assert!(app.lock().unwrap().player.is_playing);

    handle_event(
        &app,
        &theme_arc,
        cfg.theme.custom.as_ref(),
        &d,
        &Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
    )
    .unwrap();
    assert!(!app.lock().unwrap().player.is_playing);
}

#[test]
fn n_key_advances_selection_with_library() {
    let cfg = default_cfg();
    let d = deps(cfg.clone());
    let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
    let app = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
    let theme_arc = theme_arc(th);

    {
        let mut g = app.lock().unwrap();
        for i in 0..2 {
            g.library.push(rtunes::app::state::Track {
                id: format!("{i}"),
                filepath: std::path::PathBuf::from(format!("{i}.mp3")),
                title: format!("Track{i}"),
                artist: None,
                album: None,
                duration_secs: 1,
            });
        }
        g.player.current_index = Some(0);
        g.selected_track = 0;
    }

    handle_event(
        &app,
        &theme_arc,
        cfg.theme.custom.as_ref(),
        &d,
        &Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
    )
    .unwrap();
    let g = app.lock().unwrap();
    assert_eq!(g.player.current_index, Some(1));
}
