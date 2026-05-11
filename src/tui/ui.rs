//! Ratatui layout, render snapshot, and main loop.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, TryRecvError};
use crossterm::event;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::lock_shared;
use crate::app::state::{
    AppState, InputMode, LibraryFolder, PanelFocus, RepeatMode, SettingsRow, SpectrogramMode,
    VisualizerMode,
};
use crate::config::theme::{effective_glow, normalize_theme_key};
use crate::config::{RtunesConfig, Theme};
use crate::fetcher::FetchEvent;
use crate::tui::color::{gradient_at, lerp_color, parse_hex};
use crate::tui::events::{handle_event, trigger_rescan, TuiDeps};
use crate::tui::terminal::TerminalGuard;
use crate::utils::expand_path;
use crate::visualizer::renderers::make_renderer;
use crate::visualizer::smoothing;
use crate::visualizer::RendererCtx;
use crate::visualizer::{Visualizer, VisualizerData};

/// Reused `Vec` capacity for progress rows and library list (hot render path).
pub struct RenderScratch {
    progress_spans: Vec<Span<'static>>,
    controls_spans: Vec<Span<'static>>,
    library_items: Vec<ListItem<'static>>,
}

impl Default for RenderScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderScratch {
    pub fn new() -> Self {
        Self {
            progress_spans: Vec::with_capacity(512),
            controls_spans: Vec::with_capacity(512),
            library_items: Vec::with_capacity(32),
        }
    }
}

/// One row in the floating library card (index / title / duration + selection + playing).
#[derive(Debug, Clone)]
pub struct LibraryRowSnap {
    pub idx_str: String,
    pub title_str: String,
    pub duration_str: String,
    pub is_selected: bool,
    pub is_playing: bool,
}

/// One frame of UI data (cloned under lock).
pub struct RenderSnapshot {
    pub theme: Theme,
    pub is_fullscreen: bool,
    /// Library card visibility when not fullscreen. Full layout is `is_fullscreen`.
    pub panel_focus: PanelFocus,
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub show_help: bool,
    pub show_library_manager: bool,
    pub selected_folder: usize,
    pub library_folders: Vec<LibraryFolder>,
    /// `♪ Title — Artist` for status strip tail + fullscreen overlay.
    pub now_playing_left: String,
    /// Album (or placeholder) for transport row A right side.
    pub transport_album_line: String,
    pub visualizer_mode: VisualizerMode,
    pub neon_enabled: bool,
    pub spectrogram_mode: SpectrogramMode,
    /// Fullscreen track title visibility in `[0, 1]` (hold + fade after track change).
    pub fullscreen_track_alpha: f32,
    /// After toggling fullscreen: lerps library/controls from background → normal over 4 frames.
    pub panel_content_blend: f32,
    pub library_rows: Vec<LibraryRowSnap>,
    pub player_pos: f64,
    pub player_dur: f64,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub is_playing: bool,
    pub toast: Option<String>,
    pub library_track_count: usize,
    pub download_progress: Option<f32>,
    pub download_stage: Option<String>,
    /// Filtered library row count (same list as library card indices).
    pub filtered_library_len: usize,
    /// Selected row index in the filtered list (`0..filtered_library_len`).
    pub filtered_library_selected: usize,
    /// Whether the Settings overlay is visible.
    pub show_settings: bool,
    /// Which row is focused in the Settings overlay.
    pub settings_row: SettingsRow,
    /// Current config value of `fetcher.ytdlp_path`.
    pub settings_ytdlp_value: String,
    /// Resolved binary path for yt-dlp (`None` = not found on disk).
    pub settings_ytdlp_resolved: Option<std::path::PathBuf>,
    /// Current config value of `fetcher.ffmpeg_path`.
    pub settings_ffmpeg_value: String,
    /// Resolved binary path for ffmpeg (`None` = not found on disk).
    pub settings_ffmpeg_resolved: Option<std::path::PathBuf>,
    /// Current config value of `app.download_dir`.
    pub settings_download_dir: String,
    /// Whether the download dir exists as a directory on disk.
    pub settings_download_dir_exists: bool,
}

