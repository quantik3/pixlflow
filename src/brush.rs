//! Round brushes for masks and drawn directions (spec §7).

/// Changed area: x0, y0, x1, y1 (end exclusive).
pub type Area = [usize; 4];

/// Weight 0–1 at distance `d` from the center. Hardness 1 = sharp edge; lower = smoothstep fall-off.
pub fn falloff(d: f32, radius: f32, hardness: f32) -> f32 {
    let t = d / radius.max(0.5);
    let h = hardness.clamp(0.0, 1.0);
    if t >= 1.0 {
        0.0
    } else if t <= h {
        1.0
    } else {
        let u = (t - h) / (1.0 - h);
        1.0 - u * u * (3.0 - 2.0 * u)
    }
}

fn area(w: usize, h: usize, cx: f32, cy: f32, r: f32) -> Option<Area> {
    let clamp = |v: f32, max: usize| v.clamp(0.0, max as f32) as usize;
    let (x0, x1) = (clamp((cx - r).floor(), w), clamp((cx + r).ceil() + 1.0, w));
    let (y0, y1) = (clamp((cy - r).floor(), h), clamp((cy + r).ceil() + 1.0, h));
    (x0 < x1 && y0 < y1).then_some([x0, y0, x1, y1])
}

/// Paints only ever lighten the mask, erasing only darkens, so overlapping stamps don't build up.
pub fn stamp_mask(mask: &mut [u8], w: usize, h: usize, cx: f32, cy: f32, radius: f32, hardness: f32, erase: bool) -> Option<Area> {
    let a = area(w, h, cx, cy, radius)?;
    for y in a[1]..a[3] {
        for x in a[0]..a[2] {
            let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
            let v = (falloff(d, radius, hardness) * 255.0).round() as u8;
            let m = &mut mask[y * w + x];
            *m = if erase { (*m).min(255 - v) } else { (*m).max(v) };
        }
    }
    Some(a)
}

/// Hard round stamp writing `angle` (radians, y down), or clearing it with `None`.
pub fn stamp_dirs(dirs: &mut [f32], w: usize, h: usize, cx: f32, cy: f32, radius: f32, angle: Option<f32>) -> Option<Area> {
    let a = area(w, h, cx, cy, radius)?;
    for y in a[1]..a[3] {
        for x in a[0]..a[2] {
            if (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) <= radius {
                dirs[y * w + x] = angle.unwrap_or(f32::NAN);
            }
        }
    }
    Some(a)
}

/// Stamp positions from `a` (exclusive) to `b` (inclusive), about `step` apart.
pub fn stroke(a: (f32, f32), b: (f32, f32), step: f32) -> Vec<(f32, f32)> {
    let n = ((b.0 - a.0).hypot(b.1 - a.1) / step.max(0.5)).ceil().max(1.0) as usize;
    (1..=n).map(|i| {
        let t = i as f32 / n as f32;
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn painting_lightens_and_erasing_darkens() {
        let (w, h) = (20, 20);
        let mut m = vec![100u8; w * h];
        stamp_mask(&mut m, w, h, 10.0, 10.0, 5.0, 0.5, false);
        assert_eq!(m[10 * w + 10], 255);
        assert!(m.iter().all(|&v| v >= 100), "painting never darkens");
        assert_eq!(m[0], 100, "outside the brush untouched");
        stamp_mask(&mut m, w, h, 10.0, 10.0, 5.0, 1.0, true);
        assert_eq!(m[10 * w + 10], 0);
        assert!(stamp_mask(&mut m, w, h, -50.0, -50.0, 5.0, 1.0, false).is_none());
    }

    #[test]
    fn falloff_shape() {
        assert_eq!(falloff(0.0, 10.0, 0.0), 1.0);
        assert_eq!(falloff(10.0, 10.0, 0.5), 0.0);
        assert_eq!(falloff(4.0, 10.0, 0.5), 1.0);
        assert!(falloff(7.5, 10.0, 0.5) > 0.0 && falloff(7.5, 10.0, 0.5) < 1.0);
        assert_eq!(stroke((0.0, 0.0), (10.0, 0.0), 2.5).len(), 4);
    }
}
