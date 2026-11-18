//! Span selectors (spec §4): which pieces of a path get processed.

use crate::{color, settings::{Combine, SelectorKind as K, Settings}, Rgb, Rng};
use std::ops::Range;

/// `min > max` wraps around: value ≥ min or ≤ max.
pub fn in_range(v: u16, min: u32, max: u32) -> bool {
    let v = v as u32;
    if min <= max { min <= v && v <= max } else { v >= min || v <= max }
}

/// Per-pixel test of Threshold/Combined selectors; `None` for selectors that don't judge single pixels.
pub fn pixel_selected(s: &Settings, p: Rgb) -> Option<bool> {
    let a = || in_range(color::value(s.crit, p, s.target), s.min, s.max);
    let b = || in_range(color::value(s.crit2, p, s.target), s.min2, s.max2);
    match s.select {
        K::Threshold => Some(a()),
        K::Combined => Some(match s.combine {
            Combine::And => a() && b(),
            Combine::Or => a() || b(),
        }),
        _ => None,
    }
}

/// Selectors that look at colours (and so use the smoothed copy when `smooth` > 0).
pub fn looks_at_color(k: K) -> bool {
    matches!(k, K::Threshold | K::Edges | K::Combined)
}

/// Writes the spans of a path with colours `px` into `out`: ordered, non-overlapping, inside the path.
pub fn spans(s: &Settings, px: &[Rgb], rng: &mut Rng, out: &mut Vec<Range<usize>>) {
    out.clear();
    let n = px.len();
    if n == 0 {
        return;
    }
    match s.select {
        K::Full => out.push(0..n),
        K::Fixed => {
            if s.length > 0 {
                let l = s.length as usize;
                out.extend((0..n).step_by(l).map(|i| i..(i + l).min(n)));
            }
        }
        K::Random => {
            if s.max_length > 1 {
                let mut i = 0;
                while i < n {
                    let l = 1 + rng.below(s.max_length as u64 - 1) as usize;
                    out.push(i..(i + l).min(n));
                    i += l;
                }
            }
        }
        K::Edges => {
            let mut start = 0;
            for i in 1..n {
                let d: u32 = (0..3).map(|c| px[i][c].abs_diff(px[i - 1][c]) as u32).sum();
                if d > s.edge {
                    out.push(start..i);
                    start = i;
                }
            }
            out.push(start..n);
        }
        K::Threshold | K::Combined => {
            let mut start = None;
            for (i, &p) in px.iter().enumerate() {
                match (pixel_selected(s, p) == Some(true), start) {
                    (true, None) => start = Some(i),
                    (false, Some(b)) => {
                        out.push(b..i);
                        start = None;
                    }
                    _ => {}
                }
            }
            if let Some(b) = start {
                out.push(b..n);
            }
        }
    }
}