/// Alpha for fullscreen track overlay from elapsed time since last track change.
pub fn fullscreen_overlay_elapsed(elapsed: Duration) -> f32 {
    let hold = Duration::from_secs(3);
    let fade = Duration::from_secs(1);
    if elapsed < hold {
        1.0
    } else if elapsed < hold + fade {
        let t = (elapsed - hold).as_secs_f32() / fade.as_secs_f32();
        (1.0 - t).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Track-name overlay alpha: 1 for 3s after change, then linear fade to 0 over 1s.
pub fn fullscreen_overlay_alpha(now: Instant, track_change: Instant) -> f32 {
    let elapsed = now.saturating_duration_since(track_change);
    fullscreen_overlay_elapsed(elapsed)
}

fn fmt_duration(secs: f64) -> String {
    if secs < 0.0 || secs.is_nan() {
        return "0:00".into();
    }
    let s = secs.floor() as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// Build a [`RenderSnapshot`] for one frame (used by the TUI loop and integration tests).
pub fn build_snapshot(
    state: &Arc<Mutex<AppState>>,
    theme: &Arc<Mutex<Theme>>,
    library_view_height: usize,
    fullscreen_track_alpha: f32,
    panel_content_blend: f32,
) -> RenderSnapshot {
    // ── Acquire AppState lock; copy/clone only the fields we actually need. ──
    let g = lock_shared(state);

    // Scalar (Copy) fields first — no heap allocation.
    let snap_is_fullscreen = g.is_fullscreen;
    let snap_panel_focus = g.panel_focus;
    let snap_input_mode = g.input_mode;
    let snap_show_help = g.show_help;
    let snap_show_library_manager = g.show_library_manager;
    let snap_selected_folder = g.selected_folder;
    let snap_visualizer_mode = g.visualizer_mode;
    let snap_neon_enabled = g.neon_enabled;
    let snap_spectrogram_mode = g.spectrogram_mode;
    let snap_player_pos = g.player.position_secs;
    let snap_player_dur = g.player.duration_secs;
    let snap_volume = g.player.volume;
    let snap_muted = g.player.muted;
    let snap_shuffle = g.player.shuffle;
    let snap_repeat = g.player.repeat;
    let snap_is_playing = g.player.is_playing;
    let snap_download_progress = g.download_progress;
    let show_settings = g.show_settings;
    let settings_row = g.settings_row;
    let library_track_count = g.library.len();
    let playing_lib_idx = g.player.current_index;

    // Heap-allocated fields (unavoidable clones).
    let snap_input_buffer = g.input_buffer.clone();
    let snap_download_stage = g.download_stage.clone();
    let toast = g.message.as_ref().map(|(s, _)| s.clone());
    // Settings strings — only clone when the overlay is actually visible.
    let settings_ytdlp_value = if show_settings {
        g.settings_ytdlp_value.clone()
    } else {
        String::new()
    };
    let settings_ffmpeg_value = if show_settings {
        g.settings_ffmpeg_value.clone()
    } else {
        String::new()
    };
    let settings_download_dir = if show_settings {
        g.settings_download_dir.clone()
    } else {
        String::new()
    };

    // Library folders — only clone when the manager is open (rarely visible).
    let snap_library_folders = if snap_show_library_manager || snap_show_help {
        g.library_folders.clone()
    } else {
        Vec::new()
    };

    // Library rows: build while holding the lock but avoid cloning filtered_indices.
    let total = if g.filtered_indices.is_empty() {
        g.library.len()
    } else {
        g.filtered_indices.len()
    };
    let sel = g.selected_track.min(total.saturating_sub(1));
    let view_h = library_view_height.max(1);
    let start = sel.saturating_sub(view_h.saturating_sub(1));
    let end = (start + view_h).min(total);

    let mut library_rows = Vec::with_capacity(end.saturating_sub(start));
    for row in start..end {
        let lib_idx = if g.filtered_indices.is_empty() {
            row
        } else {
            g.filtered_indices[row]
        };
        let is_sel = row == sel;
        if let Some(t) = g.library.get(lib_idx) {
            let is_playing = Some(lib_idx) == playing_lib_idx;
            library_rows.push(LibraryRowSnap {
                idx_str: format!("{:03}", row + 1),
                title_str: t.title.clone(),
                duration_str: fmt_duration(t.duration_secs as f64),
                is_selected: is_sel,
                is_playing,
            });
        }
    }

    // Now-playing strings: built from library entry directly under the lock.
    let (np_left, transport_album_line) = match playing_lib_idx.and_then(|i| g.library.get(i)) {
        Some(t) => {
            let artist = t.artist.as_deref().unwrap_or("Unknown");
            let album = t.album.as_deref().filter(|s| !s.is_empty()).unwrap_or("—");
            (format!("♪ {} — {}", t.title, artist), album.to_string())
        }
        None => ("— nothing playing —".into(), "—".into()),
    };

    // Release the AppState lock before any filesystem or secondary lock operations.
    drop(g);

    // Theme: separate lock, cloned once per frame (cheap — mostly u8 colors + Vecs of strings).
    let th = lock_shared(theme).clone();

    // Path resolution only when the Settings overlay is open.
    let (settings_ytdlp_resolved, settings_ffmpeg_resolved, settings_download_dir_exists) =
        if show_settings {
            (
                crate::fetcher::resolve_fetcher_tool(&settings_ytdlp_value, "yt-dlp"),
                crate::fetcher::resolve_fetcher_tool(&settings_ffmpeg_value, "ffmpeg"),
                expand_path(&settings_download_dir).is_dir(),
            )
        } else {
            (None, None, false)
        };

    RenderSnapshot {
        theme: th,
        is_fullscreen: snap_is_fullscreen,
        panel_focus: snap_panel_focus,
        input_mode: snap_input_mode,
        input_buffer: snap_input_buffer,
        show_help: snap_show_help,
        show_library_manager: snap_show_library_manager,
        selected_folder: snap_selected_folder,
        library_folders: snap_library_folders,
        now_playing_left: np_left,
        transport_album_line,
        visualizer_mode: snap_visualizer_mode,
        neon_enabled: snap_neon_enabled,
        spectrogram_mode: snap_spectrogram_mode,
        fullscreen_track_alpha,
        panel_content_blend,
        library_rows,
        player_pos: snap_player_pos,
        player_dur: snap_player_dur,
        volume: snap_volume,
        muted: snap_muted,
        shuffle: snap_shuffle,
        repeat: snap_repeat,
        is_playing: snap_is_playing,
        toast,
        library_track_count,
        download_progress: snap_download_progress,
        download_stage: snap_download_stage,
        filtered_library_len: total,
        filtered_library_selected: sel,
        show_settings,
        settings_row,
        settings_ytdlp_value,
        settings_ytdlp_resolved,
        settings_ffmpeg_value,
        settings_ffmpeg_resolved,
        settings_download_dir,
        settings_download_dir_exists,
    }
}

/// Draw one frame (visualizer background + overlays). Public for headless / integration tests.
pub fn draw_frame(
    f: &mut Frame<'_>,
    snap: &RenderSnapshot,
    viz_data: Option<&VisualizerData>,
    sub_frame_t: f32,
    renderer: &mut dyn Visualizer,
    scratch: &mut RenderScratch,
) {
    let full = f.area();
    let bg = parse_hex(&snap.theme.background);
    f.render_widget(Block::default().style(Style::default().bg(bg)), full);

    let glow = effective_glow(&snap.theme, snap.neon_enabled);
    let (viz_intensity, baseline) = if snap.is_fullscreen {
        (1.0, false)
    } else {
        match snap.panel_focus {
            PanelFocus::Normal => (0.55, true),
            PanelFocus::TransportOnly => (0.85, true),
        }
    };
    let rctx = RendererCtx {
        theme: &snap.theme,
        fullscreen: snap.is_fullscreen,
        glow,
        spectrogram_mode: snap.spectrogram_mode,
        viz_intensity,
        baseline,
    };

    if snap.is_fullscreen {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(full);
        renderer.render(f, chunks[0], viz_data, sub_frame_t, &rctx);
        if snap.fullscreen_track_alpha > 0.01 {
            let w = chunks[0].width.saturating_sub(2).max(1);
            let overlay = Rect::new(chunks[0].x + 1, chunks[0].y, w, 1);
            let fg_txt = parse_hex(&snap.theme.text);
            let fg = lerp_color(fg_txt, bg, 1.0 - snap.fullscreen_track_alpha);
            let line = Line::from(Span::styled(
                snap.now_playing_left.as_str(),
                Style::default().fg(fg),
            ));
            f.render_widget(Paragraph::new(line), overlay);
        }
        render_progress_bar_row(f, chunks[1], snap, &mut scratch.progress_spans);
        if snap.show_help {
            render_help(f, full, &snap.theme);
        }
        if snap.show_library_manager {
            render_library_manager(f, full, snap);
        }
        if snap.show_settings {
            render_settings(f, full, snap);
        }
        if let Some(ref t) = snap.toast {
            render_toast(f, full, t, &snap.theme, 0);
        }
        return;
    }

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
    let divider_row = chunks[2];
    let transport_area = chunks[3];

    match snap.panel_focus {
        PanelFocus::Normal => {
            let lib_w = ((body.width as u32 * 45 / 100).clamp(40, 80) as u16)
                .min(body.width.saturating_sub(20).max(1));
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(lib_w), Constraint::Min(20)])
                .split(body);
            let lib_rect = cols[0];
            let viz_rect = cols[1];
            renderer.render(f, viz_rect, viz_data, sub_frame_t, &rctx);
            render_library_card(f, lib_rect, snap, &mut scratch.library_items);
        }
        PanelFocus::TransportOnly => {
            renderer.render(f, body, viz_data, sub_frame_t, &rctx);
        }
    }

    render_transport_divider(f, divider_row, snap);

    render_status_strip(f, chunks[0], snap);
    render_transport(f, transport_area, snap, &mut scratch.controls_spans);

    if snap.show_help {
        render_help(f, full, &snap.theme);
    }
    if snap.show_library_manager {
        render_library_manager(f, full, snap);
    }
    if snap.show_settings {
        render_settings(f, full, snap);
    }
    if let Some(ref t) = snap.toast {
        render_toast(f, full, t, &snap.theme, 1);
    }
}

fn truncate_to_width(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(1);
    format!("{}…", s.chars().take(take).collect::<String>())
}

fn render_status_strip(f: &mut Frame<'_>, area: Rect, snap: &RenderSnapshot) {
    let bg = parse_hex(&snap.theme.surface);
    let dim = parse_hex(&snap.theme.text_dim);
    let pri = parse_hex(&snap.theme.primary);
    let acc = parse_hex(&snap.theme.accent);
    let txt = parse_hex(&snap.theme.text);
    let theme_key = normalize_theme_key(&snap.theme.name);
    let viz = snap.visualizer_mode.to_string();
    let sep = " · ";

    // ── Build right-side status items (priority: low → high, added in reverse) ──
    // Each item is (text, style). We build from right to left, drop items that
    // don't fit. Widths are measured in chars (ASCII-only for all these labels).
    let vol = if snap.muted {
        "MUTED".to_string()
    } else {
        format!(
            "{}%",
            (snap.volume * 100.0).round().clamp(0.0, 100.0) as i32
        )
    };
    let tpos = fmt_duration(snap.player_pos);
    let tdur = fmt_duration(snap.player_dur);
    let time_str = format!("{tpos}/{tdur}");
    let track_str = if snap.filtered_library_len > 0 {
        format!(
            "{}/{}",
            snap.filtered_library_selected.saturating_add(1),
            snap.filtered_library_len
        )
    } else {
        "—/—".to_string()
    };
    let shuf_lbl = if snap.shuffle { "SHUF" } else { "shuf" };
    let rep_lbl = match snap.repeat {
        RepeatMode::Off => "rep",
        RepeatMode::All => "REP",
        RepeatMode::One => "REP¹",
    };

    // Right section spans assembled right-to-left, then reversed.
    // Format: [viz · theme_key · ] track · time · vol · SHUF · REP [· NEON]
    struct RightItem {
        text: String,
        is_accent: bool,
    }
    let mut right_items: Vec<RightItem> = Vec::new();
    if snap.neon_enabled {
        right_items.push(RightItem {
            text: "NEON".into(),
            is_accent: true,
        });
    }
    right_items.push(RightItem {
        text: rep_lbl.to_string(),
        is_accent: snap.repeat != RepeatMode::Off,
    });
    right_items.push(RightItem {
        text: shuf_lbl.to_string(),
        is_accent: snap.shuffle,
    });
    right_items.push(RightItem {
        text: vol.clone(),
        is_accent: false,
    });
    right_items.push(RightItem {
        text: time_str.clone(),
        is_accent: false,
    });
    right_items.push(RightItem {
        text: track_str.clone(),
        is_accent: false,
    });
    // Theme and viz are lowest priority (dropped first on narrow terminals).
    right_items.push(RightItem {
        text: theme_key.clone(),
        is_accent: true,
    });
    right_items.push(RightItem {
        text: viz.clone(),
        is_accent: true,
    });
    right_items.reverse(); // Now: viz, theme, track, time, vol, shuf, rep, [neon]

    // Measure right section width (all items joined by " · ", with a leading " · ").
    let right_total_width: usize = right_items.iter().map(|i| sep.len() + i.text.len()).sum();

    // ── Left prefix: " RTUNES  · " ──
    let rtunes_label = " RTUNES ";
    let prefix_width = rtunes_label.len() + sep.len(); // " RTUNES " + " · "

    // ── Title budget ──
    let total = area.width as usize;
    let available = total.saturating_sub(prefix_width);

    // How much right section we can show (may be partial — drop items from left).
    // If right section doesn't fit at all, title gets all available space.
    let title_min = 8usize; // minimum chars reserved for the title
    let right_budget = available.saturating_sub(title_min);

    // Build the right spans that fit within right_budget, dropping lowest-priority items first.
    let mut right_spans: Vec<Span<'static>> = Vec::new();
    let mut right_used = 0usize;
    let mut first_right = true;
    for item in &right_items {
        let item_width = sep.len() + item.text.len();
        if right_used + item_width > right_budget {
            break; // remaining items don't fit — drop them
        }
        if !first_right {
            right_spans.push(Span::styled(sep, Style::default().fg(dim).bg(bg)));
        } else {
            right_spans.push(Span::styled(sep, Style::default().fg(dim).bg(bg)));
            first_right = false;
        }
        let style = if item.is_accent {
            Style::default().fg(acc).bg(bg)
        } else {
            Style::default().fg(txt).bg(bg)
        };
        right_spans.push(Span::styled(item.text.clone(), style));
        right_used += item_width;
    }
    let _ = right_total_width; // suppress unused warning

    // ── Title: fill the gap between prefix and right section ──
    let title_budget = available.saturating_sub(right_used);
    let title_raw = snap.now_playing_left.as_str();
    let title_chars: Vec<char> = title_raw.chars().collect();
    let title_len = title_chars.len();

    let title_show: String = if title_len == 0 || title_budget == 0 {
        String::new()
    } else if title_len <= title_budget {
        // Fits entirely — pad with spaces to push right section flush.
        let padding = title_budget - title_len;
        format!("{}{}", title_raw, " ".repeat(padding))
    } else {
        // Title is wider than the available space — truncate with ellipsis.
        let truncated: String = title_chars
            .iter()
            .take(title_budget.saturating_sub(1))
            .collect();
        format!("{}…", truncated)
    };

    // ── Assemble final line ──
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(
        rtunes_label,
        Style::default().fg(pri).bg(bg).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(sep, Style::default().fg(dim).bg(bg)));
    spans.push(Span::styled(title_show, Style::default().fg(txt).bg(bg)));
    spans.extend(right_spans);

    let line = Line::from(spans);
    f.render_widget(
        Paragraph::new(line)
            .style(Style::default().bg(bg))
            .alignment(Alignment::Left),
        area,
    );
}

/// Library pane (left column in [`PanelFocus::Normal`]): rounded border, surface fill, track list.
fn render_library_card(
    f: &mut Frame<'_>,
    body: Rect,
    snap: &RenderSnapshot,
    items_buf: &mut Vec<ListItem<'static>>,
) {
    let surf = parse_hex(&snap.theme.surface);
    let acc = parse_hex(&snap.theme.accent);
    let pri = parse_hex(&snap.theme.primary);
    let dim = parse_hex(&snap.theme.text_dim);
    let txt = parse_hex(&snap.theme.text);
    let hi_sel_bg = lerp_color(surf, parse_hex(&snap.theme.secondary), 0.18);

    let card = Rect::new(body.x, body.y, body.width.max(1), body.height.max(1));

    // Solid surface under entire card so the visualizer never bleeds through empty rows.
    f.render_widget(Clear, card);
    f.render_widget(Block::default().style(Style::default().bg(surf)), card);

    let border = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pri))
        .title(format!(" Library ({} tracks) ", snap.library_track_count))
        .title_style(
            Style::default()
                .fg(txt)
                .bg(surf)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(surf));
    let inner = border.inner(card);
    f.render_widget(border, card);
    let list_area = Rect::new(
        inner.x.saturating_add(1),
        inner.y,
        inner.width.saturating_sub(1).max(1),
        inner.height,
    );

    if snap.library_rows.is_empty() {
        let msg = "No tracks found. Press 'd' to download, 'a' to add a folder, or Ctrl+L for the Library Manager.";
        let inner_h = list_area.height.max(1);
        let w = list_area.width.max(1) as usize;
        let est_lines = msg.chars().count().div_ceil(w).max(1).min(inner_h as usize) as u16;
        let top_pad = inner_h.saturating_sub(est_lines) / 2;
        let p_rect = Rect::new(
            list_area.x,
            list_area.y + top_pad,
            list_area.width,
            est_lines.max(1),
        );
        let p = Paragraph::new(msg)
            .style(Style::default().fg(dim).bg(surf))
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center);
        f.render_widget(p, p_rect);
        return;
    }

    let inner_w = list_area.width as usize;
    let prefix_w = 2usize;
    let idx_w = 3usize;
    let gap_before_title = 1usize;
    let gap_before_dur = 1usize; // mandatory space between title ellipsis and duration
    let dur_w = 6usize;
    let title_w = inner_w
        .saturating_sub(
            prefix_w + gap_before_title + idx_w + gap_before_title + gap_before_dur + dur_w,
        )
        .max(4);

    items_buf.clear();
    for row in &snap.library_rows {
        let prefix = if row.is_playing { "▶" } else { " " };
        let title_fit = truncate_to_width(&row.title_str, title_w);
        let title_len = title_fit.chars().count();
        let dur_pad = format!("{:>width$}", row.duration_str, width = dur_w);
        let row_bg = if row.is_selected { hi_sel_bg } else { surf };
        let mut line_spans: Vec<Span> = vec![
            Span::styled(
                format!("{prefix} "),
                if row.is_playing {
                    Style::default()
                        .fg(pri)
                        .bg(row_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(dim).bg(row_bg)
                },
            ),
            Span::styled(
                format!("{} ", row.idx_str),
                Style::default().fg(dim).bg(row_bg),
            ),
        ];
        let title_style = if row.is_playing {
            Style::default()
                .fg(acc)
                .bg(row_bg)
                .add_modifier(Modifier::BOLD)
        } else if row.is_selected {
            Style::default()
                .fg(txt)
                .bg(hi_sel_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(txt).bg(surf)
        };
        line_spans.push(Span::styled(title_fit, title_style));
        // Gap before duration (always at least one space visually)
        line_spans.push(Span::styled(" ", Style::default().bg(row_bg)));
        let used =
            prefix_w + gap_before_title + idx_w + gap_before_title + title_len + gap_before_dur;
        let pad_to_dur = inner_w.saturating_sub(used + dur_w);
        if pad_to_dur > 0 {
            line_spans.push(Span::styled(
                " ".repeat(pad_to_dur),
                Style::default().bg(row_bg),
            ));
        }
        line_spans.push(Span::styled(dur_pad, Style::default().fg(dim).bg(row_bg)));

        items_buf.push(ListItem::new(Line::from(line_spans)));
    }

    let mut state = ListState::default();
    let sel_in_view = snap
        .library_rows
        .iter()
        .position(|r| r.is_selected)
        .unwrap_or(0);
    state.select(Some(sel_in_view));

    let list = List::new(std::mem::take(items_buf)).style(Style::default().bg(surf));
    f.render_stateful_widget(list, list_area, &mut state);
}

/// Progress row: `time ▏████◆▒▒▒▕ time` on `surface` with visible unfilled cells.
fn render_progress_bar_row(
    f: &mut Frame<'_>,
    area: Rect,
    snap: &RenderSnapshot,
    spans_buf: &mut Vec<Span<'static>>,
) {
    let row_bg = parse_hex(&snap.theme.surface);
    let dim = parse_hex(&snap.theme.text_dim);
    let acc = parse_hex(&snap.theme.accent);
    let left = fmt_duration(snap.player_pos);
    let right = fmt_duration(snap.player_dur);
    let ratio = if snap.player_dur > 0.0 {
        (snap.player_pos / snap.player_dur).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let lw = left.chars().count();
    let rw = right.chars().count();
    // ` {left} ` + ▏ + inner + ▕ + ` {right} `  =>  2+lw + 1 + inner + 1 + 2+rw
    let fixed = 6usize.saturating_add(lw).saturating_add(rw);
    let inner = area.width as usize;
    let inner = inner.saturating_sub(fixed);

    let fill = if inner == 0 {
        0usize
    } else {
        ((inner as f64) * ratio).round() as usize
    };
    let fill = fill.min(inner);

    let stops = &snap.theme.viz.gradient;
    spans_buf.clear();
    spans_buf.push(Span::styled(
        format!(" {left} "),
        Style::default().fg(dim).bg(row_bg),
    ));
    spans_buf.push(Span::styled("▏", Style::default().fg(dim).bg(row_bg)));
    for i in 0..inner {
        let t = if inner <= 1 {
            0.0
        } else {
            i as f32 / (inner - 1) as f32
        };
        let cell = if fill > 0 && fill < inner && i == fill {
            Span::styled(
                "◆",
                Style::default()
                    .fg(acc)
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            )
        } else if i < fill {
            let c = gradient_at(stops, t);
            Span::styled("█", Style::default().fg(c).bg(row_bg))
        } else {
            Span::styled("▒", Style::default().fg(dim).bg(row_bg))
        };
        spans_buf.push(cell);
    }
    spans_buf.push(Span::styled("▕", Style::default().fg(dim).bg(row_bg)));
    spans_buf.push(Span::styled(
        format!(" {right} "),
        Style::default().fg(dim).bg(row_bg),
    ));

    let line = Line::from(spans_buf.clone());
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(row_bg)),
        area,
    );
}

