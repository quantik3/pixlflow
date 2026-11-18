//! Checks from spec §12.

use pixlflow::{
    box_blur, color,
    engine::{self, Extras},
    paths::{self, PathInput},
    select,
    settings::{Algorithm as A, BlendMode, Channel, Combine, Criterion, PathKind as P, SelectorKind as K, Settings},
    Rgb, Rng,
};

fn noise_image(w: usize, h: usize, seed: u64) -> Vec<Rgb> {
    let mut r = Rng::new(seed);
    (0..w * h).map(|_| [r.below(256) as u8, r.below(256) as u8, r.below(256) as u8]).collect()
}

fn sorted(mut v: Vec<Rgb>) -> Vec<Rgb> {
    v.sort();
    v
}

fn luma(img: &[Rgb]) -> Vec<f32> {
    img.iter().map(|&p| color::brightness(p) as f32).collect()
}

fn some_directions(w: usize, h: usize) -> Vec<f32> {
    (0..w * h).map(|i| if i % w == 3 { 0.7 } else if i / w == 5 && i % w > 10 { 2.5 } else { f32::NAN }).collect()
}

#[test]
fn permuting_algorithms_keep_the_same_pixels() {
    let (w, h) = (37, 23);
    let img = noise_image(w, h, 1);
    let guide: Vec<u8> = noise_image(w, h, 2).iter().map(|p| p[0]).collect();
    let algos = [A::Sort, A::Comb, A::Glitch, A::Partial, A::Shift, A::Mirror, A::Bucket, A::Shuffle];
    let path_kinds = [P::Lines, P::Diagonal, P::Circles, P::Spiral, P::Hilbert, P::Flow, P::Rays, P::Star, P::Waves];
    for &path in &path_kinds {
        for &select in K::ALL {
            for &algo in &algos {
                for by in [Criterion::Brightness, Criterion::Hue, Criterion::Guide] {
                    let s = Settings { path, select, algo, by, length: 7, max_length: 9, passes: 3, ..Default::default() };
                    let out = engine::render(&s, w, h, &img, &Extras { guide: Some(&guide), ..Default::default() });
                    assert_eq!(sorted(out), sorted(img.clone()), "{path:?} {select:?} {algo:?} {by:?}");
                }
            }
        }
    }
}

#[test]
fn sort_sorts_rows_by_brightness() {
    let (w, h) = (50, 10);
    let img = noise_image(w, h, 3);
    let s = Settings { select: K::Full, ..Default::default() };
    let out = engine::render(&s, w, h, &img, &Extras::default());
    for row in out.chunks(w) {
        assert!(row.windows(2).all(|p| color::brightness(p[0]) <= color::brightness(p[1])));
    }
}

#[test]
fn paths_stay_inside_and_never_repeat() {
    let must_cover =
        [P::Lines, P::Columns, P::AllHorizontal, P::AllVertical, P::Tiles, P::Snake, P::Flow, P::Curl, P::Swirls, P::Hilbert, P::Diagonal, P::Waves, P::Contours, P::Drawn];
    for (w, h) in [(1, 1), (1, 7), (7, 1), (17, 9), (32, 32), (41, 23)] {
        let img = noise_image(w, h, 4);
        let l = luma(&img);
        let dirs = some_directions(w, h);
        for &kind in P::ALL {
            for (angle, cx, aspect) in [(30.0, 0.5, 1.0), (135.0, 0.1, 3.0), (270.0, 0.9, 0.2)] {
                for directions in [None, Some(dirs.as_slice())] {
                    let s = Settings { path: kind, angle, cx, aspect, tile: 5, swirls: 4, noise_size: 8.0, ..Default::default() };
                    let ps = paths::build(&s, &PathInput { w, h, luma: Some(&l), directions });
                    let mut seen = vec![false; w * h];
                    for p in &ps {
                        assert!(!p.is_empty(), "{kind:?}: empty path");
                        for &i in p {
                            assert!((i as usize) < w * h, "{kind:?}: outside");
                            assert!(!seen[i as usize], "{kind:?} {w}x{h}: pixel {i} twice");
                            seen[i as usize] = true;
                        }
                    }
                    if must_cover.contains(&kind) {
                        assert!(seen.iter().all(|&s| s), "{kind:?} {w}x{h}: not covered");
                    }
                }
            }
        }
    }
}

#[test]
fn hilbert_steps_to_neighbours() {
    for (w, h) in [(7, 5), (10, 3), (16, 16), (5, 12), (33, 20)] {
        let s = Settings { path: P::Hilbert, ..Default::default() };
        let ps = paths::build(&s, &PathInput { w, h, luma: None, directions: None });
        assert_eq!(ps.len(), 1);
        for pair in ps[0].windows(2) {
            let (a, b) = (pair[0] as i64, pair[1] as i64);
            let (dx, dy) = ((a % w as i64 - b % w as i64).abs(), (a / w as i64 - b / w as i64).abs());
            assert!(dx.max(dy) == 1, "{w}x{h}: jump between {a} and {b}");
        }
    }
}

