//! The window: settings panel on the left, image on the right, status bar below.

mod export;
mod panel;
mod presets;
mod view;
mod worker;

use eframe::egui::{self, Color32, ColorImage, Key, Modifiers, TextureHandle, TextureOptions};
use pixlflow::{
    anim::{self, Ease},
    engine, imageio,
    project::{Project, ProjectLayer},
    settings::Settings,
    video, Rgb, Rng,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use worker::{Done, Job, Kind, Layer, Worker};

const PANEL_WIDTH: f32 = 380.0;
const PREVIEW_SIDE: usize = 1000;
const UNDO_STEPS: usize = 40;
const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];
const PROJECT_EXT: &str = "pxlf";

pub fn run(settings: Settings, input: Option<PathBuf>, output: Option<PathBuf>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pixlflow")
            .with_inner_size([1280.0, 760.0])
            .with_drag_and_drop(true)
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Pixlflow",
        options,
        Box::new(move |cc| {
            // dark everywhere, matching the neutral dark backdrop behind the image
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            let mut app = App::new(cc.egui_ctx.clone(), settings, output);
            if let Some(p) = input {
                app.open(&cc.egui_ctx, &p);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| format!("cannot start the window: {e}"))
}

/// The image being edited.
struct Doc {
    path: PathBuf,
    w: usize,
    h: usize,
    src: Arc<Vec<Rgb>>,
    alpha: Option<Vec<u8>>,
    /// Downscaled copy for fast previews (only for big images).
    small: Option<Small>,
    orig_tex: TextureHandle,
    result_tex: TextureHandle,
    /// Brush overlay and what it currently shows: (layer, tool, buffer address).
    overlay: Option<(TextureHandle, (usize, Tool, usize))>,
}

struct Small {
    w: usize,
    h: usize,
    src: Arc<Vec<Rgb>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Center,
    Color,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tool {
    Mask,
    Directions,
    /// Drag a rectangle; it fills the mask on release.
    Rect,
    /// Draw a free outline; it fills the mask on release.
    Lasso,
}

struct Brush {
    on: bool,
    tool: Tool,
    radius: f32,
    hardness: f32,
    /// Swap the buttons: left erases, right paints.
    erase: bool,
    /// Last stamp position (image pixels) of the stroke in progress.
    last: Option<(f32, f32)>,
    /// Rect / Lasso outline being dragged (image pixels; Rect: start and current corner), erase.
    shape: Option<(Vec<(f32, f32)>, bool)>,
    /// Soft edge of Rect / Lasso fills, px.
    feather: f32,
}

/// Layers plus the current layer index.
type Snapshot = (Vec<Layer>, usize);

/// Animation of the current layer between two settings.
struct Anim {
    start: Option<Settings>,
    end: Option<Settings>,
    frames: usize,
    fps: u32,
    ping_pong: bool,
    ease: Ease,
    /// Moment shown by the Try slider, 0–1.
    try_t: f32,
    gif_side: usize,
}

/// The opened video (the document shows one of its frames).
struct VideoDoc {
    info: video::Info,
    /// Time of the frame being edited, seconds.
    t: f64,
    /// Export animates the current layer from Start to End over the clip.
    animate: bool,
}

pub struct App {
    layers: Vec<Layer>,
    cur: usize,
    /// Full-size result per layer; entry i is valid while it was made from `layers[i]` and 0..i are valid.
    cache: Vec<(Layer, Arc<Vec<Rgb>>)>,
    committed: Vec<Layer>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    doc: Option<Doc>,
    worker: Worker,
    next_id: u64,
    shown_id: u64,
    /// Last request sent: (layers, current layer, kind).
    requested: Option<(Vec<Layer>, usize, Kind)>,
    sort_now: bool,
    auto_sort: bool,
    background: bool,
    fast_preview: bool,
    compare: bool,
    show_mask: bool,
    show_original: bool,
    /// Show the result of the selected layer and those below it instead of the whole stack.
    upto_current: bool,
    pick: Option<Pick>,
    brush: Brush,
    view: view::View,
    out_dir: Option<PathBuf>,
    /// Output path given on the command line: Save writes exactly there.
    fixed_output: Option<PathBuf>,
    sort_ms: Option<u128>,
    saved_at: Option<Instant>,
    message: Option<String>,
    presets: Vec<(String, String)>,
    preset_pick: Option<usize>,
    preset_name: String,
    preset_text: String,
    rng: Rng,
    anim: Anim,
    video: Option<VideoDoc>,
    /// Background animation export, video export or video playback.
    job: Option<export::Export>,
}

impl App {
    fn new(ctx: egui::Context, s: Settings, fixed_output: Option<PathBuf>) -> Self {
        let layers = vec![Layer { s, ..Default::default() }];
        App {
            committed: layers.clone(),
            layers,
            cur: 0,
            cache: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            doc: None,
            worker: Worker::spawn(ctx),
            next_id: 0,
            shown_id: 0,
            requested: None,
            sort_now: false,
            auto_sort: true,
            background: true,
            fast_preview: true,
            compare: false,
            show_mask: false,
            show_original: false,
            upto_current: false,
            pick: None,
            brush: Brush { on: false, tool: Tool::Mask, radius: 40.0, hardness: 0.5, erase: false, last: None, shape: None, feather: 0.0 },
            view: view::View::default(),
            out_dir: None,
            fixed_output,
            sort_ms: None,
            saved_at: None,
            message: None,
            presets: presets::load(),
            preset_pick: None,
            preset_name: String::new(),
            preset_text: String::new(),
            rng: Rng::new(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64)),
            anim: Anim { start: None, end: None, frames: 30, fps: 15, ping_pong: false, ease: Ease::Linear, try_t: 0.0, gif_side: 600 },
            video: None,
            job: None,
        }
    }

    fn layer(&mut self) -> &mut Layer {
        &mut self.layers[self.cur]
    }

    /// Opens an image, a video or a project file.
    fn open(&mut self, ctx: &egui::Context, path: &Path) {
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXT)) {
            self.open_project(ctx, path);
        } else if video::is_video(path) {
            self.open_video(ctx, path);
        } else {
            self.open_image(ctx, path);
        }
    }

    fn open_image(&mut self, ctx: &egui::Context, path: &Path) -> bool {
        let pic = match imageio::load(path) {
            Ok(p) => p,
            Err(e) => {
                self.message = Some(e);
                return false;
            }
        };
        if pic.deep {
            self.message = Some("The image has more than 8 bits per channel; it was reduced to 8 bits.".into());
        }
        self.video = None;
        self.set_doc(ctx, path, pic);
        true
    }

    /// A video opens on its first frame; that frame is edited like an image.
    fn open_video(&mut self, ctx: &egui::Context, path: &Path) -> bool {
        let opened = video::probe(path).and_then(|info| video::frame_at(path, &info, 0.0).map(|px| (info, px)));
        match opened {
            Ok((info, rgb)) => {
                self.set_doc(ctx, path, imageio::Picture { w: info.w, h: info.h, rgb, alpha: None, deep: false });
                self.video = Some(VideoDoc { info, t: 0.0, animate: false });
                true
            }
            Err(e) => {
                self.message = Some(e);
                false
            }
        }
    }

    /// Shows the video frame at `t` seconds, keeping every layer's masks and directions.
    fn show_video_frame(&mut self, t: f64) {
        let (Some(doc), Some(v)) = (&mut self.doc, &self.video) else { return };
        match video::frame_at(&doc.path, &v.info, t) {
            Ok(rgb) => {
                doc.orig_tex.set(color_image(doc.w, doc.h, &rgb, None), tex_options());
                doc.small = small_copy(doc.w, doc.h, &rgb);
                doc.src = Arc::new(rgb);
                self.cache.clear();
                self.requested = None;
            }
            Err(e) => self.message = Some(e),
        }
    }

    fn set_doc(&mut self, ctx: &egui::Context, path: &Path, pic: imageio::Picture) {
        let (w, h) = (pic.w, pic.h);
        let image = color_image(w, h, &pic.rgb, pic.alpha.as_deref());
        self.doc = Some(Doc {
            path: path.to_path_buf(),
            w,
            h,
            orig_tex: ctx.load_texture("original", image.clone(), tex_options()),
            result_tex: ctx.load_texture("result", image, tex_options()),
            small: small_copy(w, h, &pic.rgb),
            src: Arc::new(pic.rgb),
            alpha: pic.alpha,
            overlay: None,
        });
        // masks, guides and directions belong to the previous image; settings stay
        for l in &mut self.layers {
            (l.mask, l.guide, l.guide_name, l.dirs) = (None, None, None, None);
        }
        self.committed = self.layers.clone();
        self.undo.clear();
        self.redo.clear();
        self.cache.clear();
        self.shown_id = self.next_id; // drop results still coming for the previous image
        self.requested = None;
        self.view.fit_pending = true;
    }

    fn open_project(&mut self, ctx: &egui::Context, path: &Path) {
        let read = std::fs::read(path).map_err(|e| format!("cannot open '{}': {e}", path.display()));
        let mut proj = match read.and_then(|b| Project::from_bytes(&b)) {
            Ok(p) => p,
            Err(e) => return self.message = Some(format!("{e}: {}", path.display())),
        };
        // the image may have moved together with the project
        let saved = PathBuf::from(&proj.image);
        let beside = saved.file_name().and_then(|n| path.parent().map(|d| d.join(n)));
        let Some(image) = [Some(saved.clone()), beside].into_iter().flatten().find(|p| p.is_file()) else {
            return self.message = Some(format!("The project's image '{}' was not found, not even next to the project.", saved.display()));
        };
        let opened = if video::is_video(&image) { self.open_video(ctx, &image) } else { self.open_image(ctx, &image) };
        if !opened {
            return;
        }
        let doc = self.doc.as_ref().expect("image just opened");
        proj.fit(doc.w, doc.h);
        self.layers = proj
            .layers
            .into_iter()
            .map(|l| Layer {
                s: l.settings,
                mask: l.mask.map(Arc::new),
                guide: l.guide.map(Arc::new),
                guide_name: l.guide_name,
                dirs: l.dirs.map(Arc::new),
            })
            .collect();
        self.cur = proj.current;
        self.committed = self.layers.clone();
    }

    fn save_project(&mut self) {
        let Some(doc) = &self.doc else { return };
        let (stem, _) = stem_ext(&doc.path);
        let mut dialog = rfd::FileDialog::new().set_file_name(format!("{stem}.{PROJECT_EXT}")).add_filter("Pixlflow project", &[PROJECT_EXT]);
        if let Some(dir) = doc.path.parent() {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return };
        let proj = Project {
            image: std::path::absolute(&doc.path).unwrap_or_else(|_| doc.path.clone()).to_string_lossy().into_owned(),
            current: self.cur,
            layers: self
                .layers
                .iter()
                .map(|l| ProjectLayer {
                    settings: l.s.clone(),
                    mask: l.mask.as_deref().cloned(),
                    guide: l.guide.as_deref().cloned(),
                    guide_name: l.guide_name.clone(),
                    dirs: l.dirs.as_deref().cloned(),
                })
                .collect(),
        };
        match std::fs::write(&path, proj.to_bytes()) {
            Ok(()) => self.saved_at = Some(Instant::now()),
            Err(e) => self.message = Some(format!("cannot save '{}': {e}", path.display())),
        }
    }

    // ---- presets and animation ----

    fn save_preset(&mut self) {
        let name = presets::clean_name(&self.preset_name);
        let text = self.layers[self.cur].s.to_preset();
        match self.presets.iter_mut().find(|(n, _)| *n == name) {
            Some(p) => p.1 = text,
            None => self.presets.push((name.clone(), text)),
        }
        self.preset_pick = self.presets.iter().position(|(n, _)| *n == name);
        if let Err(e) = presets::save(&self.presets) {
            self.message = Some(e);
        }
    }

    fn delete_preset(&mut self, i: usize) {
        self.presets.remove(i);
        self.preset_pick = None;
        if let Err(e) = presets::save(&self.presets) {
            self.message = Some(e);
        }
    }

    /// Replaces the current layer's settings with preset text (missing keys take defaults).
    fn apply_preset_text(&mut self, text: &str) {
        match Settings::from_preset(text) {
            Ok(s) => self.layer().s = s,
            Err(e) => self.message = Some(format!("Preset not applied: {e}")),
        }
    }

    fn start_export(&mut self, ctx: &egui::Context, gif: bool) {
        let (Some(start), Some(end), Some(doc)) = (self.anim.start.clone(), self.anim.end.clone(), &self.doc) else { return };
        let (stem, _) = stem_ext(&doc.path);
        let dir = self.out_dir.clone().or_else(|| doc.path.parent().map(Path::to_path_buf));
        let target = if gif {
            let mut d = rfd::FileDialog::new().set_file_name(format!("{stem}-anim.gif")).add_filter("GIF", &["gif"]);
            if let Some(dir) = &dir {
                d = d.set_directory(dir);
            }
            let Some(path) = d.save_file() else { return };
            export::Target::Gif { path, side: self.anim.gif_side, fps: self.anim.fps }
        } else {
            let mut d = rfd::FileDialog::new();
            if let Some(dir) = &dir {
                d = d.set_directory(dir);
            }
            let Some(dir) = d.pick_folder() else { return };
            export::Target::Png { dir, stem }
        };
        let (w, h, alpha) = (doc.w, doc.h, doc.alpha.clone());
        let Some(below) = self.chain(self.cur) else { return }; // layers below the animated one: sorted once
        let frames = export::Frames {
            below,
            w,
            h,
            alpha,
            layer: self.layers[self.cur].clone(),
            above: self.layers[self.cur + 1..].to_vec(),
            start,
            end,
            times: anim::frame_times(self.anim.frames, self.anim.ping_pong, self.anim.ease),
        };
        self.job = Some(export::start(ctx.clone(), frames, target));
    }

    /// Plays the video sorted by the whole layer stack, or writes it to a file chosen in a dialog.
    fn start_video(&mut self, ctx: &egui::Context, play: bool) {
        let (Some(doc), Some(v)) = (&self.doc, &self.video) else { return };
        let sink = if play {
            video::Sink::Player
        } else {
            let (stem, _) = stem_ext(&doc.path);
            let ext = doc.path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "mp4".into());
            let mut d = rfd::FileDialog::new()
                .set_file_name(format!("{stem}-sorted.{ext}"))
                .add_filter("MP4", &["mp4"])
                .add_filter("Matroska", &["mkv"])
                .add_filter("WebM", &["webm"])
                .add_filter("QuickTime", &["mov"]);
            if let Some(dir) = self.out_dir.as_deref().or(doc.path.parent()) {
                d = d.set_directory(dir);
            }
            let Some(out) = d.save_file() else { return };
            if out == doc.path {
                return self.message = Some("Choose a new file: the video can't overwrite itself.".into());
            }
            video::Sink::File { out, streams_from: doc.path.clone() }
        };
        let anim = match (v.animate, &self.anim.start, &self.anim.end) {
            (true, Some(a), Some(b)) => Some((a.clone(), b.clone(), self.anim.ease)),
            _ => None,
        };
        let job = export::VideoJob { input: doc.path.clone(), info: v.info.clone(), layers: self.layers.clone(), cur: self.cur, anim };
        self.job = Some(export::start_video(ctx.clone(), job, sink));
    }

    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(ex) = &self.job else { return };
        match ex.finished() {
            Some(r) => {
                let text = r.unwrap_or_else(|e| e);
                if !text.is_empty() {
                    self.message = Some(text);
                }
                self.job = None;
            }
            None => ctx.request_repaint_after(Duration::from_millis(100)), // keep the progress bar moving
        }
    }

    fn load_guide(&mut self, path: &Path) {
        let Some(doc) = &self.doc else { return };
        match imageio::load_gray(path, doc.w, doc.h, true) {
            Ok(g) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                let l = self.layer();
                (l.guide, l.guide_name) = (Some(Arc::new(g)), name);
            }
            Err(e) => self.message = Some(e),
        }
    }

    // ---- layers ----

    fn add_layer(&mut self) {
        self.layers.insert(self.cur + 1, Layer::default());
        self.cur += 1;
        self.cache.truncate(self.cur);
    }

    fn delete_layer(&mut self, i: usize) {
        if self.layers.len() < 2 {
            return; // the last layer stays
        }
        self.layers.remove(i);
        self.cache.truncate(i);
        if self.cur > i {
            self.cur -= 1;
        }
        self.cur = self.cur.min(self.layers.len() - 1);
    }

    /// How many bottom layers have an up-to-date cached result.
    fn valid(&self) -> usize {
        self.cache.iter().zip(&self.layers).take_while(|((done, _), now)| done == *now).count()
    }

    /// Image after layers `0..upto`, sorting now whatever isn't cached.
    fn chain(&mut self, upto: usize) -> Option<Arc<Vec<Rgb>>> {
        let start = self.valid().min(upto);
        let doc = self.doc.as_ref()?;
        let mut img = if start == 0 { doc.src.clone() } else { self.cache[start - 1].1.clone() };
        for i in start..upto {
            img = Arc::new(worker::render_layer(&self.layers[i], doc.w, doc.h, &img, (doc.w, doc.h)));
            self.cache.truncate(i);
            self.cache.push((self.layers[i].clone(), img.clone()));
        }
        Some(img)
    }

    // ---- masks ----

    fn mask_action(&mut self, act: MaskAction) {
        let Some(doc) = &self.doc else { return };
        let n = doc.w * doc.h;
        let (w, h) = (doc.w, doc.h);
        let new = match act {
            MaskAction::Fill => Some(vec![255; n]),
            MaskAction::Clear => Some(vec![0; n]),
            MaskAction::Invert => Some(self.layers[self.cur].mask.as_ref().map_or(vec![0; n], |m| m.iter().map(|v| 255 - v).collect())),
            MaskAction::Remove => None,
            MaskAction::FromGuide => match &self.layers[self.cur].guide {
                Some(g) => Some(g.to_vec()),
                None => return self.message = Some("This layer has no guide image.".into()),
            },
            MaskAction::FromThreshold => {
                let Some(input) = self.chain(self.cur) else { return };
                Some(engine::selection_map(&self.layers[self.cur].s, w, h, &input))
            }
            MaskAction::Load => {
                let Some(p) = rfd::FileDialog::new().add_filter("Images", &IMAGE_EXTS).pick_file() else { return };
                match imageio::load_gray(&p, w, h, true) {
                    Ok(m) => Some(m),
                    Err(e) => return self.message = Some(e),
                }
            }
            MaskAction::Save => {
                let rgb: Vec<Rgb> = match &self.layers[self.cur].mask {
                    Some(m) => m.iter().map(|&v| [v; 3]).collect(),
                    None => vec![[255; 3]; n],
                };
                if let Some(p) = rfd::FileDialog::new().set_file_name("mask.png").add_filter("PNG", &["png"]).save_file() {
                    if let Err(e) = imageio::save(&p, w, h, &rgb, None) {
                        self.message = Some(e);
                    }
                }
                return;
            }
        };
        self.layer().mask = new.map(Arc::new);
    }

    // ---- sorting ----

    fn request_sort(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        let (w, h, src) = (doc.w, doc.h, doc.src.clone());
        let small = doc.small.as_ref().map(|s| (s.w, s.h, s.src.clone()));
        let dragging = ctx.input(|i| i.pointer.any_down());
        let (changed, last_kind, moved) = match &self.requested {
            Some((l, c, k)) => (*l != self.layers || (self.show_mask && *c != self.cur), Some(*k), *c != self.cur),
            None => (true, None, false),
        };
        let kind = if self.show_mask {
            Kind::Mask
        } else if dragging && self.fast_preview && small.is_some() {
            Kind::Preview
        } else {
            Kind::Full
        };
        let need = self.sort_now
            || (self.upto_current && moved)
            || match kind {
                Kind::Mask => changed || last_kind != Some(Kind::Mask),
                Kind::Preview => changed && self.auto_sort,
                Kind::Full => (changed && self.auto_sort) || last_kind.is_some_and(|k| k != Kind::Full),
            };
        if !need {
            return;
        }
        self.sort_now = false;
        self.requested = Some((self.layers.clone(), self.cur, kind));
        self.next_id += 1;
        let (n, full) = (if self.upto_current { self.cur + 1 } else { self.layers.len() }, (w, h));
        let job = match (small, kind) {
            (Some((sw, sh, small_src)), Kind::Preview) => Job {
                id: self.next_id,
                kind,
                base: small_src,
                w: sw,
                h: sh,
                full,
                layers: self.layers[..n].to_vec(),
                start: 0,
            },
            _ => {
                let end = if kind == Kind::Mask { self.cur + 1 } else { n };
                let start = self.valid().min(end - (kind == Kind::Mask) as usize);
                if start == end {
                    // everything cached: show it right away
                    self.shown_id = self.next_id;
                    let img = self.cache[end - 1].1.clone();
                    self.show(w, h, &img, true);
                    return;
                }
                let base = if start == 0 { src } else { self.cache[start - 1].1.clone() };
                Job { id: self.next_id, kind, base, w, h, full, layers: self.layers[start..end].to_vec(), start }
            }
        };
        if self.background {
            self.worker.send(job);
        } else {
            let done = worker::run(job);
            self.apply(done);
        }
    }

    fn receive(&mut self) {
        while let Ok(done) = self.worker.rx.try_recv() {
            self.apply(done);
        }
    }

    fn apply(&mut self, d: Done) {
        if d.id <= self.shown_id {
            return; // older than what is on screen, or for a previous image
        }
        self.shown_id = d.id;
        for (k, r) in d.results.into_iter().enumerate() {
            let i = d.start + k;
            if i > self.cache.len() {
                break; // the stack changed meanwhile
            }
            self.cache.truncate(i);
            self.cache.push(r);
        }
        match d.display {
            Err(e) => self.message = Some(e),
            Ok(img) => {
                self.show(d.w, d.h, &img, d.kind == Kind::Full);
                if d.kind != Kind::Mask {
                    self.sort_ms = Some(d.ms);
                }
            }
        }
    }

    fn show(&mut self, w: usize, h: usize, px: &[Rgb], with_alpha: bool) {
        if let Some(doc) = &mut self.doc {
            let alpha = if with_alpha { doc.alpha.as_deref() } else { None };
            doc.result_tex.set(color_image(w, h, px, alpha), tex_options());
        }
    }

    // ---- files ----

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let dialog = rfd::FileDialog::new()
            .add_filter("Images, videos and projects", &[&IMAGE_EXTS[..], &video::VIDEO_EXTS[..6], &[PROJECT_EXT]].concat())
            .add_filter("Pixlflow project", &[PROJECT_EXT]);
        if let Some(p) = dialog.pick_file() {
            self.open(ctx, &p);
        }
    }

    fn guide_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Images", &IMAGE_EXTS).pick_file() {
            self.load_guide(&p);
        }
    }

    fn save(&mut self) {
        let Some(doc) = &self.doc else { return };
        let path = match &self.fixed_output {
            Some(p) => p.clone(),
            None => {
                let dir = self.out_dir.clone().or_else(|| doc.path.parent().map(Path::to_path_buf)).unwrap_or_default();
                let (stem, ext) = stem_ext(&doc.path);
                let s = &self.layers[self.layers.len() - 1].s;
                let mut name = format!("{stem}-{}-{}-{}-{}", s.path.name(), s.select.name(), s.by.name(), s.algo.name());
                if self.layers.len() > 1 {
                    name += &format!("-{}layers", self.layers.len());
                }
                unique_path(&dir, &name, &ext)
            }
        };
        self.save_to(&path);
    }

    fn save_as(&mut self) {
        let Some(doc) = &self.doc else { return };
        let (stem, ext) = stem_ext(&doc.path);
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(format!("{stem}-sorted.{ext}"))
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("WebP", &["webp"])
            .add_filter("GIF", &["gif"]);
        if let Some(dir) = self.out_dir.as_deref().or(doc.path.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(p) = dialog.save_file() {
            self.save_to(&p);
        }
    }

    fn save_to(&mut self, path: &Path) {
        let Some(px) = self.chain(self.layers.len()) else { return };
        let doc = self.doc.as_ref().expect("chain needs a doc");
        match imageio::save(path, doc.w, doc.h, &px, doc.alpha.as_deref()) {
            Ok(()) => self.saved_at = Some(Instant::now()),
            Err(e) => self.message = Some(e),
        }
    }

    // ---- undo ----

    /// Records a step once the mouse is up, so a whole drag or brush stroke is one step.
    fn commit_undo(&mut self, ctx: &egui::Context) {
        if self.layers == self.committed || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let prev = std::mem::replace(&mut self.committed, self.layers.clone());
        self.undo.push((prev, self.cur));
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn restore(&mut self, (layers, cur): Snapshot) -> Snapshot {
        let now = (std::mem::replace(&mut self.layers, layers), self.cur);
        self.cur = cur.min(self.layers.len() - 1);
        self.committed = self.layers.clone();
        now
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            let now = self.restore(prev);
            self.redo.push(now);
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let now = self.restore(next);
            self.undo.push(now);
        }
    }

    // ---- input ----

    fn hotkeys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return; // typing in a field
        }
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        // shifted variants first, so Ctrl+Shift+S is not taken as Ctrl+S
        let [open, save_as, save, redo, undo, fit, mask, brush] = ctx.input_mut(|i| {
            [
                i.consume_key(cmd, Key::O),
                i.consume_key(cmd_shift, Key::S),
                i.consume_key(cmd, Key::S),
                i.consume_key(cmd_shift, Key::Z) || i.consume_key(cmd, Key::Y),
                i.consume_key(cmd, Key::Z),
                i.consume_key(Modifiers::NONE, Key::F),
                i.consume_key(Modifiers::NONE, Key::M),
                i.consume_key(Modifiers::NONE, Key::B),
            ]
        });
        if open {
            self.open_dialog(ctx);
        }
        if save_as {
            self.save_as();
        }
        if save {
            self.save();
        }
        if redo {
            self.commit_undo(ctx);
            self.redo();
        }
        if undo {
            self.commit_undo(ctx);
            self.undo();
        }
        if fit {
            self.view.fit_pending = true;
        }
        if mask {
            self.show_mask = !self.show_mask;
        }
        if brush {
            self.brush.on = !self.brush.on;
        }
    }

    fn dropped_files(&mut self, ctx: &egui::Context) {
        let (dropped, hovering) =
            ctx.input(|i| (i.raw.dropped_files.first().map(|f| f.path().to_path_buf()), !i.raw.hovered_files.is_empty()));
        if let Some(p) = dropped {
            self.open(ctx, &p);
        }
        if hovering {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            let rect = ctx.content_rect();
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop the image to open it",
                egui::FontId::proportional(28.0),
                Color32::WHITE,
            );
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match self.sort_ms {
                Some(ms) => ui.label(format!("Sorted in {ms} ms")),
                None => ui.label("Not sorted yet"),
            };
            if let Some(doc) = &self.doc {
                ui.separator();
                ui.label(format!("{} × {}", doc.w, doc.h));
                ui.separator();
                ui.label(doc.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
            }
            if let Some(t) = self.saved_at {
                let left = Duration::from_secs(2).saturating_sub(t.elapsed());
                if left.is_zero() {
                    self.saved_at = None;
                } else {
                    ui.separator();
                    ui.colored_label(Color32::from_rgb(90, 200, 120), "Saved!");
                    ui.ctx().request_repaint_after(left);
                }
            }
        });
    }

    fn message_window(&mut self, ctx: &egui::Context) {
        let Some(msg) = &self.message else { return };
        let mut close = false;
        egui::Window::new("Pixlflow")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(msg);
                close = ui.button("OK").clicked();
            });
        if close {
            self.message = None;
        }
    }
}