fn span_chip(theme: &Theme, key: &str, label: &str, out: &mut Vec<Span<'static>>) {
    let acc = parse_hex(&theme.accent);
    let pri = parse_hex(&theme.primary);
    let txt = parse_hex(&theme.text);
    let bg = parse_hex(&theme.surface);
    out.push(Span::styled("[", Style::default().fg(acc).bg(bg)));
    out.push(Span::styled(
        key.to_string(),
        Style::default().fg(pri).bg(bg).add_modifier(Modifier::BOLD),
    ));
    out.push(Span::styled(
        format!("]{label} "),
        Style::default().fg(txt).bg(bg),
    ));
}

fn render_transport_divider(f: &mut Frame<'_>, area: Rect, snap: &RenderSnapshot) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let dim = parse_hex(&snap.theme.text_dim);
    let bg = parse_hex(&snap.theme.background);
    let line = "─".repeat(area.width as usize);
    f.render_widget(
        Paragraph::new(line)
            .style(Style::default().fg(dim).bg(bg))
            .alignment(Alignment::Left),
        area,
    );
}

/// Bottom transport: now-playing, progress, hints (or input prompt on row 3).
fn render_transport(
    f: &mut Frame<'_>,
    area: Rect,
    snap: &RenderSnapshot,
    spans_buf: &mut Vec<Span<'static>>,
) {
    let surf = parse_hex(&snap.theme.surface);
    f.render_widget(Block::default().style(Style::default().bg(surf)), area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let acc = parse_hex(&snap.theme.accent);
    let dim = parse_hex(&snap.theme.text_dim);

    // Row A — now playing
    let left_a = snap.now_playing_left.clone();
    let right_a = snap.transport_album_line.clone();
    let w = rows[0].width as usize;
    let sep = " · ";
    let budget_right = right_a.chars().count() + sep.chars().count();
    let left_max = w.saturating_sub(budget_right).max(8);
    let left_show = truncate_to_width(&left_a, left_max);
    let line_a = Line::from(vec![
        Span::styled(
            format!(" {left_show}"),
            Style::default()
                .fg(acc)
                .bg(surf)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{sep}{right_a}"), Style::default().fg(dim).bg(surf)),
    ]);
    f.render_widget(Paragraph::new(line_a), rows[0]);

    render_progress_bar_row(f, rows[1], snap, spans_buf);

    // Row C — chips or input row
    match snap.input_mode {
        InputMode::Search => {
            f.render_widget(
                Paragraph::new(format!("> Search: {}_  [Enter] [Esc]", snap.input_buffer))
                    .style(Style::default().fg(dim).bg(surf)),
                rows[2],
            );
        }
        InputMode::DownloadUrl => {
            f.render_widget(
                Paragraph::new(format!("> URL: {}_  [Enter] [Esc]", snap.input_buffer))
                    .style(Style::default().fg(dim).bg(surf)),
                rows[2],
            );
        }
        InputMode::AddLibraryPath => {
            f.render_widget(
                Paragraph::new(format!("> Folder: {}_  [Enter] [Esc]", snap.input_buffer))
                    .style(Style::default().fg(dim).bg(surf)),
                rows[2],
            );
        }
        InputMode::LibraryManager => {
            f.render_widget(
                Paragraph::new("[a]Add  [x]Remove  [R]Rescan  [Esc]Close")
                    .style(Style::default().fg(dim).bg(surf)),
                rows[2],
            );
        }
        InputMode::Settings => {
            f.render_widget(
                Paragraph::new(
                    "[\u{2191}\u{2193}]Navigate  [Enter]Pick file  [d]Pick dir  [r]Reset  [Esc]Close",
                )
                .style(Style::default().fg(dim).bg(surf)),
                rows[2],
            );
        }
        InputMode::Normal => {
            spans_buf.clear();
            span_chip(&snap.theme, "Space", "Play", spans_buf);
            span_chip(&snap.theme, "/", "Search", spans_buf);
            span_chip(&snap.theme, "v", "Viz", spans_buf);
            span_chip(&snap.theme, "t", "Theme", spans_buf);
            span_chip(&snap.theme, "Tab", "Panels", spans_buf);
            span_chip(&snap.theme, "d", "Download", spans_buf);
            span_chip(&snap.theme, "Ctrl+L", "Library", spans_buf);
            span_chip(&snap.theme, "F2", "Settings", spans_buf);
            span_chip(&snap.theme, "?", "Help", spans_buf);
            span_chip(&snap.theme, "q", "Quit", spans_buf);
            if let (Some(p), Some(st)) = (snap.download_progress, snap.download_stage.as_deref()) {
                let pct = (p * 100.0).clamp(0.0, 100.0).round() as i32;
                spans_buf.push(Span::styled(
                    format!(" DL {pct}% — {} ", truncate_to_width(st, 24)),
                    Style::default()
                        .fg(acc)
                        .bg(surf)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            f.render_widget(
                Paragraph::new(Line::from(std::mem::take(spans_buf)))
                    .style(Style::default().bg(surf)),
                rows[2],
            );
        }
    }
}

/// Toast under the status strip (normal) or top-right (fullscreen). `status_rows` = height of
/// status strip above body (1 in normal layout, 0 in fullscreen).
fn render_toast(f: &mut Frame<'_>, full: Rect, text: &str, theme: &Theme, status_rows: u16) {
    let surf = parse_hex(&theme.surface);
    let acc = parse_hex(&theme.accent);
    let fg = parse_hex(&theme.text);
    let inner_w = ((text.chars().count() as u16).saturating_add(3)).clamp(10, 58);
    let w = inner_w.saturating_add(1).min(full.width);
    let h = 2u16.min(full.height.saturating_sub(status_rows));
    let x = full.x + full.width.saturating_sub(w);
    let y = full.y + status_rows;
    let area = Rect::new(
        x,
        y,
        w.min(full.width),
        h.max(1).min(full.height.saturating_sub(status_rows)),
    );
    f.render_widget(Clear, area);
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(1).max(1),
        area.height,
    );
    let accent_bar = Rect::new(area.x, area.y, 1, area.height);
    f.render_widget(
        Paragraph::new("▌").style(Style::default().fg(acc).bg(surf)),
        accent_bar,
    );
    let p = Paragraph::new(text)
        .style(Style::default().fg(fg).bg(surf))
        .wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}

fn help_chip_line(theme: &Theme, key: &str, action: &str) -> Line<'static> {
    let acc = parse_hex(&theme.accent);
    let pri = parse_hex(&theme.primary);
    let txt = parse_hex(&theme.text);
    Line::from(vec![
        Span::styled("[", Style::default().fg(acc)),
        Span::styled(
            key.to_string(),
            Style::default().fg(pri).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("] {action}"), Style::default().fg(txt)),
    ])
}

fn render_help(f: &mut Frame<'_>, full: Rect, theme: &Theme) {
    let w = (full.width as f32 * 0.7) as u16;
    let h = (full.height as f32 * 0.7) as u16;
    let x = full.x + (full.width.saturating_sub(w)) / 2;
    let y = full.y + (full.height.saturating_sub(h)) / 2;
    let area = Rect::new(x, y, w, h);
    f.render_widget(Clear, area);
    let surf = parse_hex(&theme.surface);
    let pri = parse_hex(&theme.primary);
    let block = Block::bordered()
        .border_style(Style::default().fg(pri))
        .title(Line::from(vec![
            Span::styled(" Help ", Style::default().fg(pri).bg(surf)),
            Span::styled(
                " · ",
                Style::default().fg(parse_hex(&theme.text_dim)).bg(surf),
            ),
            Span::styled(" ? ", Style::default().fg(pri).bg(surf)),
        ]));
    let inner = block.inner(area);
    f.render_widget(block.style(Style::default().bg(surf)), area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    fn section_title(theme: &Theme, t: &str) -> Line<'static> {
        Line::from(Span::styled(
            t.to_string(),
            Style::default()
                .fg(parse_hex(&theme.primary))
                .add_modifier(Modifier::BOLD),
        ))
    }

    let left_lines: Vec<Line<'static>> = vec![
        section_title(theme, "Playback"),
        help_chip_line(theme, "Space", "Play / Pause"),
        help_chip_line(theme, "n / p", "Next / Previous"),
        help_chip_line(theme, "← / →", "Seek ±5s (Shift ±30s)"),
        help_chip_line(theme, "l / h", "Seek ±5s (L/H ±30s)"),
        help_chip_line(theme, "+ / -", "Volume"),
        help_chip_line(theme, "m", "Mute"),
        help_chip_line(theme, "s", "Shuffle"),
        help_chip_line(theme, "r", "Repeat"),
        help_chip_line(theme, "Shift+R", "Rescan library"),
        Line::from(""),
        section_title(theme, "Navigation"),
        help_chip_line(theme, "↑ / ↓ / j / k", "Library cursor"),
        help_chip_line(theme, "Enter", "Play selected"),
        help_chip_line(theme, "/", "Search"),
        Line::from(""),
        section_title(theme, "Visualizer"),
        help_chip_line(theme, "v / V", "Cycle visualizer"),
        help_chip_line(theme, "1–9", "Jump visualizer"),
    ];

    let right_lines: Vec<Line<'static>> = vec![
        help_chip_line(theme, "t", "Cycle theme"),
        help_chip_line(theme, "g", "Neon toggle"),
        help_chip_line(theme, "Shift+M", "Spectrogram layout"),
        help_chip_line(theme, "f", "Fullscreen"),
        Line::from(""),
        section_title(theme, "Library"),
        help_chip_line(theme, "d", "Download URL"),
        help_chip_line(theme, "a", "Add folder"),
        help_chip_line(theme, "Ctrl+L", "Library manager"),
        help_chip_line(theme, "F2", "Settings (binaries)"),
        Line::from(""),
        section_title(theme, "General"),
        help_chip_line(theme, "Tab", "Panel focus"),
        help_chip_line(theme, "?", "This help"),
        help_chip_line(theme, "Esc", "Close / quit"),
        help_chip_line(theme, "q", "Quit"),
    ];

    let left = Paragraph::new(left_lines).style(Style::default().bg(surf));
    let right = Paragraph::new(right_lines).style(Style::default().bg(surf));
    f.render_widget(left, cols[0]);
    f.render_widget(right, cols[1]);
}

fn render_library_manager(f: &mut Frame<'_>, full: Rect, snap: &RenderSnapshot) {
    let w = (full.width as f32 * 0.7) as u16;
    let h = (full.height as f32 * 0.6) as u16;
    let x = full.x + (full.width.saturating_sub(w)) / 2;
    let y = full.y + (full.height.saturating_sub(h)) / 2;
    let area = Rect::new(x, y, w, h);
    f.render_widget(Clear, area);
    let block = Block::bordered().title(" Library Manager ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let bg = parse_hex(&snap.theme.surface);
    let fg = parse_hex(&snap.theme.text);
    let hi = parse_hex(&snap.theme.primary);

    let mut lines: Vec<Line> = vec![Line::from("Folders:")];
    let total_tracks = snap.library_track_count;
    for (i, folder) in snap.library_folders.iter().enumerate() {
        let prefix = if i == snap.selected_folder {
            "▸ "
        } else {
            "  "
        };
        let style = if i == snap.selected_folder {
            Style::default().fg(hi).bg(bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(fg).bg(bg)
        };
        lines.push(Line::from(vec![Span::styled(
            format!(
                "{}{} — {} tracks",
                prefix,
                folder.path.display(),
                folder.track_count
            ),
            style,
        )]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Total: {total_tracks} tracks (library)"),
        Style::default().fg(fg),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[a] Add  [x] Remove  [R] Rescan  [Esc] Close",
        Style::default().fg(parse_hex(&snap.theme.text_dim)),
    )));

    let p = Paragraph::new(lines).style(Style::default().bg(bg));
    f.render_widget(p, inner);
}

fn render_settings(f: &mut Frame<'_>, full: Rect, snap: &RenderSnapshot) {
    let w = ((full.width as f32 * 0.70) as u16).max(50).min(full.width);
    let h = 16u16.min(full.height);
    let x = full.x + (full.width.saturating_sub(w)) / 2;
    let y = full.y + (full.height.saturating_sub(h)) / 2;
    let area = Rect::new(x, y, w, h);
    f.render_widget(Clear, area);

    let bg = parse_hex(&snap.theme.surface);
    let fg = parse_hex(&snap.theme.text);
    let hi = parse_hex(&snap.theme.primary);
    let dim = parse_hex(&snap.theme.text_dim);

    let block = Block::bordered()
        .border_style(Style::default().fg(hi))
        .title(" Settings \u{2014} Binary Paths ")
        .title_style(Style::default().fg(hi).add_modifier(Modifier::BOLD));
    let inner = block.inner(area);
    f.render_widget(block.style(Style::default().bg(bg)), area);

    let ok_color = ratatui::style::Color::Green;
    let err_color = ratatui::style::Color::Red;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    // yt-dlp and ffmpeg rows
    let binary_rows: [(SettingsRow, &str, &str, &Option<std::path::PathBuf>); 2] = [
        (
            SettingsRow::YtDlp,
            "yt-dlp",
            &snap.settings_ytdlp_value,
            &snap.settings_ytdlp_resolved,
        ),
        (
            SettingsRow::Ffmpeg,
            "ffmpeg",
            &snap.settings_ffmpeg_value,
            &snap.settings_ffmpeg_resolved,
        ),
    ];
    for (row_id, label, value, resolved) in &binary_rows {
        let is_sel = snap.settings_row == *row_id;
        let prefix = if is_sel { "\u{25b8} " } else { "  " };
        let row_style = if is_sel {
            Style::default().fg(hi).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(fg)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{prefix}{label}: "), row_style),
            Span::styled(value.to_string(), Style::default().fg(fg)),
        ]));
        let (status_str, status_color) = match resolved {
            Some(p) => (format!("  \u{2713} {}", p.display()), ok_color),
            None => ("  \u{2717} not found".to_string(), err_color),
        };
        lines.push(Line::from(Span::styled(
            status_str,
            Style::default().fg(status_color),
        )));
        lines.push(Line::from(""));
    }

    // Download dir row
    {
        let is_sel = snap.settings_row == SettingsRow::DownloadDir;
        let prefix = if is_sel { "\u{25b8} " } else { "  " };
        let row_style = if is_sel {
            Style::default().fg(hi).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(fg)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{prefix}Download dir: "), row_style),
            Span::styled(snap.settings_download_dir.clone(), Style::default().fg(fg)),
        ]));
        let (status_str, status_color) = if snap.settings_download_dir_exists {
            ("\u{2713} exists".to_string(), ok_color)
        } else {
            (
                "\u{2717} not found (will be created on first download)".to_string(),
                err_color,
            )
        };
        lines.push(Line::from(Span::styled(
            format!("  {status_str}"),
            Style::default().fg(status_color),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "[Enter] pick file  [d] pick dir  [r] reset  [Esc] close",
        Style::default().fg(dim),
    )));

    let p = Paragraph::new(lines).style(Style::default().bg(bg));
    f.render_widget(p, inner);
}

