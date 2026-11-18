//! Path generators (spec §3). A path is a list of pixel indices `y * w + x`.
//! Every pixel belongs to at most one path: a pixel produced again later is skipped.

use crate::{box_blur, gilbert, settings::{PathKind as P, Settings}, Rng};
use rayon::prelude::*;
use std::collections::VecDeque;
use std::f64::consts::PI;

pub struct PathInput<'a> {
    pub w: usize,
    pub h: usize,
    /// Brightness per pixel; needed by `Contours`.
    pub luma: Option<&'a [f32]>,
    /// Drawn direction per pixel in radians (image axes, y down); NaN = nothing drawn.
    pub directions: Option<&'a [f32]>,
}

pub fn build(s: &Settings, inp: &PathInput) -> Vec<Vec<u32>> {
    let (w, h) = (inp.w, inp.h);
    let mut paths = if w == 0 || h == 0 {
        Vec::new()
    } else {
        match s.path {
            P::Diagonal => diagonal(w, h, s.angle as f64),
            P::Flow | P::Curl | P::Swirls | P::Contours | P::Drawn => field_paths(s, inp),
            _ => {
                let mut b = Builder::new(w, h);
                shapes(s, &mut b);
                b.end();
                b.paths
            }
        }
    };
    if s.reverse {
        paths.iter_mut().for_each(|p| p.reverse());
    }
    paths
}

/// Collects candidate points: drops points outside the image and pixels already taken.
struct Builder {
    w: i64,
    h: i64,
    used: Vec<bool>,
    cur: Vec<u32>,
    paths: Vec<Vec<u32>>,
}

impl Builder {
    fn new(w: usize, h: usize) -> Self {
        Builder { w: w as i64, h: h as i64, used: vec![false; w * h], cur: Vec::new(), paths: Vec::new() }
    }
    fn push(&mut self, x: i64, y: i64) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            let i = (y * self.w + x) as usize;
            if !self.used[i] {
                self.used[i] = true;
                self.cur.push(i as u32);
            }
        }
    }
    fn pushf(&mut self, x: f64, y: f64) {
        if x.is_finite() && y.is_finite() {
            self.push(x.floor() as i64, y.floor() as i64);
        }
    }
    fn end(&mut self) {
        if !self.cur.is_empty() {
            self.paths.push(std::mem::take(&mut self.cur));
        }
    }
}

fn shapes(s: &Settings, b: &mut Builder) {
    let (w, h) = (b.w, b.h);
    let (cx, cy) = (s.cx as f64 * w as f64, s.cy as f64 * h as f64);
    let corner = [(0, 0), (w, 0), (0, h), (w, h)]
        .iter()
        .map(|&(x, y)| (x as f64 - cx).hypot(y as f64 - cy))
        .fold(0.0, f64::max);
    match s.path {
        P::Lines | P::AllHorizontal => {
            for y in 0..h {
                (0..w).for_each(|x| b.push(x, y));
                if s.path == P::Lines {
                    b.end();
                }
            }
        }
        P::Columns | P::AllVertical => {
            for x in 0..w {
                (0..h).for_each(|y| b.push(x, y));
                if s.path == P::Columns {
                    b.end();
                }
            }
        }
        P::Snake => {
            for y in 0..h {
                (0..w).for_each(|i| b.push(if y % 2 == 0 { i } else { w - 1 - i }, y));
            }
        }
        P::Tiles => {
            let t = s.tile.max(1) as i64;
            for ty in (0..h).step_by(t as usize) {
                for tx in (0..w).step_by(t as usize) {
                    for y in ty..(ty + t).min(h) {
                        (tx..(tx + t).min(w)).for_each(|x| b.push(x, y));
                    }
                    b.end();
                }
            }
        }
        P::Hilbert => gilbert::curve(w, h, &mut |x, y| b.push(x, y)),
        P::Rays => rays(b, cx, cy),
        P::Circles | P::Spiral => ellipses(b, s, cx, cy, corner),
        P::SquareSpiral => spiral(b, cx, cy, 0, true),
        P::RectSpiral => spiral(b, cx, cy, (w - h).abs(), w >= h),
        P::Waves => waves(b, s),
        P::Star => star(b, s, cx, cy, corner),
        P::Diagonal | P::Flow | P::Curl | P::Swirls | P::Contours | P::Drawn => unreachable!(),
    }
}