#[derive(Clone, Copy)]
enum MaskAction {
    Fill,
    Clear,
    Invert,
    Remove,
    FromThreshold,
    FromGuide,
    Load,
    Save,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive();
        self.poll_export(&ctx);
        self.dropped_files(&ctx);
        self.hotkeys(&ctx);
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("settings").exact_size(PANEL_WIDTH).resizable(false).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.settings_panel(ui));
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.image_view(ui));
        self.message_window(&ctx);
        self.commit_undo(&ctx);
        self.request_sort(&ctx);
    }
}

/// Downscaled copy for fast previews, for images with a long side over `PREVIEW_SIDE`.
fn small_copy(w: usize, h: usize, rgb: &[Rgb]) -> Option<Small> {
    let long = w.max(h);
    (long > PREVIEW_SIDE).then(|| {
        let k = PREVIEW_SIDE as f32 / long as f32;
        let (sw, sh) = (((w as f32 * k).round() as usize).max(1), ((h as f32 * k).round() as usize).max(1));
        Small { w: sw, h: sh, src: Arc::new(imageio::resize_rgb(rgb, w, h, sw, sh)) }
    })
}

fn tex_options() -> TextureOptions {
    TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR }
}

fn color_image(w: usize, h: usize, px: &[Rgb], alpha: Option<&[u8]>) -> ColorImage {
    match alpha {
        Some(a) => {
            let rgba: Vec<u8> = px.iter().zip(a).flat_map(|(p, &a)| [p[0], p[1], p[2], a]).collect();
            ColorImage::from_rgba_unmultiplied([w, h], &rgba)
        }
        None => ColorImage::from_rgb([w, h], px.as_flattened()),
    }
}

