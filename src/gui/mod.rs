//! The window: settings panel on the left, image on the right, status bar below.

mod panel;
mod view;
mod worker;

use eframe::egui::{self, Color32, ColorImage, Key, Modifiers, TextureHandle, TextureOptions};
use pixlflow::{engine, imageio, settings::Settings, Rgb};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use worker::{Done, Job, Kind, Worker};

const PANEL_WIDTH: f32 = 380.0;
const PREVIEW_SIDE: usize = 1000;
const UNDO_STEPS: usize = 40;
const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

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
    guide: Option<Arc<Vec<u8>>>,
    guide_name: Option<String>,
    orig_tex: TextureHandle,
    result_tex: TextureHandle,
    /// Last full-size result and the settings that made it.
    result: Option<(Settings, Vec<Rgb>)>,
}

struct Small {
    w: usize,
    h: usize,
    k: f32,
    src: Arc<Vec<Rgb>>,
    guide: Option<Arc<Vec<u8>>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Center,
    Color,
}

pub struct App {
    s: Settings,
    committed: Settings,
    undo: Vec<Settings>,
    redo: Vec<Settings>,
    doc: Option<Doc>,
    worker: Worker,
    next_id: u64,
    shown_id: u64,
    /// Last request sent: (settings, kind).
    requested: Option<(Settings, Kind)>,
    sort_now: bool,
    auto_sort: bool,
    background: bool,
    fast_preview: bool,
    compare: bool,
    show_mask: bool,
    pick: Option<Pick>,
    view: view::View,
    out_dir: Option<PathBuf>,
    /// Output path given on the command line: Save writes exactly there.
    fixed_output: Option<PathBuf>,
    sort_ms: Option<u128>,
    saved_at: Option<Instant>,
    message: Option<String>,
}

impl App {
    fn new(ctx: egui::Context, s: Settings, fixed_output: Option<PathBuf>) -> Self {
        App {
            committed: s.clone(),
            s,
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
            pick: None,
            view: view::View::default(),
            out_dir: None,
            fixed_output,
            sort_ms: None,
            saved_at: None,
            message: None,
        }
    }

    fn open(&mut self, ctx: &egui::Context, path: &Path) {
        let pic = match imageio::load(path) {
            Ok(p) => p,
            Err(e) => return self.message = Some(e),
        };
        if pic.deep {
            self.message = Some("The image has more than 8 bits per channel; it was reduced to 8 bits.".into());
        }
        let (w, h) = (pic.w, pic.h);
        let long = w.max(h);
        let small = (long > PREVIEW_SIDE).then(|| {
            let k = PREVIEW_SIDE as f32 / long as f32;
            let (sw, sh) = (((w as f32 * k).round() as usize).max(1), ((h as f32 * k).round() as usize).max(1));
            Small { w: sw, h: sh, k, src: Arc::new(imageio::resize_rgb(&pic.rgb, w, h, sw, sh)), guide: None }
        });
        let image = color_image(w, h, &pic.rgb, pic.alpha.as_deref());
        self.doc = Some(Doc {
            path: path.to_path_buf(),
            w,
            h,
            orig_tex: ctx.load_texture("original", image.clone(), tex_options()),
            result_tex: ctx.load_texture("result", image, tex_options()),
            src: Arc::new(pic.rgb),
            alpha: pic.alpha,
            small,
            guide: None,
            guide_name: None,
            result: None,
        });
        self.shown_id = self.next_id; // drop results still coming for the previous image
        self.requested = None;
        self.view.fit_pending = true;
    }

    fn load_guide(&mut self, path: &Path) {
        let Some(doc) = &mut self.doc else { return };
        match imageio::load_gray(path, doc.w, doc.h, true) {
            Ok(g) => {
                if let Some(sm) = &mut doc.small {
                    sm.guide = Some(Arc::new(imageio::resize_gray(&g, doc.w, doc.h, sm.w, sm.h)));
                }
                doc.guide = Some(Arc::new(g));
                doc.guide_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                self.invalidate();
            }
            Err(e) => self.message = Some(e),
        }
    }

    fn clear_guide(&mut self) {
        if let Some(doc) = &mut self.doc {
            doc.guide = None;
            doc.guide_name = None;
            if let Some(sm) = &mut doc.small {
                sm.guide = None;
            }
            self.invalidate();
        }
    }

