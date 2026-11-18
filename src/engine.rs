//! The render pipeline: paths → spans → algorithm → extras (§6) → blend with the input (mode, opacity, mask).

use crate::{
    algo, box_blur, color, lerp_rgb, paths, select,
    settings::{Algorithm, BlendMode, Channel, Criterion, PathKind, Settings},
    Rgb, Rng,
};
use rayon::prelude::*;

/// Optional per-pixel inputs, all `w * h` long.
#[derive(Default, Clone, Copy)]
pub struct Extras<'a> {
    /// 0 = leave alone, 255 = full effect, in between = blend.
    pub mask: Option<&'a [u8]>,
    /// Brightness of the guide image, already resized to the image.
    pub guide: Option<&'a [u8]>,
    /// Drawn directions in radians, NaN where nothing is drawn.
    pub directions: Option<&'a [f32]>,
}

pub fn render(s: &Settings, w: usize, h: usize, src: &[Rgb], ex: &Extras) -> Vec<Rgb> {
    assert_eq!(src.len(), w * h, "image buffer does not match its size");
    let mask = ex.mask.filter(|m| m.len() == w * h);
    let guide = ex.guide.filter(|g| g.len() == w * h);
    let luma: Option<Vec<f32>> =
        (s.path == PathKind::Contours).then(|| src.iter().map(|&p| color::brightness(p) as f32).collect());
    let mut paths = paths::build(s, &paths::PathInput { w, h, luma: luma.as_deref(), directions: ex.directions });
    if let Some(m) = mask {
        // pixels outside the mask cut paths: nothing moves across them
        paths = paths
            .iter()
            .flat_map(|p| p.split(|&i| m[i as usize] == 0).filter(|s| !s.is_empty()).map(<[u32]>::to_vec))
            .collect();
    }
    let blurred = (s.smooth > 0 && select::looks_at_color(s.select)).then(|| blur_rgb(src, w, h, s.smooth as usize));

    let results: Vec<Option<Vec<Rgb>>> = paths
        .par_iter()
        .enumerate()
        .map(|(pi, path)| process_path(s, pi, path, src, blurred.as_deref(), guide))
        .collect();

    let mut out = src.to_vec();
    for (path, res) in paths.iter().zip(results) {
        if let Some(res) = res {
            path.iter().zip(res).for_each(|(&i, p)| out[i as usize] = p);
        }
    }
    if s.fringes != 0 {
        fringes(&mut out, w, h, s.fringes as i64, mask);
    }
    // result = below + (blend(below, sorted) − below) · opacity · mask
    let opacity = s.opacity.clamp(0.0, 1.0);
    if s.blend != BlendMode::Normal || opacity < 1.0 || mask.is_some() {
        out.par_iter_mut().enumerate().for_each(|(i, o)| {
            let k = mask.map_or(1.0, |m| m[i] as f32 / 255.0) * opacity;
            *o = if k == 0.0 { src[i] } else { lerp_rgb(src[i], blend(s.blend, src[i], *o), k) };
        });
    }
    out
}

/// Blends `top` over `lower` (channels as 0–1).
pub fn blend(mode: BlendMode, lower: Rgb, top: Rgb) -> Rgb {
    let (a, b) = (lower.map(|c| c as f32 / 255.0), top.map(|c| c as f32 / 255.0));
    let r = match mode {
        BlendMode::Color => set_lum(b, lum(a)),
        BlendMode::Luminosity => set_lum(a, lum(b)),
        _ => std::array::from_fn(|c| {
            let (x, y) = (a[c], b[c]);
            match mode {
                BlendMode::Lighten => x.max(y),
                BlendMode::Darken => x.min(y),
                BlendMode::Multiply => x * y,
                BlendMode::Screen => 1.0 - (1.0 - x) * (1.0 - y),
                BlendMode::Overlay if x < 0.5 => 2.0 * x * y,
                BlendMode::Overlay => 1.0 - 2.0 * (1.0 - x) * (1.0 - y),
                BlendMode::Difference => (x - y).abs(),
                BlendMode::Add => (x + y).min(1.0),
                _ => y, // Normal
            }
        }),
    };
    r.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// Gives `c` luminance `l`; a colour pushed outside 0–1 is desaturated toward its luminance.
fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let c = c.map(|v| v + l - lum(c));
    let l = lum(c);
    let (n, x) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
    let mut c = c;
    if n < 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    c
}

