//! Built-in themes and theme resolution.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Bar rendering style for spectrum-style visualizers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarStyle {
    Solid,
    Rounded,
    Dots,
}

/// Visualizer-specific colors and options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VizTheme {
    pub gradient: Vec<String>,
    pub glow: bool,
    pub bar_style: BarStyle,
    pub particle_colors: Vec<String>,
    pub wave_color: String,
    pub wave_trail: String,
}

/// Full theme: UI colors + visualizer palette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub background: String,
    pub surface: String,
    pub primary: String,
    pub secondary: String,
    pub text: String,
    pub text_dim: String,
    pub accent: String,
    pub viz: VizTheme,
}

/// Runtime neon toggle (`g`) overrides the theme default for glow/halo effects.
#[inline]
pub fn effective_glow(_theme: &Theme, neon_enabled: bool) -> bool {
    neon_enabled
}

fn synthwave() -> Theme {
    Theme {
        name: "Synthwave".into(),
        background: "#0f0a1e".into(),
        surface: "#1a1035".into(),
        primary: "#f92a82".into(),
        secondary: "#edfd09".into(),
        text: "#ffffff".into(),
        text_dim: "#7b6b8a".into(),
        accent: "#00d9ff".into(),
        viz: VizTheme {
            gradient: vec![
                "#6b0f6b".into(),
                "#f92a82".into(),
                "#edfd09".into(),
                "#00d9ff".into(),
            ],
            glow: true,
            bar_style: BarStyle::Rounded,
            particle_colors: vec![
                "#f92a82".into(),
                "#edfd09".into(),
                "#00d9ff".into(),
                "#ff6b35".into(),
            ],
            wave_color: "#f92a82".into(),
            wave_trail: "#6b0f6b".into(),
        },
    }
}

fn dracula() -> Theme {
    Theme {
        name: "Dracula".into(),
        background: "#282a36".into(),
        surface: "#44475a".into(),
        primary: "#ff79c6".into(),
        secondary: "#8be9fd".into(),
        text: "#f8f8f2".into(),
        text_dim: "#6272a4".into(),
        accent: "#50fa7b".into(),
        viz: VizTheme {
            gradient: vec![
                "#6272a4".into(),
                "#bd93f9".into(),
                "#ff79c6".into(),
                "#8be9fd".into(),
            ],
            glow: true,
            bar_style: BarStyle::Rounded,
            particle_colors: vec![
                "#ff79c6".into(),
                "#8be9fd".into(),
                "#bd93f9".into(),
                "#50fa7b".into(),
                "#ffb86c".into(),
            ],
            wave_color: "#50fa7b".into(),
            wave_trail: "#2d4a3e".into(),
        },
    }
}

fn nord() -> Theme {
    Theme {
        name: "Nord".into(),
        background: "#2e3440".into(),
        surface: "#3b4252".into(),
        primary: "#88c0d0".into(),
        secondary: "#81a1c1".into(),
        text: "#eceff4".into(),
        text_dim: "#4c566a".into(),
        accent: "#a3be8c".into(),
        viz: VizTheme {
            gradient: vec![
                "#4c566a".into(),
                "#5e81ac".into(),
                "#88c0d0".into(),
                "#eceff4".into(),
            ],
            glow: false,
            bar_style: BarStyle::Rounded,
            particle_colors: vec![
                "#88c0d0".into(),
                "#81a1c1".into(),
                "#a3be8c".into(),
                "#ebcb8b".into(),
            ],
            wave_color: "#88c0d0".into(),
            wave_trail: "#3b4252".into(),
        },
    }
}

