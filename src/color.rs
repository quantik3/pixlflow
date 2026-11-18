//! Per-pixel colour values (spec §2). All results are integers.

use crate::{settings::Criterion, Rgb};
use std::sync::LazyLock;

static LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let c = i as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    })
});

pub fn brightness(p: Rgb) -> u16 {
    ((2126 * p[0] as u32 + 7152 * p[1] as u32 + 722 * p[2] as u32) / 10000) as u16
}

/// Brightness with channels read as signed bytes — a deliberate bug used by the Glitch algorithm.
/// Returned as the raw byte of the signed result.
pub fn glitch_brightness(p: Rgb) -> u16 {
    let v = (2126 * p[0] as i8 as i32 + 7152 * p[1] as i8 as i32 + 722 * p[2] as i8 as i32) / 10000;
    v as i8 as u8 as u16
}

pub fn hue(p: Rgb) -> u16 {
    let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d == 0.0 {
        return 0;
    }
    let h = if max == r { (g - b) / d } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    ((h * 60.0).rem_euclid(360.0) as u16).min(359)
}

pub fn saturation(p: Rgb) -> u16 {
    let (max, min) = (max_ch(p) as u32, min_ch(p) as u32);
    if max == 0 { 0 } else { (255 * (max - min) / max) as u16 }
}

/// CIE L* scaled to 0–255.
pub fn lightness(p: Rgb) -> u16 {
    let y = 0.2126 * LINEAR[p[0] as usize] + 0.7152 * LINEAR[p[1] as usize] + 0.0722 * LINEAR[p[2] as usize];
    let f = if y > 216.0 / 24389.0 { y.cbrt() } else { (24389.0 / 27.0 * y + 16.0) / 116.0 };
    ((116.0 * f - 16.0) * 2.55).round().clamp(0.0, 255.0) as u16
}

fn max_ch(p: Rgb) -> u8 {
    p[0].max(p[1]).max(p[2])
}
fn min_ch(p: Rgb) -> u8 {
    p[0].min(p[1]).min(p[2])
}

/// Value of `c` for pixel `p`. `Guide` has no per-pixel colour meaning and falls back to brightness;
/// the engine substitutes the guide image where one exists.
pub fn value(c: Criterion, p: Rgb, target: Rgb) -> u16 {
    match c {
        Criterion::Brightness | Criterion::Guide => brightness(p),
        Criterion::Hue => hue(p),
        Criterion::Saturation => saturation(p),
        Criterion::Red => p[0] as u16,
        Criterion::Green => p[1] as u16,
        Criterion::Blue => p[2] as u16,
        Criterion::Warmth => ((p[0] as i32 - p[2] as i32 + 255) / 2) as u16,
        Criterion::Lightness => lightness(p),
        Criterion::Chroma => (max_ch(p) - min_ch(p)) as u16,
        Criterion::MinChannel => min_ch(p) as u16,
        Criterion::MaxChannel => max_ch(p) as u16,
        Criterion::ColorDistance => (0..3).map(|c| p[c].abs_diff(target[c]) as u16).sum::<u16>() / 3,
    }
}

/// Largest value a criterion can produce.
pub fn max_value(c: Criterion) -> u16 {
    if c == Criterion::Hue { 360 } else { 255 }
}
