//! Layers and sorting off the UI thread. Requests queue up; the thread always skips to the newest one.

use eframe::egui;
use pixlflow::{engine, settings::Settings, Rgb};
use std::borrow::Cow;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc};
use std::time::Instant;

/// Everything one layer owns. Buffers are shared, so undo snapshots don't copy unchanged ones.
#[derive(Clone, Default)]
pub struct Layer {
    pub s: Settings,
    pub mask: Option<Arc<Vec<u8>>>,
    pub guide: Option<Arc<Vec<u8>>>,
    pub guide_name: Option<String>,
    /// Drawn directions in radians, NaN where nothing is drawn.
    pub dirs: Option<Arc<Vec<f32>>>,
}

impl PartialEq for Layer {
    fn eq(&self, o: &Self) -> bool {
        fn same<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
            match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
        }
        self.s == o.s && same(&self.mask, &o.mask) && same(&self.guide, &o.guide) && same(&self.dirs, &o.dirs)
    }
}

/// Applies one layer to `input` (w×h). Layer buffers are `full`-sized; a smaller `input` is a preview.
pub fn render_layer(l: &Layer, w: usize, h: usize, input: &[Rgb], full: (usize, usize)) -> Vec<Rgb> {
    let preview = (w, h) != full;
    fn fit<'a, T: Copy>(v: &'a Option<Arc<Vec<T>>>, full: (usize, usize), w: usize, h: usize) -> Option<Cow<'a, [T]>> {
        let v = v.as_deref()?;
        Some(if (w, h) == full { Cow::Borrowed(v.as_slice()) } else { Cow::Owned(nearest(v, full, w, h)) })
    }
    let s = if preview { l.s.scaled(w as f32 / full.0 as f32) } else { l.s.clone() };
    let (mask, guide, dirs) = (fit(&l.mask, full, w, h), fit(&l.guide, full, w, h), fit(&l.dirs, full, w, h));
    let ex = engine::Extras { mask: mask.as_deref(), guide: guide.as_deref(), directions: dirs.as_deref() };
    engine::render(&s, w, h, input, &ex)
}

/// Nearest-neighbour downscale (keeps NaN directions intact).
fn nearest<T: Copy>(v: &[T], (fw, fh): (usize, usize), w: usize, h: usize) -> Vec<T> {
    (0..w * h).map(|i| v[(i / w) * fh / h * fw + (i % w) * fw / w]).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Full-size result.
    Full,
    /// Downscaled result while the mouse button is held.
    Preview,
    /// Threshold selection map of the last layer in the job, over the layers below it.
    Mask,
}

pub struct Job {
    pub id: u64,
    pub kind: Kind,
    /// Image the first layer of `layers` applies to.
    pub base: Arc<Vec<Rgb>>,
    pub w: usize,
    pub h: usize,
    /// Size of the layer buffers (the full image).
    pub full: (usize, usize),
    /// Layers `start..` of the stack, in order.
    pub layers: Vec<Layer>,
    pub start: usize,
}

pub struct Done {
    pub id: u64,
    pub kind: Kind,
    pub start: usize,
    pub w: usize,
    pub h: usize,
    /// Full-size result of each layer applied (layer `start + i`), for the cache.
    pub results: Vec<(Layer, Arc<Vec<Rgb>>)>,
    pub display: Result<Arc<Vec<Rgb>>, String>,
    pub ms: u128,
}

pub fn run(job: Job) -> Done {
    let started = Instant::now();
    let mut results = Vec::new();
    let display = catch_unwind(AssertUnwindSafe(|| {
        let apply = if job.kind == Kind::Mask { job.layers.len() - 1 } else { job.layers.len() };
        let mut img = job.base.clone();
        for l in &job.layers[..apply] {
            img = Arc::new(render_layer(l, job.w, job.h, &img, job.full));
            if job.kind != Kind::Preview {
                results.push((l.clone(), img.clone()));
            }
        }
        if job.kind == Kind::Mask {
            let s = &job.layers[apply].s;
            img = Arc::new(engine::selection_map(s, job.w, job.h, &img).into_iter().map(|v| [v; 3]).collect());
        }
        img
    }))
    .map_err(|_| "Sorting failed with an internal error.".to_string());
    Done { id: job.id, kind: job.kind, start: job.start, w: job.w, h: job.h, results, display, ms: started.elapsed().as_millis() }
}

pub struct Worker {
    tx: mpsc::Sender<Job>,
    pub rx: mpsc::Receiver<Done>,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, jobs) = mpsc::channel::<Job>();
        let (done, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(mut job) = jobs.recv() {
                while let Ok(newer) = jobs.try_recv() {
                    job = newer;
                }
                if done.send(run(job)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Worker { tx, rx }
    }

    pub fn send(&self, job: Job) {
        let _ = self.tx.send(job); // the thread only stops when the app does
    }
}
