//! Parse theme hex strings into ratatui colors and interpolate gradients.

use ratatui::style::Color;

/// Parse `#RRGGBB` or `RRGGBB` (first 6 hex digits if longer). Invalid input returns [`Color::Reset`].
pub fn parse_hex(s: &str) -> Color {
    let s = s.trim();
    let hex = s.strip_prefix('#').unwrap_or(s);
    let hex = if hex.len() >= 8 {
        &hex[..6]
    } else {
        hex
    };
    if hex.len() != 6 {
        tracing::warn!(value = %s, "invalid hex color");
        return Color::Reset;
    }
    let r = u8::from_str_radix(&hex[0..2], 16);
    let g = u8::from_str_radix(&hex[2..4], 16);
    let b = u8::from_str_radix(&hex[4..6], 16);
    match (r, g, b) {
        (Ok(r), Ok(g), Ok(b)) => Color::Rgb(r, g, b),
        _ => {
            tracing::warn!(value = %s, "invalid hex color");
            Color::Reset
        }
    }
}

/// Linear RGB interpolation; non-`Rgb` endpoints return `a` if `t < 0.5` else `b`.
/// Blend `foreground` toward `background` by `(1 - intensity)` (for viz-under-chrome dimming).
#[inline]
pub fn dim_with_intensity(foreground: Color, background: Color, intensity: f32) -> Color {
    let t = intensity.clamp(0.0, 1.0);
    lerp_color(foreground, background, 1.0 - t)
}

pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else {
        return if t < 0.5 { a } else { b };
    };
    let lerp = |x: u8, y: u8| -> u8 { (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8 };
    Color::Rgb(lerp(ar, br), lerp(ag, bg), lerp(ab, bb))
}

/// Map `t ∈ [0,1]` across `stops` (at least one); piecewise linear in RGB space.
pub fn gradient_at(stops: &[String], t: f32) -> Color {
    if stops.is_empty() {
        return Color::Reset;
    }
    if stops.len() == 1 {
        return parse_hex(&stops[0]);
    }
    let t = t.clamp(0.0, 1.0);
    let n = stops.len() - 1;
    let x = t * n as f32;
    let i = (x.floor() as usize).min(n - 1);
    let local = x - i as f32;
    let c0 = parse_hex(&stops[i]);
    let c1 = parse_hex(&stops[i + 1]);
    lerp_color(c0, c1, local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn parse_hex_basic() {
        assert_eq!(parse_hex("#FF00FF"), Color::Rgb(255, 0, 255));
        assert_eq!(parse_hex("00FF00"), Color::Rgb(0, 255, 0));
        assert_eq!(parse_hex("abc"), Color::Reset);
    }

    #[test]
    fn gradient_at_endpoints() {
        let s = vec!["#FF0000".into(), "#0000FF".into()];
        assert_eq!(gradient_at(&s, 0.0), Color::Rgb(255, 0, 0));
        assert_eq!(gradient_at(&s, 1.0), Color::Rgb(0, 0, 255));
    }

    #[test]
    fn gradient_at_midpoint_lerps() {
        let s = vec!["#000000".into(), "#FFFFFF".into()];
        let m = gradient_at(&s, 0.5);
        assert_eq!(m, Color::Rgb(128, 128, 128));
    }
}