/// Bresenham line, both ends included.
fn line((x0, y0): (i64, i64), (x1, y1): (i64, i64), mut f: impl FnMut(i64, i64)) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (x0, y0, dx + dy);
    loop {
        f(x, y);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Rays from the center to every border pixel, "middle first" on each side, breadth-first
/// across all four sides so early rays stay whole.
fn rays(b: &mut Builder, cx: f64, cy: f64) {
    let (w, h) = (b.w, b.h);
    let c = ((cx.floor() as i64).clamp(0, w - 1), (cy.floor() as i64).clamp(0, h - 1));
    let side_len = |k: usize| if k % 2 == 0 { w } else { h };
    let side = |k: usize, i: i64| match k {
        0 => (i, 0),
        1 => (w - 1, i),
        2 => (w - 1 - i, h - 1),
        _ => (0, h - 1 - i),
    };
    let mut q: VecDeque<(usize, i64, i64)> = (0..4).map(|k| (k, 0, side_len(k))).collect();
    while let Some((k, lo, hi)) = q.pop_front() {
        if lo >= hi {
            continue;
        }
        let mid = lo + (hi - lo) / 2;
        line(c, side(k, mid), |x, y| b.push(x, y));
        b.end();
        q.push_back((k, lo, mid));
        q.push_back((k, mid + 1, hi));
    }
}

/// Concentric ellipses (Circles: two half-paths per ring) or all rings joined (Spiral).
fn ellipses(b: &mut Builder, s: &Settings, cx: f64, cy: f64, corner: f64) {
    let a = (s.aspect as f64).clamp(0.2, 5.0);
    let (ax, ay) = (a.sqrt(), 1.0 / a.sqrt());
    let (sn, cs) = (s.rotation as f64).to_radians().sin_cos();
    let rmax = (corner / ax.min(ay)).ceil() as i64 + 1;
    // t = 0 is the top; growing t goes clockwise on screen
    let pt = |b: &mut Builder, r: f64, t: f64| {
        let (lx, ly) = (t.sin() * r * ax, -t.cos() * r * ay);
        b.pushf(cx + lx * cs - ly * sn, cy + lx * sn + ly * cs);
    };
    for r in 1..=rmax {
        let rf = r as f64;
        if s.path == P::Spiral {
            let n = 16 * r;
            (0..n).for_each(|i| pt(b, rf, 2.0 * PI * i as f64 / n as f64));
        } else {
            let n = 8 * r;
            for side in [-1.0, 1.0] {
                (0..=n).for_each(|i| pt(b, rf, side * PI * i as f64 / n as f64));
                b.end();
            }
        }
    }
}

/// Rectangular spiral from the center. Legs along the long axis are `extra` longer than the
/// legs across it; `extra = 0` gives the square spiral.
fn spiral(b: &mut Builder, cx: f64, cy: f64, extra: i64, long_horizontal: bool) {
    let dirs: [(i64, i64); 4] =
        if long_horizontal { [(1, 0), (0, 1), (-1, 0), (0, -1)] } else { [(0, 1), (-1, 0), (0, -1), (1, 0)] };
    let (mut x, mut y) = (cx.floor() as i64, cy.floor() as i64);
    if long_horizontal { x -= extra / 2 } else { y -= extra / 2 }
    b.push(x, y);
    let limit = 2 * b.w.max(b.h) + 2;
    for leg in 0.. {
        let n = leg / 2 + 1;
        if n > limit {
            break;
        }
        let (dx, dy) = dirs[(leg % 4) as usize];
        for _ in 0..n + if leg % 2 == 0 { extra } else { 0 } {
            x += dx;
            y += dy;
            b.push(x, y);
        }
    }
}

/// Parallel sine-shaped lines; every line has the same shape, so they never cross.
fn waves(b: &mut Builder, s: &Settings) {
    let amp = (s.amplitude as f64).abs();
    let lambda = (s.wavelength as f64).max(1.0);
    let vertical = s.wave_vertical;
    let (len, across) = if vertical { (b.h, b.w) } else { (b.w, b.h) };
    let off: Vec<i64> = (0..len).map(|t| (amp * (2.0 * PI * t as f64 / lambda).sin()).round() as i64).collect();
    let a = amp.ceil() as i64;
    for base in -a..across + a {
        for t in 0..len {
            let o = base + off[t as usize];
            if vertical { b.push(o, t) } else { b.push(t, o) }
        }
        b.end();
    }
}

/// Nested star outlines, starting at the top and going clockwise.
fn star(b: &mut Builder, s: &Settings, cx: f64, cy: f64, corner: f64) {
    let n = s.star_points.clamp(2, 1000) as usize;
    let k = (s.star_inner as f64).clamp(0.0, 1.0);
    let verts: Vec<(f64, f64)> = (0..=2 * n)
        .map(|j| {
            let a = PI * j as f64 / n as f64;
            let r = if j % 2 == 0 { 1.0 } else { k };
            (r * a.sin(), -r * a.cos())
        })
        .collect();
    // shortcut: below 5% of the outer radius the star stops growing to cover corners; fine for a visual effect
    let min_frac = verts.windows(2).map(|e| seg_dist(e[0], e[1])).fold(1.0, f64::min).max(0.05);
    let rmax = (corner / min_frac).ceil() as i64 + 1;
    let (w, h) = (b.w as f64, b.h as f64);
    for r in 1..=rmax {
        let rf = r as f64;
        for e in verts.windows(2) {
            let (ax, ay) = (cx + e[0].0 * rf, cy + e[0].1 * rf);
            let (bx, by) = (cx + e[1].0 * rf, cy + e[1].1 * rf);
            let Some((t0, t1)) = clip(ax, ay, bx, by, -1.0, -1.0, w + 1.0, h + 1.0) else { continue };
            let steps = ((bx - ax).hypot(by - ay) * (t1 - t0) * 2.0).ceil().max(1.0) as usize;
            for i in 0..=steps {
                let t = t0 + (t1 - t0) * i as f64 / steps as f64;
                b.pushf(ax + (bx - ax) * t, ay + (by - ay) * t);
            }
        }
        b.end();
    }
}

/// Distance from the origin to segment a–b.
fn seg_dist((ax, ay): (f64, f64), (bx, by): (f64, f64)) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (-(ax * dx + ay * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (ax + dx * t).hypot(ay + dy * t)
}

/// Liang–Barsky: parameter range of segment a–b inside the box, if any.
#[allow(clippy::too_many_arguments)]
fn clip(ax: f64, ay: f64, bx: f64, by: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> Option<(f64, f64)> {
    let (dx, dy) = (bx - ax, by - ay);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, ax - x0), (dx, x1 - ax), (-dy, ay - y0), (dy, y1 - ay)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 { t0 = t0.max(t) } else { t1 = t1.min(t) }
        }
    }
    (t0 <= t1).then_some((t0, t1))
}

