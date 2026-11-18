//! Animating one layer between two settings (spec §9, Animation).

use crate::settings::Settings;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Ease {
    #[default]
    Linear,
    In,
    Out,
    InOut,
}

impl Ease {
    pub const ALL: [Ease; 4] = [Ease::Linear, Ease::In, Ease::Out, Ease::InOut];

    pub fn name(self) -> &'static str {
        match self {
            Ease::Linear => "Linear",
            Ease::In => "Ease in",
            Ease::Out => "Ease out",
            Ease::InOut => "Ease in-out",
        }
    }

    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::In => t * t,
            Ease::Out => 1.0 - (1.0 - t) * (1.0 - t),
            Ease::InOut => t * t * (3.0 - 2.0 * t),
        }
    }
}

/// Time (0–1, eased) of every frame. Ping-pong comes back without repeating the two end frames.
pub fn frame_times(frames: usize, ping_pong: bool, ease: Ease) -> Vec<f32> {
    let n = frames.max(2);
    let mut idx: Vec<usize> = (0..n).collect();
    if ping_pong {
        idx.extend((1..n - 1).rev());
    }
    idx.into_iter().map(|i| ease.apply(i as f32 / (n - 1) as f32)).collect()
}

/// Settings at time `t` between `a` and `b`. Numbers move smoothly (path and selector parameters only
/// when both ends use the same path/selector); everything else switches at the middle.
pub fn lerp(a: &Settings, b: &Settings, t: f32) -> Settings {
    let mut s = if t < 0.5 { a.clone() } else { b.clone() };
    let f = |x: f32, y: f32| x * (1.0 - t) + y * t; // exact at both ends
    let u = |x: u32, y: u32| f(x as f32, y as f32).round() as u32;
    (s.cx, s.cy) = (f(a.cx, b.cx), f(a.cy, b.cy));
    (s.rotation, s.angle, s.aspect) = (f(a.rotation, b.rotation), f(a.angle, b.angle), f(a.aspect, b.aspect));
    (s.strength, s.opacity, s.passes) = (f(a.strength, b.strength), f(a.opacity, b.opacity), u(a.passes, b.passes));
    if a.path == b.path {
        (s.amplitude, s.wavelength, s.noise_size) = (f(a.amplitude, b.amplitude), f(a.wavelength, b.wavelength), f(a.noise_size, b.noise_size));
        (s.swirls, s.contour_smooth, s.tile) = (u(a.swirls, b.swirls), u(a.contour_smooth, b.contour_smooth), u(a.tile, b.tile));
        (s.star_points, s.star_inner) = (u(a.star_points, b.star_points), f(a.star_inner, b.star_inner));
    }
    if a.select == b.select {
        (s.length, s.max_length, s.edge, s.smooth) = (u(a.length, b.length), u(a.max_length, b.max_length), u(a.edge, b.edge), u(a.smooth, b.smooth));
        (s.min, s.max, s.min2, s.max2) = (u(a.min, b.min), u(a.max, b.max), u(a.min2, b.min2), u(a.max2, b.max2));
    }
    s
}