fn tokyo_night() -> Theme {
    Theme {
        name: "Tokyo Night".into(),
        background: "#1a1b26".into(),
        surface: "#24283b".into(),
        primary: "#7aa2f7".into(),
        secondary: "#bb9af7".into(),
        text: "#c0caf5".into(),
        text_dim: "#565f89".into(),
        accent: "#9ece6a".into(),
        viz: VizTheme {
            gradient: vec![
                "#565f89".into(),
                "#7aa2f7".into(),
                "#bb9af7".into(),
                "#ff9e64".into(),
            ],
            glow: true,
            bar_style: BarStyle::Rounded,
            particle_colors: vec![
                "#7aa2f7".into(),
                "#bb9af7".into(),
                "#ff9e64".into(),
                "#9ece6a".into(),
                "#f7768e".into(),
            ],
            wave_color: "#7dcfff".into(),
            wave_trail: "#1a3a5c".into(),
        },
    }
}

fn monochrome() -> Theme {
    Theme {
        name: "Monochrome".into(),
        background: "#0a0a0a".into(),
        surface: "#1a1a1a".into(),
        primary: "#ffffff".into(),
        secondary: "#888888".into(),
        text: "#e0e0e0".into(),
        text_dim: "#555555".into(),
        accent: "#ffffff".into(),
        viz: VizTheme {
            gradient: vec![
                "#333333".into(),
                "#666666".into(),
                "#aaaaaa".into(),
                "#ffffff".into(),
            ],
            glow: false,
            bar_style: BarStyle::Rounded,
            particle_colors: vec![
                "#ffffff".into(),
                "#cccccc".into(),
                "#999999".into(),
                "#666666".into(),
            ],
            wave_color: "#ffffff".into(),
            wave_trail: "#333333".into(),
        },
    }
}

/// All built-in themes keyed by normalized id (`synthwave`, `tokyo_night`, …).
pub fn builtin_themes() -> HashMap<String, Theme> {
    HashMap::from([
        ("synthwave".into(), synthwave()),
        ("dracula".into(), dracula()),
        ("nord".into(), nord()),
        ("tokyo_night".into(), tokyo_night()),
        ("monochrome".into(), monochrome()),
    ])
}

/// Normalize a theme name or id for lookup (`"Tokyo Night"` → `"tokyo_night"`).
pub fn normalize_theme_key(active: &str) -> String {
    active.trim().to_lowercase().replace([' ', '-'], "_")
}

/// Resolve `active` against optional `custom` themes, then builtins. Unknown → Synthwave + warn.
pub fn resolve_active_theme(active: &str, custom: Option<&HashMap<String, Theme>>) -> Theme {
    let key = normalize_theme_key(active);
    if let Some(map) = custom {
        if let Some(t) = map.get(&key).or_else(|| map.get(active.trim())) {
            return t.clone();
        }
        // YAML keys may use display names; try case-insensitive match on Theme.name
        for t in map.values() {
            if normalize_theme_key(&t.name) == key {
                return t.clone();
            }
        }
    }
    let builtins = builtin_themes();
    if let Some(t) = builtins.get(&key) {
        return t.clone();
    }
    tracing::warn!(theme = %active, "unknown theme; falling back to Synthwave");
    synthwave()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_names_resolve() {
        assert_eq!(resolve_active_theme("synthwave", None).name, "Synthwave");
        assert_eq!(resolve_active_theme("dracula", None).name, "Dracula");
        assert_eq!(resolve_active_theme("nord", None).name, "Nord");
        assert_eq!(
            resolve_active_theme("tokyo_night", None).name,
            "Tokyo Night"
        );
        assert_eq!(resolve_active_theme("monochrome", None).name, "Monochrome");
    }

    #[test]
    fn unknown_theme_falls_back_to_synthwave() {
        let t = resolve_active_theme("no_such_theme", None);
        assert_eq!(t.name, "Synthwave");
    }

    #[test]
    fn custom_overrides_builtin_same_key() {
        let mut custom = HashMap::new();
        let mut t = synthwave();
        t.primary = "#010101".into();
        custom.insert("synthwave".into(), t.clone());
        let got = resolve_active_theme("synthwave", Some(&custom));
        assert_eq!(got.primary, "#010101");
    }
}
