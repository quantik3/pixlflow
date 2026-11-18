//! Animation export (GIF or numbered PNG frames) on its own thread, with progress and Stop.

use super::worker::{render_layer, Layer};
use eframe::egui;
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};
use pixlflow::{anim, imageio, settings::Settings, video, Rgb};
use std::fs::File;
use std::io::BufWriter;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{mpsc, Arc};

pub enum Target {
    /// Looping GIF, longest side at most `side` px.
    Gif { path: PathBuf, side: usize, fps: u32 },
    /// `stem-0001.png`, `stem-0002.png`, … in `dir`.
    Png { dir: PathBuf, stem: String },
}

/// What to render: the animated layer over a fixed image, then the layers above it.
pub struct Frames {
    pub below: Arc<Vec<Rgb>>,
    pub w: usize,
    pub h: usize,
    pub alpha: Option<Vec<u8>>,
    pub layer: Layer,
    pub above: Vec<Layer>,
    pub start: Settings,
    pub end: Settings,
    pub times: Vec<f32>,
}

pub struct Export {
    pub done: Arc<AtomicUsize>,
    pub total: usize,
    pub stop: Arc<AtomicBool>,
    pub started: std::time::Instant,
    result: mpsc::Receiver<Result<String, String>>,
}

impl Export {
    /// The final message once the thread is finished.
    pub fn finished(&self) -> Option<Result<String, String>> {
        self.result.try_recv().ok()
    }
}

/// Runs `job(done, stop)` on its own thread; a panic becomes an error message.
fn spawn_job(
    ctx: egui::Context,
    total: usize,
    job: impl FnOnce(&AtomicUsize, &AtomicBool) -> Result<String, String> + Send + 'static,
) -> Export {
    let (done, stop) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
    let (tx, result) = mpsc::channel();
    let (d, s) = (done.clone(), stop.clone());
    std::thread::spawn(move || {
        let r = catch_unwind(AssertUnwindSafe(|| job(&d, &s))).unwrap_or_else(|_| Err("The job failed with an internal error.".into()));
        let _ = tx.send(r);
        ctx.request_repaint();
    });
    Export { done, total, stop, started: std::time::Instant::now(), result }
}

pub fn start(ctx: egui::Context, frames: Frames, target: Target) -> Export {
    spawn_job(ctx, frames.times.len(), move |done, stop| run(&frames, &target, done, stop))
}

/// A whole video through the layer stack, optionally animating layer `cur` from its start to its end.
pub struct VideoJob {
    pub input: PathBuf,
    pub info: video::Info,
    pub layers: Vec<Layer>,
    pub cur: usize,
    pub anim: Option<(Settings, Settings, anim::Ease)>,
}

impl VideoJob {
    fn frame(&self, i: usize, px: Vec<Rgb>) -> Vec<Rgb> {
        let (w, h) = (self.info.w, self.info.h);
        let last = (self.info.frames.max(2) - 1) as f32;
        self.layers.iter().enumerate().fold(px, |img, (k, l)| match &self.anim {
            Some((a, b, ease)) if k == self.cur => {
                let moving = Layer { s: anim::lerp(a, b, ease.apply(i as f32 / last)), ..l.clone() };
                render_layer(&moving, w, h, &img, (w, h))
            }
            _ => render_layer(l, w, h, &img, (w, h)),
        })
    }
}

/// Writes the sorted video to a file, or plays it (`Sink::Player`, finishes without a message).
pub fn start_video(ctx: egui::Context, job: VideoJob, sink: video::Sink) -> Export {
    spawn_job(ctx, job.info.frames as usize, move |done, stop| {
        let n = video::run(&job.input, &job.info, &sink, |i, px| job.frame(i, px), |i| {
            done.store(i, Relaxed);
            !stop.load(Relaxed)
        })?;
        Ok(match &sink {
            video::Sink::File { out, .. } => format!("Exported {n} frames to {}.", out.display()),
            video::Sink::Player => String::new(),
        })
    })
}