#[test]
fn pixels_outside_the_mask_never_change() {
    let (w, h) = (31, 19);
    let img = noise_image(w, h, 5);
    let mut r = Rng::new(9);
    let mask: Vec<u8> = (0..w * h).map(|_| [0, 0, 128, 255][r.below(4) as usize]).collect();
    for &algo in A::ALL {
        for (&path, &blend) in [P::Lines, P::Circles, P::Flow, P::Snake].iter().cycle().zip(BlendMode::ALL) {
            let s = Settings {
                algo,
                path,
                blend,
                opacity: 0.8,
                select: K::Full,
                posterize: 3,
                soft_ends: 2,
                fringes: 3,
                channel: Channel::Green,
                alternate: true,
                smooth: 2,
                ..Default::default()
            };
            let out = engine::render(&s, w, h, &img, &Extras { mask: Some(&mask), ..Default::default() });
            for i in 0..w * h {
                if mask[i] == 0 {
                    assert_eq!(out[i], img[i], "{algo:?} {path:?} changed pixel {i}");
                }
            }
        }
    }
}

#[test]
fn spans_are_ordered_inside_and_disjoint() {
    let mut rng = Rng::new(11);
    for n in [0, 1, 2, 5, 64, 300] {
        let px = noise_image(n, 1, n as u64);
        for &select in K::ALL {
            for (length, max_length) in [(0, 0), (1, 1), (3, 2), (100, 50)] {
                let s = Settings { select, length, max_length, min: 100, max: 50, edge: 200, ..Default::default() };
                let mut out = Vec::new();
                select::spans(&s, &px, &mut rng, &mut out);
                let mut end = 0;
                for r in &out {
                    assert!(r.start >= end && r.start < r.end && r.end <= n, "{select:?} n={n}: bad span {r:?}");
                    end = r.end;
                }
                if n == 0 || (select == K::Fixed && length == 0) || (select == K::Random && max_length <= 1) {
                    assert!(out.is_empty(), "{select:?} n={n}: expected no spans");
                }
            }
        }
    }
}

#[test]
fn wrapped_hue_range_selects_red_not_green() {
    let s = Settings { select: K::Threshold, crit: Criterion::Hue, min: 340, max: 20, ..Default::default() };
    assert_eq!(select::pixel_selected(&s, [255, 0, 0]), Some(true));
    assert_eq!(select::pixel_selected(&s, [255, 0, 40]), Some(true)); // hue ~350
    assert_eq!(select::pixel_selected(&s, [0, 255, 0]), Some(false));
    let combined = Settings { select: K::Combined, crit2: Criterion::Brightness, min2: 0, max2: 10, combine: Combine::Or, ..s };
    assert_eq!(select::pixel_selected(&combined, [0, 255, 0]), Some(false));
    assert_eq!(select::pixel_selected(&combined, [0, 0, 0]), Some(true));
}

#[test]
fn preset_text_round_trips() {
    let s = Settings {
        path: P::Star,
        reverse: true,
        cx: 0.123_456_7,
        rotation: -12.5,
        star_inner: 0.3,
        seed: 987_654_321,
        select: K::Combined,
        crit2: Criterion::ColorDistance,
        min: 340,
        max: 20,
        combine: Combine::Or,
        algo: A::Glitch,
        channel: Channel::Blue,
        fringes: -4,
        target: [1, 2, 255],
        ..Default::default()
    };
    let text = s.to_preset();
    assert_eq!(Settings::from_preset(&text).unwrap(), s);
    // unknown keys ignored, missing keys default
    let t = Settings::from_preset("bogus=1 path=snake").unwrap();
    assert_eq!(t, Settings { path: P::Snake, ..Default::default() });
    assert!(Settings::from_preset("path=nowhere").is_err());
}

#[test]
fn box_blur_is_window_mean() {
    let (w, h, r) = (9, 6, 2);
    let src: Vec<f32> = (0..w * h).map(|i| ((i * 37) % 11) as f32).collect();
    let out = box_blur(&src, w, h, r);
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut n) = (0.0, 0.0);
            for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    sum += src[yy * w + xx];
                    n += 1.0;
                }
            }
            assert!((out[y * w + x] - sum / n).abs() < 1e-4);
        }
    }
    assert!(box_blur(&vec![7.0; 40 * 30], 40, 30, 5).iter().all(|&v| v == 7.0));
}