    /// Inputs other than settings changed: the next frame re-sorts.
    fn invalidate(&mut self) {
        self.requested = None;
        self.sort_now = self.auto_sort;
        if let Some(doc) = &mut self.doc {
            doc.result = None;
        }
    }

    // ---- sorting ----

    fn request_sort(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        let dragging = ctx.input(|i| i.pointer.any_down());
        let (last_s, last_kind) = match &self.requested {
            Some((s, k)) => (Some(s), Some(*k)),
            None => (None, None),
        };
        let changed = last_s != Some(&self.s);
        let kind = if self.show_mask {
            Kind::Mask
        } else if dragging && self.fast_preview && doc.small.is_some() {
            Kind::Preview
        } else {
            Kind::Full
        };
        let need = self.sort_now
            || match kind {
                Kind::Mask => changed || last_kind != Some(Kind::Mask),
                Kind::Preview => changed && self.auto_sort,
                Kind::Full => (changed && self.auto_sort) || last_kind.is_some_and(|k| k != Kind::Full),
            };
        if !need {
            return;
        }
        self.sort_now = false;
        self.next_id += 1;
        let job = match (&doc.small, kind) {
            (Some(sm), Kind::Preview) => Job {
                id: self.next_id,
                kind,
                settings: self.s.scaled(sm.k),
                w: sm.w,
                h: sm.h,
                src: sm.src.clone(),
                guide: sm.guide.clone(),
            },
            _ => Job {
                id: self.next_id,
                kind,
                settings: self.s.clone(),
                w: doc.w,
                h: doc.h,
                src: doc.src.clone(),
                guide: doc.guide.clone(),
            },
        };
        self.requested = Some((self.s.clone(), kind));
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
        let Some(doc) = &mut self.doc else { return };
        if d.id <= self.shown_id {
            return; // older than what is on screen, or for a previous image
        }
        self.shown_id = d.id;
        match d.px {
            Err(e) => self.message = Some(e),
            Ok(px) => {
                let alpha = if d.kind == Kind::Full { doc.alpha.as_deref() } else { None };
                doc.result_tex.set(color_image(d.w, d.h, &px, alpha), tex_options());
                if d.kind != Kind::Mask {
                    self.sort_ms = Some(d.ms);
                }
                if d.kind == Kind::Full {
                    doc.result = Some((d.settings, px));
                }
            }
        }
    }

    /// Full-size result for the current settings, sorting now if the last one is stale.
    fn full_result(&mut self) -> Option<Vec<Rgb>> {
        let doc = self.doc.as_mut()?;
        if doc.result.as_ref().is_none_or(|(s, _)| *s != self.s) {
            let ex = engine::Extras { guide: doc.guide.as_ref().map(|g| g.as_slice()), ..Default::default() };
            doc.result = Some((self.s.clone(), engine::render(&self.s, doc.w, doc.h, &doc.src, &ex)));
        }
        doc.result.as_ref().map(|(_, px)| px.clone())
    }

    // ---- files ----

    fn open_dialog(&mut self, ctx: &egui::Context) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Images", &IMAGE_EXTS).pick_file() {
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
                let s = &self.s;
                let name = format!("{stem}-{}-{}-{}-{}", s.path.name(), s.select.name(), s.by.name(), s.algo.name());
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
        let Some(px) = self.full_result() else { return };
        let doc = self.doc.as_ref().expect("full_result needs a doc");
        match imageio::save(path, doc.w, doc.h, &px, doc.alpha.as_deref()) {
            Ok(()) => self.saved_at = Some(Instant::now()),
            Err(e) => self.message = Some(e),
        }
    }

    // ---- undo ----

    /// Records a step once the mouse is up, so a whole slider drag is one step.
    fn commit_undo(&mut self, ctx: &egui::Context) {
        if self.s == self.committed || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        self.undo.push(std::mem::replace(&mut self.committed, self.s.clone()));
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.s, prev.clone()));
            self.committed = prev;
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.s, next.clone()));
            self.committed = next;
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
        let [open, save_as, save, redo, undo, fit, mask] = ctx.input_mut(|i| {
            [
                i.consume_key(cmd, Key::O),
                i.consume_key(cmd_shift, Key::S),
                i.consume_key(cmd, Key::S),
                i.consume_key(cmd_shift, Key::Z) || i.consume_key(cmd, Key::Y),
                i.consume_key(cmd, Key::Z),
                i.consume_key(Modifiers::NONE, Key::F),
                i.consume_key(Modifiers::NONE, Key::M),
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive();
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