/// Parallel straight lines at `angle` degrees (0 = right, 90 = up). Each pixel goes to the line
/// nearest its center; lines are walked along the angle direction.
fn diagonal(w: usize, h: usize, angle: f64) -> Vec<Vec<u32>> {
    let a = angle.to_radians();
    let (dx, dy) = (a.cos(), -a.sin());
    let (nx, ny) = (-dy, dx);
    let key = |x: f64, y: f64| (x * nx + y * ny).round() as i64;
    let (fw, fh) = (w as f64 - 0.5, h as f64 - 0.5);
    let ks = [key(0.5, 0.5), key(fw, 0.5), key(0.5, fh), key(fw, fh)];
    let kmin = *ks.iter().min().unwrap();
    let kmax = *ks.iter().max().unwrap();
    let mut lines = vec![Vec::new(); (kmax - kmin + 1) as usize];
    for y in 0..h {
        for x in 0..w {
            lines[(key(x as f64 + 0.5, y as f64 + 0.5) - kmin) as usize].push((y * w + x) as u32);
        }
    }
    let along = |i: u32| ((i as usize % w) as f64 + 0.5) * dx + ((i as usize / w) as f64 + 0.5) * dy;
    lines.par_iter_mut().for_each(|l| l.sort_by(|&i, &j| along(i).total_cmp(&along(j))));
    lines.retain(|l| !l.is_empty());
    lines
}

// ---- direction fields ----

fn field_paths(s: &Settings, inp: &PathInput) -> Vec<Vec<u32>> {
    let (w, h) = (inp.w, inp.h);
    let size = s.noise_size.max(1.0);
    let seed = s.seed;
    let (dirs, unsigned) = match s.path {
        P::Flow => (
            par_map(w, h, |x, y| {
                let a = fbm(x, y, size, seed) * 4.0 * std::f32::consts::PI;
                (a.cos(), a.sin())
            }),
            false,
        ),
        P::Curl => (
            par_map(w, h, |x, y| {
                let gx = fbm(x + 1.0, y, size, seed) - fbm(x - 1.0, y, size, seed);
                let gy = fbm(x, y + 1.0, size, seed) - fbm(x, y - 1.0, size, seed);
                (gy, -gx)
            }),
            false,
        ),
        P::Swirls => (swirl_field(s, w, h), false),
        P::Contours => (contour_field(inp.luma, w, h, s.contour_smooth as usize), true),
        _ => match drawn_field(inp.directions, w, h) {
            Some(d) => (d, false),
            None => {
                let mut b = Builder::new(w, h);
                for x in 0..w as i64 {
                    (0..h as i64).for_each(|y| b.push(x, y));
                    b.end();
                }
                return b.paths;
            }
        },
    };
    trace(w, h, &dirs, unsigned)
}