fn run(f: &Frames, target: &Target, done: &AtomicUsize, stop: &AtomicBool) -> Result<String, String> {
    let (w, h) = (f.w, f.h);
    let mut gif = match target {
        Target::Gif { path, .. } => {
            let file = File::create(path).map_err(|e| format!("cannot create '{}': {e}", path.display()))?;
            let mut enc = GifEncoder::new_with_speed(BufWriter::new(file), 10);
            enc.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?;
            Some(enc)
        }
        Target::Png { .. } => None,
    };
    for (i, &t) in f.times.iter().enumerate() {
        if stop.load(Relaxed) {
            return Ok(format!("Export stopped after {i} of {} frames.", f.times.len()));
        }
        let layer = Layer { s: anim::lerp(&f.start, &f.end, t), ..f.layer.clone() };
        let mut img = render_layer(&layer, w, h, &f.below, (w, h));
        for l in &f.above {
            img = render_layer(l, w, h, &img, (w, h));
        }
        match (target, &mut gif) {
            (Target::Gif { side, fps, .. }, Some(enc)) => {
                let k = (*side as f32 / w.max(h) as f32).min(1.0);
                let (gw, gh) = (((w as f32 * k).round() as usize).max(1), ((h as f32 * k).round() as usize).max(1));
                let small = if k < 1.0 { imageio::resize_rgb(&img, w, h, gw, gh) } else { img };
                let rgba = RgbaImage::from_raw(gw as u32, gh as u32, small.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect())
                    .expect("buffer matches size");
                let delay = Delay::from_numer_denom_ms(1000, (*fps).max(1));
                enc.encode_frame(Frame::from_parts(rgba, 0, 0, delay)).map_err(|e| format!("cannot write the GIF: {e}"))?;
            }
            (Target::Png { dir, stem }, _) => imageio::save(&dir.join(format!("{stem}-{:04}.png", i + 1)), w, h, &img, f.alpha.as_deref())?,
            _ => unreachable!(),
        }
        done.fetch_add(1, Relaxed);
    }
    Ok(format!("Exported {} frames.", f.times.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::AnimationDecoder;

    fn frames(n: usize) -> Frames {
        let (w, h) = (12, 8);
        let below = Arc::new((0..w * h).map(|i| [(i * 7 % 256) as u8, (i * 13 % 256) as u8, (i * 29 % 256) as u8]).collect());
        let start = Settings { cx: 0.0, ..Default::default() };
        let end = Settings { cx: 1.0, ..Default::default() };
        let times = anim::frame_times(n, true, anim::Ease::Linear);
        Frames { below, w, h, alpha: None, layer: Layer::default(), above: vec![Layer::default()], start, end, times }
    }

    #[test]
    fn exports_gif_and_png_frames() {
        let dir = std::env::temp_dir().join(format!("pixlflow-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (done, stop) = (AtomicUsize::new(0), AtomicBool::new(false));

        let gif = dir.join("a.gif");
        let msg = run(&frames(4), &Target::Gif { path: gif.clone(), side: 6, fps: 10 }, &done, &stop).unwrap();
        assert_eq!(msg, "Exported 6 frames."); // 4 frames ping-pong: 0 1 2 3 2 1
        let decoded = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(File::open(&gif).unwrap())).unwrap();
        let all = decoded.into_frames().collect_frames().unwrap();
        assert_eq!(all.len(), 6);
        assert_eq!(all[0].buffer().dimensions(), (6, 4), "longest side limited");

        let msg = run(&frames(3), &Target::Png { dir: dir.clone(), stem: "f".into() }, &done, &stop).unwrap();
        assert_eq!(msg, "Exported 4 frames.");
        assert!(dir.join("f-0004.png").is_file() && !dir.join("f-0005.png").exists());

        stop.store(true, Relaxed);
        assert!(run(&frames(3), &Target::Png { dir: dir.clone(), stem: "g".into() }, &done, &stop).unwrap().contains("stopped"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
