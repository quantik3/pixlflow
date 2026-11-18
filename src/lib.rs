//! Pixlflow core: paths, span selectors, span algorithms and the render pipeline.

pub mod algo;
pub mod color;
pub mod engine;
mod gilbert;
pub mod paths;
pub mod select;
pub mod settings;

use rayon::prelude::*;

pub type Rgb = [u8; 3];

/// Small deterministic RNG (splitmix64), so results repeat for the same seed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x5DEE_CE66_D1CE_4E5B)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n` (0 when n == 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
    /// Uniform in `[0, 1)`.
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

pub fn lerp_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|c| (a[c] as f32 + (b[c] as f32 - a[c] as f32) * t).round().clamp(0.0, 255.0) as u8)
}

/// Box blur with radius `r`: every value becomes the mean of the window clipped to the image.
pub fn box_blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let mut rows = vec![0f32; w * h];
    rows.par_chunks_mut(w).zip(src.par_chunks(w)).for_each(|(o, s)| blur_line(s, o, r));
    let t = transpose(&rows, w, h);
    let mut cols = vec![0f32; w * h];
    cols.par_chunks_mut(h).zip(t.par_chunks(h)).for_each(|(o, s)| blur_line(s, o, r));
    transpose(&cols, h, w)
}

fn blur_line(src: &[f32], out: &mut [f32], r: usize) {
    let n = src.len();
    let mut sum: f64 = src[..=r.min(n - 1)].iter().map(|&v| v as f64).sum();
    for i in 0..n {
        let (lo, hi) = (i.saturating_sub(r), (i + r).min(n - 1));
        out[i] = (sum / (hi - lo + 1) as f64) as f32;
        if i + r + 1 < n {
            sum += src[i + r + 1] as f64;
        }
        if i >= r {
            sum -= src[i - r] as f64;
        }
    }
}

fn transpose(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            out[x * h + y] = src[y * w + x];
        }
    }
    out
}
