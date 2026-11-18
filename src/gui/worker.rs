//! Sorting off the UI thread. Requests queue up; the thread always skips to the newest one.

use eframe::egui;
use pixlflow::{engine, settings::Settings, Rgb};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Full-size result.
    Full,
    /// Downscaled result while a slider is being dragged.
    Preview,
    /// Threshold selection map.
    Mask,
}

pub struct Job {
    pub id: u64,
    pub kind: Kind,
    pub settings: Settings,
    pub w: usize,
    pub h: usize,
    pub src: Arc<Vec<Rgb>>,
    pub guide: Option<Arc<Vec<u8>>>,
}

pub struct Done {
    pub id: u64,
    pub kind: Kind,
    pub settings: Settings,
    pub w: usize,
    pub h: usize,
    pub px: Result<Vec<Rgb>, String>,
    pub ms: u128,
}

pub fn run(job: Job) -> Done {
    let started = Instant::now();
    let px = catch_unwind(AssertUnwindSafe(|| match job.kind {
        Kind::Mask => engine::selection_map(&job.settings, job.w, job.h, &job.src).into_iter().map(|v| [v; 3]).collect(),
        Kind::Full | Kind::Preview => {
            let ex = engine::Extras { guide: job.guide.as_ref().map(|g| g.as_slice()), ..Default::default() };
            engine::render(&job.settings, job.w, job.h, &job.src, &ex)
        }
    }))
    .map_err(|_| "Sorting failed with an internal error.".to_string());
    Done { id: job.id, kind: job.kind, settings: job.settings, w: job.w, h: job.h, px, ms: started.elapsed().as_millis() }
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