/// Evaluates `f` at every pixel center.
fn par_map(w: usize, h: usize, f: impl Fn(f32, f32) -> (f32, f32) + Sync) -> Vec<(f32, f32)> {
    (0..w * h).into_par_iter().map(|i| f((i % w) as f32 + 0.5, (i / w) as f32 + 0.5)).collect()
}

fn hash2(x: i64, y: i64, seed: u64) -> f32 {
    Rng::new(seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .f32()
}

fn value_noise(x: f32, y: f32, seed: u64) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (ix, iy) = (x0 as i64, y0 as i64);
    let v = |dx, dy| hash2(ix + dx, iy + dy, seed);
    let top = v(0, 0) + (v(1, 0) - v(0, 0)) * sx;
    let bottom = v(0, 1) + (v(1, 1) - v(0, 1)) * sx;
    top + (bottom - top) * sy
}

/// Smooth value noise in [0, 1): 3 octaves, amplitude ×0.5 and frequency ×2 per octave.
pub fn fbm(x: f32, y: f32, size: f32, seed: u64) -> f32 {
    let (mut f, mut amp, mut sum, mut norm) = (1.0 / size, 1.0, 0.0, 0.0);
    for o in 0..3u64 {
        sum += amp * value_noise(x * f, y * f, seed.wrapping_add(o));
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// Vortices around the center and `swirls - 1` random points; even/odd centers spin opposite ways.
fn swirl_field(s: &Settings, w: usize, h: usize) -> Vec<(f32, f32)> {
    let mut rng = Rng::new(s.seed);
    let (fw, fh) = (w as f32, h as f32);
    let centers: Vec<(f32, f32)> = (0..s.swirls.clamp(1, 256))
        .map(|i| if i == 0 { (s.cx * fw, s.cy * fh) } else { (rng.f32() * fw, rng.f32() * fh) })
        .collect();
    let soft = (0.05 * fw.min(fh)).powi(2).max(1.0);
    par_map(w, h, |x, y| {
        let (mut vx, mut vy) = (0.0, 0.0);
        for (i, &(px, py)) in centers.iter().enumerate() {
            let (dx, dy) = (x - px, y - py);
            let d2 = dx * dx + dy * dy;
            let d = d2.sqrt().max(1e-6);
            let f = if i % 2 == 0 { 1.0 } else { -1.0 } / (d2 + soft);
            vx -= dy / d * f;
            vy += dx / d * f;
        }
        (vx, vy)
    })
}

/// Along image contours: structure tensor of the brightness gradient, box-blurred, turned 90°.
fn contour_field(luma: Option<&[f32]>, w: usize, h: usize, r: usize) -> Vec<(f32, f32)> {
    let Some(l) = luma else { return vec![(0.0, 1.0); w * h] };
    let (mut jxx, mut jxy, mut jyy) = (vec![0f32; w * h], vec![0f32; w * h], vec![0f32; w * h]);
    for y in 0..h {
        for x in 0..w {
            let gx = l[y * w + (x + 1).min(w - 1)] - l[y * w + x.saturating_sub(1)];
            let gy = l[(y + 1).min(h - 1) * w + x] - l[y.saturating_sub(1) * w + x];
            let i = y * w + x;
            (jxx[i], jxy[i], jyy[i]) = (gx * gx, gx * gy, gy * gy);
        }
    }
    let (jxx, jxy, jyy) = (box_blur(&jxx, w, h, r), box_blur(&jxy, w, h, r), box_blur(&jyy, w, h, r));
    (0..w * h)
        .map(|i| {
            let t = 0.5 * (2.0 * jxy[i]).atan2(jxx[i] - jyy[i]) + std::f32::consts::FRAC_PI_2;
            (t.cos(), t.sin())
        })
        .collect()
}

const CELL: usize = 16;

/// Drawn directions, smoothly filled in between strokes via a coarse grid. `None` if nothing drawn.
fn drawn_field(dirs: Option<&[f32]>, w: usize, h: usize) -> Option<Vec<(f32, f32)>> {
    let d = dirs.filter(|d| d.len() == w * h && d.iter().any(|a| !a.is_nan()))?;
    let (gw, gh) = (w.div_ceil(CELL), h.div_ceil(CELL));
    let mut grid = vec![(0f32, 0f32); gw * gh];
    let mut filled = vec![false; gw * gh];
    for (i, &a) in d.iter().enumerate() {
        if !a.is_nan() {
            let c = (i / w / CELL) * gw + (i % w) / CELL;
            grid[c].0 += a.cos();
            grid[c].1 += a.sin();
            filled[c] = true;
        }
    }
    let unit = |(x, y): (f32, f32)| {
        let l = x.hypot(y);
        if l > 1e-9 { (x / l, y / l) } else { (0.0, 0.0) }
    };
    // grow filled cells into empty neighbours until the grid is full
    while filled.iter().any(|f| !f) {
        let (g0, f0) = (grid.clone(), filled.clone());
        for cy in 0..gh {
            for cx in 0..gw {
                let c = cy * gw + cx;
                if f0[c] {
                    continue;
                }
                let mut acc = (0.0, 0.0);
                for (nx, ny) in [(cx.wrapping_sub(1), cy), (cx + 1, cy), (cx, cy.wrapping_sub(1)), (cx, cy + 1)] {
                    if nx < gw && ny < gh && f0[ny * gw + nx] {
                        let v = unit(g0[ny * gw + nx]);
                        acc = (acc.0 + v.0, acc.1 + v.1);
                        filled[c] = true;
                    }
                }
                grid[c] = acc;
            }
        }
    }
    let grid: Vec<(f32, f32)> = grid.into_iter().map(unit).collect();
    Some(par_map(w, h, |x, y| {
        let a = d[y as usize * w + x as usize];
        if !a.is_nan() {
            return (a.cos(), a.sin());
        }
        let gx = (x / CELL as f32 - 0.5).clamp(0.0, (gw - 1) as f32);
        let gy = (y / CELL as f32 - 0.5).clamp(0.0, (gh - 1) as f32);
        let (x0, y0) = (gx as usize, gy as usize);
        let (x1, y1) = ((x0 + 1).min(gw - 1), (y0 + 1).min(gh - 1));
        let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
        let g = |cx: usize, cy: usize| grid[cy * gw + cx];
        let mix = |a: (f32, f32), b: (f32, f32), t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        mix(mix(g(x0, y0), g(x1, y0), fx), mix(g(x0, y1), g(x1, y1), fx), fy)
    }))
}

const MAX_LINE: usize = 4096;

/// Traces a line through every still-free pixel, forward and backward along the field.
fn trace(w: usize, h: usize, dirs: &[(f32, f32)], unsigned: bool) -> Vec<Vec<u32>> {
    let mut used = vec![false; w * h];
    let mut paths = Vec::new();
    for start in 0..w * h {
        if used[start] {
            continue;
        }
        used[start] = true;
        let fwd = walk(w, h, dirs, unsigned, &mut used, start, 1.0, MAX_LINE / 2);
        let back = walk(w, h, dirs, unsigned, &mut used, start, -1.0, MAX_LINE - 1 - fwd.len());
        let mut line: Vec<u32> = back.into_iter().rev().collect();
        line.push(start as u32);
        line.extend(fwd);
        paths.push(line);
    }
    paths
}

#[allow(clippy::too_many_arguments)]
fn walk(
    w: usize,
    h: usize,
    dirs: &[(f32, f32)],
    unsigned: bool,
    used: &mut [bool],
    start: usize,
    sign: f32,
    budget: usize,
) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut x, mut y) = ((start % w) as f32 + 0.5, (start / w) as f32 + 0.5);
    let (mut cell, mut prev, mut stuck) = (start, None::<(f32, f32)>, 0);
    while out.len() < budget {
        let (dx, dy) = dirs[cell];
        let len = dx.hypot(dy);
        if !(len > 1e-12) {
            break;
        }
        let (mut dx, mut dy) = (dx / len * sign, dy / len * sign);
        if let (true, Some((px, py))) = (unsigned, prev) {
            if dx * px + dy * py < 0.0 {
                (dx, dy) = (-dx, -dy);
            }
        }
        prev = Some((dx, dy));
        x += dx;
        y += dy;
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
            break;
        }
        let c = y as usize * w + x as usize;
        if c == cell {
            stuck += 1;
            if stuck > 3 {
                break;
            }
            continue;
        }
        stuck = 0;
        if used[c] {
            break;
        }
        used[c] = true;
        out.push(c as u32);
        cell = c;
    }
    out
}