#[test]
fn blend_modes() {
    use engine::blend;
    let (grey, red) = ([128, 128, 128], [255, 0, 0]);
    assert_eq!(blend(BlendMode::Normal, grey, red), red);
    assert_eq!(blend(BlendMode::Multiply, red, [255; 3]), red);
    assert_eq!(blend(BlendMode::Screen, red, [0; 3]), red);
    assert_eq!(blend(BlendMode::Difference, red, red), [0; 3]);
    assert_eq!(blend(BlendMode::Add, grey, grey), [255; 3]);
    assert_eq!(blend(BlendMode::Lighten, [10, 200, 30], [100, 20, 30]), [100, 200, 30]);
    // Color keeps the lower brightness, Luminosity keeps the lower hue
    let c = blend(BlendMode::Color, grey, red);
    assert!(c[0] > c[1] && (color::brightness(c) as i32 - 128).abs() < 40);
    assert_eq!(blend(BlendMode::Luminosity, grey, [255; 3]), [255; 3]);
    // opacity 0 leaves the image as it was
    let img = noise_image(9, 7, 21);
    let s = Settings { select: K::Full, opacity: 0.0, ..Default::default() };
    assert_eq!(engine::render(&s, 9, 7, &img, &Extras::default()), img);
}

#[test]
fn project_round_trips() {
    use pixlflow::project::{Project, ProjectLayer};
    let p = Project {
        image: "C:/pics/фото 1.png".into(),
        current: 1,
        layers: vec![
            ProjectLayer { settings: Settings { path: P::Star, opacity: 0.25, ..Default::default() }, mask: Some(vec![0, 9, 255]), ..Default::default() },
            ProjectLayer {
                settings: Settings::default(),
                guide: Some(vec![1, 2, 3]),
                guide_name: Some("g.png".into()),
                dirs: Some(vec![f32::NAN, 1.5, -3.0]),
                ..Default::default()
            },
        ],
    };
    let bytes = p.to_bytes();
    let q = Project::from_bytes(&bytes).unwrap();
    assert_eq!(q.to_bytes(), bytes); // NaN directions survive bit for bit
    assert_eq!((q.image.as_str(), q.current, q.layers.len()), (p.image.as_str(), 1, 2));
    assert_eq!(q.layers[0].settings, p.layers[0].settings);
    assert_eq!(q.layers[1].guide_name.as_deref(), Some("g.png"));
    assert!(Project::from_bytes(&bytes[..bytes.len() - 2]).is_err(), "truncated file is rejected");
    assert!(Project::from_bytes(b"hello").is_err());
    let mut fitted = q.clone();
    fitted.fit(3, 1);
    assert!(fitted.layers[0].mask.is_some() && fitted.layers[1].dirs.is_some());
    fitted.fit(2, 2);
    assert!(fitted.layers[0].mask.is_none() && fitted.layers[1].guide_name.is_none());
}

#[test]
fn random_settings_need_no_input() {
    let mut rng = Rng::new(5);
    for _ in 0..300 {
        let s = Settings::random(&mut rng);
        assert!(s.path != P::Drawn && s.algo != A::Debug);
        for c in [s.by, s.crit, s.crit2] {
            assert!(c != Criterion::Guide && c != Criterion::ColorDistance);
        }
        assert_eq!(Settings::from_preset(&s.to_preset()).unwrap(), s);
    }
}

#[test]
fn animation_interpolates() {
    use pixlflow::anim::{frame_times, lerp, Ease};
    let a = Settings { cx: 0.1, passes: 10, path: P::Waves, amplitude: 5.0, algo: A::Sort, ..Default::default() };
    let b = Settings { cx: 0.9, passes: 30, path: P::Waves, amplitude: 45.0, algo: A::Mirror, ..Default::default() };
    assert_eq!(lerp(&a, &b, 0.0), a);
    assert_eq!(lerp(&a, &b, 1.0), b);
    let mid = lerp(&a, &b, 0.25);
    assert!((mid.cx - 0.3).abs() < 1e-6 && mid.passes == 15 && mid.amplitude == 15.0 && mid.algo == A::Sort);
    assert_eq!(lerp(&a, &b, 0.5).algo, A::Mirror);
    // different path kinds: path parameters switch instead of moving
    let c = Settings { path: P::Lines, ..b.clone() };
    assert_eq!(lerp(&a, &c, 0.25).amplitude, 5.0);
    assert_eq!(frame_times(3, true, Ease::Linear), vec![0.0, 0.5, 1.0, 0.5]);
    assert_eq!(frame_times(4, false, Ease::In).len(), 4);
    assert_eq!(Ease::InOut.apply(0.5), 0.5);
}

#[test]
fn colour_values() {
    assert_eq!(color::hue([255, 0, 0]), 0);
    assert_eq!(color::hue([0, 255, 0]), 120);
    assert_eq!(color::hue([0, 0, 255]), 240);
    assert_eq!(color::brightness([255, 255, 255]), 255);
    assert_eq!(color::lightness([255, 255, 255]), 255);
    assert_eq!(color::lightness([0, 0, 0]), 0);
    assert_eq!(color::value(Criterion::Warmth, [255, 0, 0], [0; 3]), 255);
    assert_eq!(color::value(Criterion::Warmth, [0, 0, 255], [0; 3]), 0);
    assert_eq!(color::saturation([0, 0, 0]), 0);
    assert_eq!(color::value(Criterion::ColorDistance, [255, 0, 0], [255, 0, 0]), 0);
}
