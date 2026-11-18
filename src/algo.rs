//! Span algorithms (spec §5). `keys[i]` is the sort value of `px[i]` before processing.

use crate::{color, lerp_rgb, settings::{Algorithm as A, Settings}, Rgb, Rng};

pub fn apply(s: &Settings, px: &mut [Rgb], keys: &[u16], rng: &mut Rng) {
    let n = px.len();
    if n == 0 || (n == 1 && s.algo != A::Debug) {
        return;
    }
    match s.algo {
        A::Sort => permute(px, &stable_order(keys)),
        A::Comb => {
            let mut k = keys.to_vec();
            comb_sort(n, |i, j| {
                let out_of_order = k[i] > k[j];
                if out_of_order {
                    k.swap(i, j);
                    px.swap(i, j);
                }
                out_of_order
            });
        }
        A::Glitch => {
            // Deliberately broken: signed comparison, and swaps hit the records' stored positions.
            let mut rec: Vec<(i8, usize)> = keys.iter().map(|&k| k as u8 as i8).zip(0..).collect();
            comb_sort(n, |i, j| {
                let out_of_order = rec[i].0 > rec[j].0;
                if out_of_order {
                    px.swap(rec[i].1, rec[j].1);
                    rec.swap(i, j);
                }
                out_of_order
            });
        }
        A::Partial => {
            let mut k = keys.to_vec();
            for p in 0..(s.passes as usize).min(n) {
                for i in (p % 2..n - 1).step_by(2) {
                    if k[i] > k[i + 1] {
                        k.swap(i, i + 1);
                        px.swap(i, i + 1);
                    }
                }
            }
        }
        A::Stretch => {
            let m = ((1.0 - s.strength.clamp(0.0, 1.0)) * n as f32).round().max(1.0) as usize;
            let src = px.to_vec();
            for (i, p) in px.iter_mut().enumerate() {
                *p = src[i * m / n];
            }
        }
        A::Shift => {
            let k = rng.below((s.strength.clamp(0.0, 1.0) * n as f32) as u64 + 1) as usize;
            px.rotate_right(k % n);
        }
        A::Gradient => {
            let (a, b) = (px[0], px[n - 1]);
            for (i, p) in px.iter_mut().enumerate() {
                *p = lerp_rgb(a, b, i as f32 / (n - 1) as f32);
            }
        }
        A::Average => {
            let mut sum = [0u64; 3];
            px.iter().for_each(|p| (0..3).for_each(|c| sum[c] += p[c] as u64));
            let avg = sum.map(|v| ((v + n as u64 / 2) / n as u64) as u8);
            px.fill(avg);
        }
        A::Mirror => {
            let sorted: Vec<Rgb> = stable_order(keys).into_iter().map(|i| px[i]).collect();
            let evens = sorted.iter().step_by(2);
            let odds = sorted.iter().skip(1).step_by(2).rev();
            for (p, &v) in px.iter_mut().zip(evens.chain(odds)) {
                *p = v;
            }
        }
        A::Bucket => {
            let levels = s.buckets.clamp(2, 64);
            let span = color::max_value(s.by) as u32 + 1;
            let coarse: Vec<u16> = keys.iter().map(|&k| ((k as u32).min(span - 1) * levels / span) as u16).collect();
            permute(px, &stable_order(&coarse));
        }
        A::Shuffle => {
            for i in (1..n).rev() {
                px.swap(i, rng.below(i as u64 + 1) as usize);
            }
        }
        A::Debug => {
            let mut ch = || 80 + rng.below(161) as u8;
            px.fill([ch(), ch(), ch()]);
        }
    }
}

/// Indices of `keys` in stable ascending order; counting sort for long spans.
pub fn stable_order(keys: &[u16]) -> Vec<usize> {
    let n = keys.len();
    if n < 64 {
        let mut o: Vec<usize> = (0..n).collect();
        o.sort_by_key(|&i| keys[i]);
        return o;
    }
    let max = *keys.iter().max().unwrap() as usize;
    let mut start = vec![0usize; max + 2];
    keys.iter().for_each(|&k| start[k as usize + 1] += 1);
    for i in 1..start.len() {
        start[i] += start[i - 1];
    }
    let mut o = vec![0usize; n];
    for (i, &k) in keys.iter().enumerate() {
        o[start[k as usize]] = i;
        start[k as usize] += 1;
    }
    o
}

fn permute(px: &mut [Rgb], order: &[usize]) {
    let src = px.to_vec();
    for (p, &o) in px.iter_mut().zip(order) {
        *p = src[o];
    }
}

/// Comb sort driver (shrink 1.3). `step(i, j)` compares/swaps i < j and returns whether it swapped.
fn comb_sort(n: usize, mut step: impl FnMut(usize, usize) -> bool) {
    let mut gap = n;
    loop {
        gap = ((gap as f32 / 1.3) as usize).max(1);
        let mut swapped = false;
        for i in 0..n - gap {
            swapped |= step(i, i + gap);
        }
        if gap == 1 && !swapped {
            break;
        }
    }
}