fn stem_ext(path: &Path) -> (String, String) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .filter(|e| IMAGE_EXTS.contains(&e.as_str()))
        .unwrap_or_else(|| "png".into());
    (stem, ext)
}

/// `dir/name.ext`, or `name-2.ext`, `name-3.ext`, … if taken.
fn unique_path(dir: &Path, name: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{name}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{name}-{n}.{ext}"));
        n += 1;
    }
    p
}

/// Window icon, drawn in code: bars of colour sorted from dark to light.
fn icon() -> egui::IconData {
    const S: usize = 64;
    let mut rgba = vec![0u8; S * S * 4];
    for y in 0..S {
        for x in 0..S {
            let (dx, dy) = ((x as f32 - 31.5).abs() - 24.0, (y as f32 - 31.5).abs() - 24.0);
            let corner = dx.max(0.0).hypot(dy.max(0.0));
            if corner > 7.5 {
                continue; // outside the rounded square
            }
            let bar = x * 6 / S;
            let hue = 0.08 + bar as f32 * 0.14;
            let top = ((bar * 37) % 5) as f32 * 0.08; // ragged bar starts, like sorted spans
            let t = (y as f32 / S as f32 - top).clamp(0.0, 1.0);
            let c: Color32 = egui::ecolor::Hsva::new(hue, 0.75, 0.25 + 0.75 * t, 1.0).into();
            rgba[(y * S + x) * 4..][..4].copy_from_slice(&[c.r(), c.g(), c.b(), 255]);
        }
    }
    egui::IconData { rgba, width: S as u32, height: S as u32 }
}