/// Run the interactive TUI until [`AppState::quit`] is set.
#[allow(clippy::too_many_arguments)]
pub fn run(
    state: Arc<Mutex<AppState>>,
    theme: Arc<Mutex<Theme>>,
    config: Arc<Mutex<RtunesConfig>>,
    _config_path: std::path::PathBuf,
    custom_themes: Option<Arc<std::collections::HashMap<String, Theme>>>,
    deps: TuiDeps,
    fetch_rx: Receiver<FetchEvent>,
    picker_rx: crossbeam_channel::Receiver<crate::fetcher::PickerEvent>,
    fps: u8,
    silent_mode: bool,
    viz_rx: Receiver<Arc<VisualizerData>>,
) -> anyhow::Result<()> {
    let mut guard = TerminalGuard::new()?;
    if silent_mode {
        let mut g = lock_shared(&state);
        g.message = Some((
            "No audio device — TUI in silent mode. Connect speakers/headphones or a virtual audio device, then restart.".into(),
            Instant::now(),
        ));
    }

    let fps = fps.clamp(1, 120);
    let frame = Duration::from_millis(1000 / u64::from(fps));

    let tui_session_start = Instant::now();
    let mut logged_startup_ready = false;
    let mut render_scratch = RenderScratch::new();

    let mut latest_viz: Option<Arc<VisualizerData>> = None;
    let mut frame_idx: u64 = 0;
    let mut active_mode: Option<VisualizerMode> = None;
    let mut renderer: Box<dyn Visualizer> = Box::new(crate::visualizer::renderers::NoopVisualizer);
    let mut last_track_idx: Option<usize> = None;
    let mut last_track_change = Instant::now();
    // Read visualizer settings once (only changes on restart).
    let viz_settings = config.lock().unwrap().visualizer.clone();

    let mut prev_fullscreen = lock_shared(&state).is_fullscreen;
    let mut fullscreen_toggle_frame: Option<u64> = None;
    // Track transitions for device reconnect toasts and deferred rescan.
    let mut prev_silent_mode = lock_shared(&state).player.silent_mode;
    let mut prev_is_rescanning = false;

    loop {
        let t0 = Instant::now();
        loop {
            match viz_rx.try_recv() {
                Ok(f) => latest_viz = Some(f),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        frame_idx += 1;
        if frame_idx.is_multiple_of(60) {
            if let Some(ref v) = latest_viz {
                let t = smoothing::sub_frame_t(Instant::now(), v.timestamp, v.fft_period);
                let peak = v.bins_smoothed.iter().copied().fold(0.0f32, f32::max);
                tracing::debug!(
                    sub_frame = t,
                    bass = v.bass_energy,
                    beat = v.beat,
                    peak_bin = peak,
                    "viz frame"
                );
            }
        }

        while event::poll(Duration::ZERO)? {
            let ev = event::read()?;
            handle_event(&state, &theme, custom_themes.as_deref(), &deps, &ev)?;
        }

        while let Ok(ev) = fetch_rx.try_recv() {
            match ev {
                FetchEvent::Stage(s) => {
                    let short = if s.chars().count() > 72 {
                        format!("{}…", s.chars().take(72).collect::<String>())
                    } else {
                        s
                    };
                    let mut g = lock_shared(&state);
                    g.download_stage = Some(short);
                }
                FetchEvent::Progress(p) => {
                    let mut g = lock_shared(&state);
                    g.download_progress = Some(p);
                }
                FetchEvent::Done(path) => {
                    let title = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "track".into());
                    // Ensure the directory that received the download is in the
                    // library so a rescan will pick it up.
                    if let Some(dl_dir) = path.parent() {
                        let dl_canon =
                            dunce::canonicalize(dl_dir).unwrap_or_else(|_| dl_dir.to_path_buf());
                        let covered = {
                            let c = lock_shared(&config);
                            c.app.library_paths.iter().any(|lp| {
                                dunce::canonicalize(expand_path(lp))
                                    .map(|c| dl_canon.starts_with(&c))
                                    .unwrap_or(false)
                            })
                        };
                        if !covered {
                            let dl_str = dl_canon.to_string_lossy().into_owned();
                            {
                                let mut c = lock_shared(&config);
                                c.app.library_paths.push(dl_str.clone());
                                if let Err(e) = crate::config::save(&deps.config_path, &c.clone()) {
                                    tracing::warn!(error = %e, "failed to save config after auto-adding download dir");
                                }
                            }
                            {
                                let cfg_snap = lock_shared(&config).clone();
                                let mut g = lock_shared(&state);
                                crate::tui::events::sync_library_folders_from_config(
                                    &mut g, &cfg_snap,
                                );
                            }
                        }
                    }
                    {
                        let mut g = lock_shared(&state);
                        g.download_progress = None;
                        g.download_stage = None;
                        g.message = Some((format!("Downloaded: {title}"), Instant::now()));
                    }
                    trigger_rescan(&state, &config);
                }
                FetchEvent::Failed(msg) => {
                    let mut g = lock_shared(&state);
                    g.download_progress = None;
                    g.download_stage = None;
                    g.message = Some((msg, Instant::now()));
                }
            }
        }

        // Drain picker results (binary paths chosen via the native OS dialog).
        while let Ok(ev) = picker_rx.try_recv() {
            crate::tui::events::handle_picker_event(&state, &deps, ev);
        }

        let now = Instant::now();
        let mut should_quit = false;
        let mut fire_deferred_rescan = false;
        let (fullscreen_track_alpha, panel_content_blend) = {
            let mut g = lock_shared(&state);
            if let Some((_, t_msg)) = g.message.as_ref() {
                if t_msg.elapsed() > Duration::from_secs(5) {
                    g.message = None;
                }
            }
            if g.quit {
                should_quit = true;
            }
            let idx = g.player.current_index;
            if idx != last_track_idx {
                last_track_idx = idx;
                last_track_change = now;
            }
            if g.is_fullscreen != prev_fullscreen {
                prev_fullscreen = g.is_fullscreen;
                fullscreen_toggle_frame = Some(frame_idx);
            }
            // Silent mode transitions → toast feedback.
            let cur_silent = g.player.silent_mode;
            if cur_silent && !prev_silent_mode {
                g.message = Some((
                    "Audio device lost. Press Ctrl+D to reconnect.".into(),
                    Instant::now(),
                ));
            } else if !cur_silent && prev_silent_mode {
                g.message = Some(("Audio device reconnected.".into(), Instant::now()));
            }
            prev_silent_mode = cur_silent;
            // Deferred rescan: detect when a scan finishes with a pending request.
            let rescan_just_finished = prev_is_rescanning && !g.is_rescanning;
            prev_is_rescanning = g.is_rescanning;
            if rescan_just_finished && g.rescan_pending {
                g.rescan_pending = false;
                fire_deferred_rescan = true;
            }
            let fs_alpha = if g.is_fullscreen {
                fullscreen_overlay_alpha(now, last_track_change)
            } else {
                0.0
            };
            let pb = if let Some(tf) = fullscreen_toggle_frame {
                let age = frame_idx.saturating_sub(tf);
                if age >= 4 {
                    1.0
                } else {
                    ((age + 1) as f32 / 4.0).min(1.0)
                }
            } else {
                1.0
            };
            (fs_alpha, pb)
        };
        if fire_deferred_rescan {
            trigger_rescan(&state, &config);
        }
        if should_quit {
            break;
        }

        let lib_h = {
            let th = guard
                .terminal()
                .size()
                .map(|r| r.height as usize)
                .unwrap_or(24);
            let body_h = th.saturating_sub(1 + 1 + 3);
            body_h.saturating_sub(4).max(4)
        };
        let snap = build_snapshot(
            &state,
            &theme,
            lib_h,
            fullscreen_track_alpha,
            panel_content_blend,
        );

        let want = snap.visualizer_mode;
        if Some(want) != active_mode {
            renderer = make_renderer(want, &viz_settings);
            active_mode = Some(want);
        }
        let sub_frame_t = latest_viz
            .as_ref()
            .map(|v| smoothing::sub_frame_t(Instant::now(), v.timestamp, v.fft_period))
            .unwrap_or(1.0);
        guard.terminal().draw(|f| {
            draw_frame(
                f,
                &snap,
                latest_viz.as_deref(),
                sub_frame_t,
                renderer.as_mut(),
                &mut render_scratch,
            )
        })?;

        if !logged_startup_ready {
            logged_startup_ready = true;
            tracing::info!(
                elapsed_ms = tui_session_start.elapsed().as_millis() as u64,
                "tui_startup_ready"
            );
        }

        let elapsed = t0.elapsed();
        if elapsed < frame {
            std::thread::sleep(frame - elapsed);
        } else {
            tracing::trace!("frame over budget");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{PanelFocus, Track, VisualizerMode};
    use crate::config::{resolve_active_theme, RtunesConfig};
    use crate::tui::color::parse_hex;
    use crate::visualizer::renderers::make_renderer;
    use crate::visualizer::VisualizerData;
    use ratatui::backend::TestBackend;
    use ratatui::layout::{Constraint, Direction, Layout, Rect};
    use ratatui::widgets::{Block, BorderType};
    use ratatui::Terminal;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn default_cfg_yaml() -> RtunesConfig {
        const YAML: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/default_config.yaml"
        ));
        serde_yaml::from_str(YAML).expect("default config")
    }

    #[test]
    fn fullscreen_overlay_elapsed_hold_then_fade() {
        assert!((fullscreen_overlay_elapsed(Duration::ZERO) - 1.0).abs() < 1e-5);
        assert!((fullscreen_overlay_elapsed(Duration::from_secs(2)) - 1.0).abs() < 1e-5);
        let mid = fullscreen_overlay_elapsed(Duration::from_secs_f32(3.5));
        assert!(
            mid > 0.0 && mid < 1.0,
            "mid fade should be between 0 and 1, got {mid}"
        );
        assert!(fullscreen_overlay_elapsed(Duration::from_secs(5)) < 0.01);
    }

    #[test]
    fn fullscreen_overlay_alpha_uses_elapsed() {
        let t0 = Instant::now();
        assert!((fullscreen_overlay_alpha(t0, t0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn status_strip_truncates_gracefully_on_narrow_terminal() {
        let cfg = default_cfg_yaml();
        let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let state = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
        let theme_arc = Arc::new(Mutex::new(th));
        {
            let mut g = state.lock().unwrap();
            g.library.push(Track {
                id: "1".into(),
                filepath: PathBuf::from("x.mp3"),
                title: "A".repeat(120),
                artist: Some("Artist".into()),
                album: None,
                duration_secs: 200,
            });
            g.filtered_indices = vec![0];
            g.player.current_index = Some(0);
            g.player.duration_secs = 100.0;
            g.player.position_secs = 50.0;
        }
        let snap = build_snapshot(&state, &theme_arc, 4, 0.0, 1.0);
        let backend = TestBackend::new(32, 12);
        let mut term = Terminal::new(backend).unwrap();
        let mut scratch = RenderScratch::new();
        let mut viz = crate::visualizer::renderers::NoopVisualizer;
        term.draw(|f| draw_frame(f, &snap, None, 1.0, &mut viz, &mut scratch))
            .unwrap();
        let buf = term.backend().buffer();
        let w = buf.area().width;
        let mut row0 = String::new();
        for x in 0..w {
            row0.push_str(buf[(x, 0)].symbol());
        }
        assert!(
            row0.contains("RTUNES"),
            "status strip should start with branding: {row0:?}"
        );
        assert!(
            row0.contains('…'),
            "long tail should truncate with ellipsis on narrow terminal: {row0:?}"
        );
    }

    #[test]
    fn transport_row_playhead_inside_fill() {
        let cfg = default_cfg_yaml();
        let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let state = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
        let theme_arc = Arc::new(Mutex::new(th));
        {
            let mut g = state.lock().unwrap();
            g.library.push(Track {
                id: "1".into(),
                filepath: PathBuf::from("x.mp3"),
                title: "Song".into(),
                artist: Some("Artist".into()),
                album: Some("Album".into()),
                duration_secs: 100,
            });
            g.filtered_indices = vec![0];
            g.player.current_index = Some(0);
            g.player.duration_secs = 100.0;
            g.player.position_secs = 50.0;
        }
        let snap = build_snapshot(&state, &theme_arc, 4, 0.0, 1.0);
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let mut scratch = RenderScratch::new();
        let mut viz = crate::visualizer::renderers::NoopVisualizer;
        term.draw(|f| draw_frame(f, &snap, None, 1.0, &mut viz, &mut scratch))
            .unwrap();
        let buf = term.backend().buffer();
        let mut found = false;
        for y in 0..24u16 {
            for x in 0..80u16 {
                if buf[(x, y)].symbol() == "◆" {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        assert!(
            found,
            "progress row should render diamond playhead when 0 < fill < width"
        );
    }

    fn normal_layout_body_and_transport(
        full: ratatui::layout::Rect,
    ) -> (ratatui::layout::Rect, ratatui::layout::Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(1),
                Constraint::Length(3),
            ])
            .split(full);
        (chunks[1], chunks[3])
    }

    /// Matches [`draw_frame`] `PanelFocus::Normal` left column (`lib_rect`).
    fn library_pane_rect(body: ratatui::layout::Rect) -> ratatui::layout::Rect {
        let lib_w = ((body.width as u32 * 45 / 100).clamp(40, 80) as u16)
            .min(body.width.saturating_sub(20).max(1));
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(lib_w), Constraint::Min(20)])
            .split(body);
        cols[0]
    }

    #[test]
    fn library_card_fills_full_height_with_surface_bg() {
        let cfg = default_cfg_yaml();
        let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let state = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
        let theme_arc = Arc::new(Mutex::new(th.clone()));
        {
            let mut g = state.lock().unwrap();
            g.panel_focus = PanelFocus::Normal;
            g.is_fullscreen = false;
            for i in 0..2 {
                g.library.push(Track {
                    id: format!("{i}"),
                    filepath: PathBuf::from(format!("t{i}.mp3")),
                    title: format!("Track {i}"),
                    artist: None,
                    album: None,
                    duration_secs: 60,
                });
            }
            g.filtered_indices = vec![0, 1];
            g.selected_track = 0;
            g.player.current_index = Some(0);
            g.player.duration_secs = 60.0;
            g.player.position_secs = 0.0;
        }
        let snap = build_snapshot(&state, &theme_arc, 8, 0.0, 1.0);
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let mut scratch = RenderScratch::new();
        let mut viz = crate::visualizer::renderers::NoopVisualizer;
        term.draw(|f| draw_frame(f, &snap, None, 1.0, &mut viz, &mut scratch))
            .unwrap();
        let buf = term.backend().buffer();
        let full = Rect::new(0, 0, 80, 24);
        let (body, _) = normal_layout_body_and_transport(full);
        let card = library_pane_rect(body);
        let surf = parse_hex(&snap.theme.surface);
        let pri = parse_hex(&snap.theme.primary);
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
        assert!(
            list_area.height as usize > n + 1,
            "test needs dead zone rows below short track list"
        );
        for y in (list_area.y + n as u16)..(list_area.y + list_area.height) {
            for x in list_area.x..(list_area.x + list_area.width) {
                assert_eq!(
                    buf[(x, y)].bg,
                    surf,
                    "rows below list should stay surface-filled (no viz bleed) at ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn viz_renders_only_into_right_pane_in_normal_mode() {
        let cfg = default_cfg_yaml();
        let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let state = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
        let theme_arc = Arc::new(Mutex::new(th.clone()));
        {
            let mut g = state.lock().unwrap();
            g.panel_focus = PanelFocus::Normal;
            g.is_fullscreen = false;
            g.visualizer_mode = VisualizerMode::Spectrum;
            for i in 0..2 {
                g.library.push(Track {
                    id: format!("{i}"),
                    filepath: PathBuf::from(format!("t{i}.mp3")),
                    title: format!("Track {i}"),
                    artist: None,
                    album: None,
                    duration_secs: 60,
                });
            }
            g.filtered_indices = vec![0, 1];
            g.selected_track = 0;
            g.player.current_index = Some(0);
            g.player.duration_secs = 60.0;
            g.player.position_secs = 0.0;
        }
        let snap = build_snapshot(&state, &theme_arc, 8, 0.0, 1.0);
        let mut vd = VisualizerData::empty(64);
        vd.loudness = 1.0;
        for b in vd.bins_smoothed.iter_mut() {
            *b = 0.5;
        }
        vd.bins_prev.copy_from_slice(&vd.bins_smoothed);
        vd.bins_peak.copy_from_slice(&vd.bins_smoothed);

        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let mut scratch = RenderScratch::new();
        let mut viz = make_renderer(
            VisualizerMode::Spectrum,
            &crate::config::VisualizerSettings::default(),
        );
        term.draw(|f| draw_frame(f, &snap, Some(&vd), 1.0, &mut *viz, &mut scratch))
            .unwrap();
        let buf = term.backend().buffer();
        let full = Rect::new(0, 0, 80, 24);
        let (body, _) = normal_layout_body_and_transport(full);
        let lib = library_pane_rect(body);

        for y in lib.y..lib.y.saturating_add(lib.height) {
            for x in lib.x..lib.x.saturating_add(lib.width) {
                let sym = buf[(x, y)].symbol();
                assert!(
                    !sym.contains('█') && !sym.contains('▔'),
                    "spectrum block glyphs must not appear in library pane at ({x},{y}): {sym:?}"
                );
            }
        }
    }

    #[test]
    fn progress_row_renders_unfilled_cells_visibly() {
        let cfg = default_cfg_yaml();
        let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let state = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
        let theme_arc = Arc::new(Mutex::new(th.clone()));
        {
            let mut g = state.lock().unwrap();
            g.library.push(Track {
                id: "1".into(),
                filepath: PathBuf::from("x.mp3"),
                title: "Song".into(),
                artist: None,
                album: None,
                duration_secs: 3600,
            });
            g.filtered_indices = vec![0];
            g.player.current_index = Some(0);
            g.player.duration_secs = 3600.0;
            g.player.position_secs = 30.0;
        }
        let snap = build_snapshot(&state, &theme_arc, 4, 0.0, 1.0);
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let mut scratch = RenderScratch::new();
        let mut viz = crate::visualizer::renderers::NoopVisualizer;
        term.draw(|f| draw_frame(f, &snap, None, 1.0, &mut viz, &mut scratch))
            .unwrap();
        let buf = term.backend().buffer();
        let mut found = false;
        for y in 0..24u16 {
            for x in 0..80u16 {
                let c = &buf[(x, y)];
                if c.symbol() == "▒" {
                    found = true;
                    let row_bg = parse_hex(&th.surface);
                    let dim = parse_hex(&th.text_dim);
                    assert_eq!(c.bg, row_bg, "▒ cell should sit on surface row bg");
                    assert_ne!(
                        c.fg, c.bg,
                        "unfilled progress glyph fg should contrast surface bg"
                    );
                    assert_eq!(c.fg, dim, "▒ should use text_dim fg");
                }
            }
        }
        assert!(found, "expected at least one ▒ unfilled progress cell");
    }

    #[test]
    fn transport_band_uses_surface_background() {
        let cfg = default_cfg_yaml();
        let th = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let state = Arc::new(Mutex::new(AppState::new(&cfg, th.clone())));
        let theme_arc = Arc::new(Mutex::new(th.clone()));
        let snap = build_snapshot(&state, &theme_arc, 4, 0.0, 1.0);
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        let mut scratch = RenderScratch::new();
        let mut viz = crate::visualizer::renderers::NoopVisualizer;
        term.draw(|f| draw_frame(f, &snap, None, 1.0, &mut viz, &mut scratch))
            .unwrap();
        let buf = term.backend().buffer();
        let full = Rect::new(0, 0, 80, 24);
        let (_, transport) = normal_layout_body_and_transport(full);
        let surf = parse_hex(&th.surface);
        for y in transport.y..transport.y.saturating_add(transport.height) {
            for x in transport.x..transport.x.saturating_add(transport.width) {
                assert_eq!(
                    buf[(x, y)].bg,
                    surf,
                    "transport band should use surface bg at ({x},{y})"
                );
            }
        }
    }
}
