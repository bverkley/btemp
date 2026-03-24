//! Braille temperature chart: fixed-width history window, vertical btop-style heat coloring, and Y scaling from data.
//! Kept separate from the ratatui event loop so layout code stays easier to follow.

use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::symbols::Marker;
use ratatui::widgets::canvas::{Canvas, Line};
use ratatui::Frame;

fn interpolate_series(vals: &[f64], x: f64) -> f64 {
    if vals.is_empty() {
        return 20.0;
    }
    if vals.len() == 1 {
        return vals[0];
    }
    let xmax = (vals.len() - 1) as f64;
    let x = x.clamp(0.0, xmax);
    let i = x.floor() as usize;
    let j = (i + 1).min(vals.len() - 1);
    let t = x - i as f64;
    vals[i] * (1.0 - t) + vals[j] * t
}

/// Upper bound on the X axis in **sample index** space for a history of capacity `cap` (indices `0..cap-1`).
fn history_x_upper(history_cap: usize) -> f64 {
    (history_cap as f64 - 1.0).max(0.0)
}

/// btop-style placement: newest sample sits at the right edge of the window; shorter histories leave empty space on the left.
fn displayed_temp_at_x(vals: &[f64], history_cap: usize, x: f64) -> Option<f64> {
    let n = vals.len();
    if n == 0 {
        return None;
    }
    let base = history_cap.saturating_sub(n) as f64;
    if x + 1e-9 < base {
        return None;
    }
    let local_x = x - base;
    if local_x < -1e-9 {
        return None;
    }
    let local_max = (n.saturating_sub(1)) as f64;
    Some(interpolate_series(vals, local_x.clamp(0.0, local_max)))
}

pub(super) fn render_braille_temp_canvas(
    frame: &mut Frame<'_>,
    area: Rect,
    vals: &[f64],
    history_cap: usize,
) {
    if area.width < 2 || area.height < 2 {
        return;
    }

    let cap = history_cap.max(2);
    let x_upper = history_x_upper(cap);

    if vals.is_empty() {
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, x_upper.max(1.0)])
            .y_bounds([0.0, 1.0])
            .paint(|_| {});
        frame.render_widget(canvas, area);
        return;
    }

    let vals_vec: Vec<f64> = vals.to_vec();

    let mut dmin = vals_vec
        .iter()
        .cloned()
        .fold(f64::INFINITY, f64::min);
    let mut dmax = vals_vec
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    if !dmin.is_finite() || !dmax.is_finite() {
        dmin = 30.0;
        dmax = 50.0;
    }
    if dmax < dmin {
        std::mem::swap(&mut dmin, &mut dmax);
    }
    let span_data = (dmax - dmin).max(0.0);
    let pad = (span_data * 0.2).max(1.5);
    let mut y_bottom = dmin - pad;
    let mut y_top = dmax + pad;
    const MIN_VISIBLE_SPAN: f64 = 5.0;
    if y_top - y_bottom < MIN_VISIBLE_SPAN {
        let mid = (dmin + dmax) * 0.5;
        y_bottom = mid - MIN_VISIBLE_SPAN / 2.0;
        y_top = mid + MIN_VISIBLE_SPAN / 2.0;
    }
    let view_h = (y_top - y_bottom).max(1e-6);
    let chart_h = area.height.max(1);

    // ~6 samples per terminal column so braille columns touch like btop.
    let steps = (area.width as usize)
        .saturating_mul(6)
        .max(96)
        .max(cap.saturating_mul(8));

    let canvas = Canvas::default()
        .marker(Marker::Braille)
        // Fixed window in sample space: constant horizontal scale; right-aligned data (see `displayed_temp_at_x`).
        .x_bounds([0.0, x_upper.max(1.0)])
        .y_bounds([y_bottom, y_top])
        .paint(move |ctx| {
            for s in 0..=steps {
                let x = (s as f64 / steps as f64) * x_upper.max(1.0);
                let Some(v) = displayed_temp_at_x(&vals_vec, cap, x) else {
                    continue;
                };
                let fill_top = v.clamp(y_bottom, y_top);
                let fill_span = fill_top - y_bottom;
                if fill_span <= 1e-6 {
                    continue;
                }
                let n_seg = (((fill_span / view_h) * f64::from(chart_h) * 14.0).round() as usize)
                    .clamp(6, 56);
                for k in 0..n_seg {
                    let y0 = y_bottom + (k as f64 / n_seg as f64) * fill_span;
                    let y1 = y_bottom + ((k + 1) as f64 / n_seg as f64) * fill_span;
                    let y_mid = (y0 + y1) * 0.5;
                    let g = ((y_mid - y_bottom) / view_h).clamp(0.0, 1.0);
                    let col = btop_vertical_heat_color(g);
                    ctx.draw(&Line::new(x, y0, x, y1, col));
                }
            }
        });

    frame.render_widget(canvas, area);
}

fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    ((1.0 - t) * f64::from(a) + t * f64::from(b)).round() as u8
}

/// btop-style color from **vertical position** in the chart: bottom = cool/dark green, top = red.
/// `t` is normalized in [0, 1] from chart bottom to chart top (not from temperature value).
fn btop_vertical_heat_color(t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    const STOPS: [(f64, u8, u8, u8); 7] = [
        (0.00, 30, 82, 52),
        (0.18, 42, 118, 68),
        (0.35, 68, 175, 88),
        (0.50, 118, 205, 78),
        (0.62, 220, 205, 65),
        (0.78, 248, 155, 52),
        (1.00, 238, 58, 55),
    ];
    for i in 0..STOPS.len() - 1 {
        let (t0, r0, g0, b0) = STOPS[i];
        let (t1, r1, g1, b1) = STOPS[i + 1];
        let last = i == STOPS.len() - 2;
        if t >= t0 && (t <= t1 || last) {
            let u = ((t - t0) / (t1 - t0).max(1e-9)).clamp(0.0, 1.0);
            return Color::Rgb(
                lerp_u8(r0, r1, u),
                lerp_u8(g0, g1, u),
                lerp_u8(b0, b1, u),
            );
        }
    }
    let (_, r, g, b) = STOPS[STOPS.len() - 1];
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod history_x_tests {
    use super::*;

    #[test]
    fn history_x_upper_matches_sample_indices() {
        assert!((history_x_upper(160) - 159.0).abs() < 1e-9);
        assert!((history_x_upper(2) - 1.0).abs() < 1e-9);
        assert!((history_x_upper(1) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn displayed_temp_right_aligns_short_series() {
        let cap = 10usize;
        let vals = vec![10.0, 20.0, 30.0];
        assert!(displayed_temp_at_x(&vals, cap, 6.0).is_none());
        assert!(displayed_temp_at_x(&vals, cap, 6.9).is_none());
        assert!((displayed_temp_at_x(&vals, cap, 7.0).unwrap() - 10.0).abs() < 1e-9);
        assert!((displayed_temp_at_x(&vals, cap, 9.0).unwrap() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn displayed_temp_full_window_covers_all_x() {
        let cap = 5usize;
        let vals = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        for i in 0..5 {
            let x = i as f64;
            assert!((displayed_temp_at_x(&vals, cap, x).unwrap() - (i + 1) as f64).abs() < 1e-9);
        }
    }
}

#[cfg(test)]
mod gradient_tests {
    use super::*;

    #[test]
    fn vertical_heat_bottom_is_green_top_is_red_tint() {
        let c0 = btop_vertical_heat_color(0.0);
        let c1 = btop_vertical_heat_color(1.0);
        let Color::Rgb(r0, g0, _b0) = c0 else {
            panic!("expected Rgb");
        };
        let Color::Rgb(r1, g1, _b1) = c1 else {
            panic!("expected Rgb");
        };
        assert!(g0 > r0, "bottom should be green-dominant");
        assert!(r1 > g1, "top should be red-dominant");
    }
}