/// White where a Threshold/Combined selector picks the pixel (everything white for other selectors).
pub fn selection_map(s: &Settings, w: usize, h: usize, src: &[Rgb]) -> Vec<u8> {
    let blurred = (s.smooth > 0).then(|| blur_rgb(src, w, h, s.smooth as usize));
    blurred
        .as_deref()
        .unwrap_or(src)
        .par_iter()
        .map(|&p| if select::pixel_selected(s, p) == Some(false) { 0 } else { 255 })
        .collect()
}

pub fn blur_rgb(src: &[Rgb], w: usize, h: usize, r: usize) -> Vec<Rgb> {
    let planes: Vec<Vec<f32>> =
        (0..3).map(|c| box_blur(&src.iter().map(|p| p[c] as f32).collect::<Vec<_>>(), w, h, r)).collect();
    (0..w * h).map(|i| std::array::from_fn(|c| planes[c][i].round().clamp(0.0, 255.0) as u8)).collect()
}

fn process_path(
    s: &Settings,
    pi: usize,
    path: &[u32],
    src: &[Rgb],
    blurred: Option<&[Rgb]>,
    guide: Option<&[u8]>,
) -> Option<Vec<Rgb>> {
    let orig: Vec<Rgb> = path.iter().map(|&i| src[i as usize]).collect();
    let mut rng = Rng::new(s.seed ^ (pi as u64 + 1).wrapping_mul(0x2545_F491_4F6C_DD1D));
    let mut spans = Vec::new();
    match blurred {
        Some(b) => select::spans(s, &path.iter().map(|&i| b[i as usize]).collect::<Vec<_>>(), &mut rng, &mut spans),
        None => select::spans(s, &orig, &mut rng, &mut spans),
    }
    if spans.is_empty() {
        return None;
    }
    let glitch_brightness = s.algo == Algorithm::Glitch && s.by == Criterion::Brightness;
    let keys: Vec<u16> = path
        .iter()
        .zip(&orig)
        .map(|(&i, &p)| match (s.by, guide) {
            (Criterion::Guide, Some(g)) => g[i as usize] as u16,
            _ if glitch_brightness => color::glitch_brightness(p),
            (c, _) => color::value(c, p, s.target),
        })
        .collect();

    let mut buf = orig.clone();
    let soft = s.soft_ends as usize;
    for (k, r) in spans.into_iter().enumerate() {
        algo::apply(s, &mut buf[r.clone()], &keys[r.clone()], &mut rng);
        if s.alternate && k % 2 == 1 {
            buf[r.clone()].reverse();
        }
        let len = r.len();
        for (j, i) in r.enumerate() {
            let (o, p) = (orig[i], &mut buf[i]);
            match s.channel {
                Channel::All => {}
                Channel::Red => *p = [p[0], o[1], o[2]],
                Channel::Green => *p = [o[0], p[1], o[2]],
                Channel::Blue => *p = [o[0], o[1], p[2]],
            }
            if s.posterize >= 2 {
                let l = (s.posterize - 1) as f32;
                *p = p.map(|c| ((c as f32 * l / 255.0).round() * 255.0 / l).round() as u8);
            }
            let d = j.min(len - 1 - j);
            if d < soft {
                *p = lerp_rgb(o, *p, (d + 1) as f32 / (soft + 1) as f32);
            }
        }
    }
    Some(buf)
}

/// Chromatic aberration: red moves right by `f` px, blue left. Skipped outside the mask.
fn fringes(out: &mut [Rgb], w: usize, h: usize, f: i64, mask: Option<&[u8]>) {
    let res = out.to_vec();
    let max = w as i64 - 1;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if mask.is_some_and(|m| m[i] == 0) {
                continue;
            }
            let xr = (x as i64 - f).clamp(0, max) as usize;
            let xb = (x as i64 + f).clamp(0, max) as usize;
            out[i][0] = res[y * w + xr][0];
            out[i][2] = res[y * w + xb][2];
        }
    }
}
