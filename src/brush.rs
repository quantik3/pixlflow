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

/// Corners of the rectangle spanned by `a` and `b`, as a polygon for `fill_shape`.
pub fn rect_points(a: (f32, f32), b: (f32, f32)) -> [(f32, f32); 4] {
    [a, (b.0, a.1), b, (a.0, b.1)]
}

/// Fills a polygon (image pixels; pixel centers inside, even-odd) into the mask with an edge
/// softened over about `feather` px. Like the brush, painting only lightens and erasing only darkens.
pub fn fill_shape(mask: &mut [u8], w: usize, h: usize, pts: &[(f32, f32)], feather: f32, erase: bool) -> Option<Area> {
    if pts.len() < 3 {
        return None;
    }
    let r = (feather.max(0.0) / 2.0).round() as usize; // box blur radius: the edge ramp is 2r+1 px wide
    let pad = 2.0 * r as f32 + 1.0; // room for the ramp, so the blur window is never cut short by the crop
    let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
    for p in pts {
        (lo, hi) = ((lo.0.min(p.0), lo.1.min(p.1)), (hi.0.max(p.0), hi.1.max(p.1)));
    }
    let clamp = |v: f32, max: usize| v.clamp(0.0, max as f32) as usize;
    let (x0, x1) = (clamp((lo.0 - pad).floor(), w), clamp((hi.0 + pad).ceil(), w));
    let (y0, y1) = (clamp((lo.1 - pad).floor(), h), clamp((hi.1 + pad).ceil(), h));
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let bw = x1 - x0;
    let mut cover = vec![0f32; bw * (y1 - y0)];
    let mut xs = Vec::new();
    for y in y0..y1 {
        let cy = y as f32 + 0.5;
        xs.clear();
        for (i, a) in pts.iter().enumerate() {
            let b = pts[(i + 1) % pts.len()];
            if (a.1 <= cy) != (b.1 <= cy) {
                xs.push(a.0 + (cy - a.1) / (b.1 - a.1) * (b.0 - a.0));
            }
        }
        xs.sort_by(f32::total_cmp);
        let row = &mut cover[(y - y0) * bw..][..bw];
        for span in xs.chunks_exact(2) {
            // pixels x with span[0] <= x + 0.5 < span[1]
            let (s, e) = (clamp((span[0] - 0.5).ceil(), w).max(x0), clamp((span[1] - 0.5).ceil(), w).min(x1));
            if s < e {
                row[s - x0..e - x0].fill(1.0);
            }
        }
    }
    let cover = crate::box_blur(&cover, bw, y1 - y0, r);
    for y in y0..y1 {
        for x in x0..x1 {
            let v = (cover[(y - y0) * bw + x - x0] * 255.0).round() as u8;
            let m = &mut mask[y * w + x];
            *m = if erase { (*m).min(255 - v) } else { (*m).max(v) };
        }
    }
    Some([x0, y0, x1, y1])
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
    fn shape_fill() {
        let (w, h) = (10, 8);
        let mut m = vec![0u8; w * h];
        // rectangle with corners in any order, clipped to the image: pixel centers 7.5..9.5 × 0.5..5.5
        assert!(fill_shape(&mut m, w, h, &rect_points((12.0, 6.0), (7.0, -3.0)), 0.0, false).is_some());
        assert_eq!(m.iter().filter(|&&v| v == 255).count(), 3 * 6);
        assert_eq!((m[5 * w + 7], m[6 * w + 7], m[5 * w + 6]), (255, 0, 0));
        fill_shape(&mut m, w, h, &rect_points((8.0, 1.0), (9.0, 2.0)), 0.0, true);
        assert_eq!((m[w + 8], m[w + 9]), (0, 255));
        assert!(fill_shape(&mut m, w, h, &rect_points((20.0, 1.0), (30.0, 5.0)), 0.0, false).is_none());

        // lasso triangle: the right-angle corner is in, the far side of the diagonal is out
        let mut m = vec![0u8; 20 * 20];
        fill_shape(&mut m, 20, 20, &[(0.0, 0.0), (20.0, 0.0), (0.0, 20.0)], 0.0, false);
        assert_eq!((m[2 * 20 + 2], m[17 * 20 + 17]), (255, 0));

        // soft edge: full inside, a ramp across the edge, nothing far outside
        let mut m = vec![0u8; 40 * 40];
        fill_shape(&mut m, 40, 40, &rect_points((10.0, 10.0), (30.0, 30.0)), 8.0, false);
        let row: Vec<u8> = (0..40).map(|x| m[20 * 40 + x]).collect();
        assert_eq!((row[20], row[2]), (255, 0));
        assert!(row[10] > 60 && row[10] < 200, "edge is half-way: {}", row[10]);
        assert!(row.windows(2).take(20).all(|p| p[0] <= p[1]), "ramps up towards the inside");
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
