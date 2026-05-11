//! Shared helpers for ratatui canvas-based visualizers.

use ratatui::style::Color;
use ratatui::widgets::canvas::{Context, Points};

use crate::config::Theme;
use crate::tui::color::{gradient_at, lerp_color, parse_hex};

/// Map `t ∈ [0,1]` across theme gradient stops.
pub fn gradient_color(stops: &[String], t: f32) -> Color {
    gradient_at(stops, t)
}

/// Catmull–Rom spline through `points` (at least 2). Returns dense polyline in the same space.
pub fn catmull_rom(points: &[(f32, f32)], samples_per_segment: usize) -> Vec<(f32, f32)> {
    let n = points.len();
    if n < 2 {
        return points.to_vec();
    }
    if n == 2 {
        return vec![points[0], points[1]];
    }
    let seg = samples_per_segment.max(2);
    // Pre-allocate: each segment contributes `seg - 1` points plus the final endpoint.
    let mut out = Vec::with_capacity((n - 1) * (seg - 1) + 1);

    fn cr(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), t: f32) -> (f32, f32) {
        let t2 = t * t;
        let t3 = t2 * t;
        let x = 0.5
            * ((2.0 * p1.0)
                + (-p0.0 + p2.0) * t
                + (2.0 * p0.0 - 5.0 * p1.0 + 4.0 * p2.0 - p3.0) * t2
                + (-p0.0 + 3.0 * p1.0 - 3.0 * p2.0 + p3.0) * t3);
        let y = 0.5
            * ((2.0 * p1.1)
                + (-p0.1 + p2.1) * t
                + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2
                + (-p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1) * t3);
        (x, y)
    }

    for i in 0..n - 1 {
        let p0 = if i == 0 { points[0] } else { points[i - 1] };
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = if i + 2 < n {
            points[i + 2]
        } else {
            points[i + 1]
        };
        let start_s = if i == 0 { 0 } else { 1 };
        for s in start_s..seg {
            let t = s as f32 / (seg - 1) as f32;
            out.push(cr(p0, p1, p2, p3, t));
        }
    }
    out.push(points[n - 1]);
    out
}

/// Companion glow pass: ±1 grid dot offsets in canvas space, blended toward background.
pub fn glow_pass(
    ctx: &mut Context<'_>,
    width: u16,
    height: u16,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    primary_pts: &[(f64, f64)],
    theme: &Theme,
    primary: Color,
    glow_enabled: bool,
) {
    if !glow_enabled || primary_pts.is_empty() {
        return;
    }
    let bg = parse_hex(&theme.background);
    let glow_c = lerp_color(primary, bg, 0.6);
    let res_x = f64::from(width) * 2.0;
    let res_y = f64::from(height) * 4.0;
    let xspan = (x_bounds[1] - x_bounds[0]).abs();
    let yspan = (y_bounds[1] - y_bounds[0]).abs();
    if res_x <= 1.0 || res_y <= 1.0 || xspan <= 0.0 || yspan <= 0.0 {
        return;
    }
    let dx = xspan / (res_x - 1.0);
    let dy = yspan / (res_y - 1.0);
    let mut halo: Vec<(f64, f64)> = Vec::with_capacity(primary_pts.len() * 4);
    for &(x, y) in primary_pts {
        halo.push((x - dx, y));
        halo.push((x + dx, y));
        halo.push((x, y - dy));
        halo.push((x, y + dy));
    }
    ctx.draw(&Points {
        coords: &halo,
        color: glow_c,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catmull_rom_endpoints_match_input() {
        let pts = vec![(0.0, 0.0), (1.0, 2.0), (3.0, 1.0), (4.0, 0.0)];
        let d = catmull_rom(&pts, 8);
        assert!((d[0].0 - pts[0].0).abs() < 1e-3 && (d[0].1 - pts[0].1).abs() < 1e-3);
        let last = *d.last().unwrap();
        assert!((last.0 - pts[3].0).abs() < 1e-3 && (last.1 - pts[3].1).abs() < 1e-3);
    }

    #[test]
    fn gradient_color_endpoints() {
        let stops = vec!["#FF0000".into(), "#0000FF".into()];
        assert_eq!(gradient_color(&stops, 0.0), Color::Rgb(255, 0, 0));
        assert_eq!(gradient_color(&stops, 1.0), Color::Rgb(0, 0, 255));
    }
}
