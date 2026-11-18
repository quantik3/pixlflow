//! The settings panel (fixed width, vertical scroll only).

use super::{presets, App, MaskAction, Pick, Tool};
use eframe::egui::{self, pos2, vec2, Button, Color32, DragValue, Rect, RichText, Sense, Slider, Ui};
use pixlflow::{
    anim::{self, Ease},
    color,
    settings::{Algorithm, BlendMode, Channel, Combine, Criterion, PathKind as P, SelectorKind as K, Settings},
    Rgb,
};

impl App {
    pub(super) fn settings_panel(&mut self, ui: &mut Ui) {
        ui.spacing_mut().slider_width = 180.0;
        let ctx = ui.ctx().clone();
        let has_doc = self.doc.is_some();
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            if ui.button("Open…").on_hover_text("Ctrl+O").clicked() {
                self.open_dialog(&ctx);
            }
            if ui.add_enabled(has_doc, Button::new("Save")).on_hover_text("Ctrl+S").clicked() {
                self.save();
            }
            if ui.add_enabled(has_doc, Button::new("Save as…")).on_hover_text("Ctrl+Shift+S").clicked() {
                self.save_as();
            }
            if ui.add_enabled(has_doc, Button::new("Save project…")).on_hover_text("All layers, masks and directions").clicked() {
                self.save_project();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Save to:");
            let place = match (&self.fixed_output, &self.out_dir) {
                (Some(p), _) => p.display().to_string(),
                (None, Some(d)) => d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                (None, None) => "next to the image".into(),
            };
            ui.add(egui::Label::new(place).truncate());
            if self.fixed_output.is_none() {
                if ui.small_button("Folder…").clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        self.out_dir = Some(d);
                    }
                }
                if self.out_dir.is_some() && ui.small_button("Clear").on_hover_text("Next to the image").clicked() {
                    self.out_dir = None;
                }
            }
        });
        ui.separator();

        let sort = Button::new(RichText::new("SORT IMAGE").strong().size(16.0));
        if ui.add_enabled(has_doc, sort).clicked() {
            self.sort_now = true;
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.auto_sort, "Auto-sort");
            ui.checkbox(&mut self.background, "Background");
            ui.checkbox(&mut self.fast_preview, "Fast preview")
                .on_hover_text("Sort a smaller copy while a slider is dragged");
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.compare, "Compare");
            ui.checkbox(&mut self.show_mask, "Show mask").on_hover_text("M: white where the threshold selects");
            if ui.add_enabled(!self.undo.is_empty(), Button::new("Undo").small()).on_hover_text("Undo (Ctrl+Z)").clicked() {
                self.undo();
            }
            if ui.add_enabled(!self.redo.is_empty(), Button::new("Redo").small()).on_hover_text("Redo (Ctrl+Y)").clicked() {
                self.redo();
            }
        });
        ui.separator();
        if self.video.is_some() {
            self.video_section(ui);
        }
        self.layer_list(ui);
        ui.add_space(4.0);

        section(ui, "Layer", |ui| {
            let s = &mut self.layers[self.cur].s;
            row(ui, "Blend", |ui| combo(ui, "blend", &mut s.blend, BlendMode::ALL.iter().copied(), BlendMode::name));
            row(ui, "Opacity", |ui| {
                ui.add(
                    Slider::new(&mut s.opacity, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                        .custom_parser(|t| t.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
                )
            });
        });
        section(ui, "Path", |ui| self.path_rows(ui));
        section(ui, "Selection", |ui| selection_rows(ui, &mut self.layers[self.cur].s));
        section(ui, "Sorting", |ui| sorting_rows(ui, &mut self.layers[self.cur].s));
        section(ui, "Extras", |ui| self.extra_rows(ui));
        self.brush_section(ui);
        self.preset_section(ui);
        self.anim_section(ui);
    }

    fn preset_section(&mut self, ui: &mut Ui) {
        egui::CollapsingHeader::new(RichText::new("Presets").strong()).show(ui, |ui| {
            let picked = self.preset_pick.and_then(|i| self.presets.get(i)).map(|(n, _)| n.clone());
            let (mut apply, mut delete, mut save, mut random, mut paste) = (false, false, false, false, false);
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("presets").selected_text(picked.as_deref().unwrap_or("Saved presets")).width(170.0).show_ui(ui, |ui| {
                    for (i, (name, _)) in self.presets.iter().enumerate() {
                        ui.selectable_value(&mut self.preset_pick, Some(i), name);
                    }
                });
                apply = ui.add_enabled(picked.is_some(), Button::new("Apply")).clicked();
                delete = ui.add_enabled(picked.is_some(), Button::new("Delete")).clicked();
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.preset_name).hint_text("Name").desired_width(170.0));
                save = ui.add_enabled(!presets::clean_name(&self.preset_name).is_empty(), Button::new("Save")).on_hover_text("Same name replaces").clicked();
            });
            ui.horizontal(|ui| {
                if ui.button("Copy as text").clicked() {
                    ui.ctx().copy_text(self.layers[self.cur].s.to_preset());
                }
                random = ui.button("Random").on_hover_text("Random look for this layer").clicked();
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.preset_text).hint_text("Paste preset text").desired_width(170.0));
                paste = ui.add_enabled(!self.preset_text.trim().is_empty(), Button::new("Apply text")).clicked();
            });
            if apply {
                let text = self.presets[self.preset_pick.unwrap()].1.clone();
                self.apply_preset_text(&text);
            }
            if delete {
                self.delete_preset(self.preset_pick.unwrap());
            }
            if save {
                self.save_preset();
            }
            if paste {
                let text = std::mem::take(&mut self.preset_text);
                self.apply_preset_text(&text);
            }
            if random {
                let s = &mut self.layers[self.cur].s;
                // blending stays: random changes the look, not how the layer mixes in
                *s = Settings { blend: s.blend, opacity: s.opacity, ..Settings::random(&mut self.rng) };
            }
        });
    }

    fn anim_section(&mut self, ui: &mut Ui) {
        let mut export = None;
        egui::CollapsingHeader::new(RichText::new("Animation").strong()).show(ui, |ui| {
            let s = self.layers[self.cur].s.clone();
            let a = &mut self.anim;
            egui::Grid::new("anim").num_columns(2).spacing([10.0, 6.0]).min_col_width(80.0).show(ui, |ui| {
                for (label, slot) in [("Start", &mut a.start), ("End", &mut a.end)] {
                    row(ui, label, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Set").on_hover_text("Use this layer's current settings").clicked() {
                                *slot = Some(s.clone());
                            }
                            ui.label(if slot.is_some() { "set" } else { "not set" });
                        })
                    });
                }
                row(ui, "Frames", |ui| ui.add(Slider::new(&mut a.frames, 2..=1000).logarithmic(true)));
                row(ui, "FPS", |ui| ui.add(Slider::new(&mut a.fps, 1..=60)));
                row(ui, "Ping-pong", |ui| ui.checkbox(&mut a.ping_pong, "").on_hover_text("Forward, then back"));
                row(ui, "Speed", |ui| {
                    egui::ComboBox::from_id_salt("ease").selected_text(a.ease.name()).width(180.0).show_ui(ui, |ui| {
                        for e in Ease::ALL {
                            ui.selectable_value(&mut a.ease, e, e.name());
                        }
                    })
                });
                let ready = a.start.is_some() && a.end.is_some();
                let mut tried = false;
                row(ui, "Try", |ui| tried = ui.add_enabled(ready, Slider::new(&mut a.try_t, 0.0..=1.0)).changed());
                if tried {
                    if let (Some(st), Some(en)) = (&a.start, &a.end) {
                        self.layers[self.cur].s = anim::lerp(st, en, a.ease.apply(a.try_t));
                    }
                }
                row(ui, "GIF size", |ui| ui.add(Slider::new(&mut a.gif_side, 100..=2000).logarithmic(true).suffix(" px")).on_hover_text("Longest side"));
            });
            if !job_progress(ui, &self.job, None) {
                let ready = a.start.is_some() && a.end.is_some() && self.doc.is_some();
                ui.horizontal(|ui| {
                    if ui.add_enabled(ready, Button::new("Export GIF…")).clicked() {
                        export = Some(true);
                    }
                    if ui.add_enabled(ready, Button::new("Export PNG frames…")).clicked() {
                        export = Some(false);
                    }
                });
            }
        });
        if let Some(gif) = export {
            let ctx = ui.ctx().clone();
            self.start_export(&ctx, gif);
        }
    }

    fn video_section(&mut self, ui: &mut Ui) {
        let (mut seek, mut start) = (None, None);
        let anim_ready = self.anim.start.is_some() && self.anim.end.is_some();
        egui::CollapsingHeader::new(RichText::new("Video").strong()).default_open(true).show(ui, |ui| {
            let Some(v) = &mut self.video else { return };
            let i = &v.info;
            ui.label(format!("{} × {}, {} frames, {:.2} fps, {:.1} s", i.w, i.h, i.frames, i.fps, i.duration));
            ui.horizontal(|ui| {
                ui.label("Frame at").on_hover_text("The frame you edit with layers and masks");
                let r = ui.add(Slider::new(&mut v.t, 0.0..=i.duration.max(0.0)).suffix(" s").max_decimals(2));
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    seek = Some(v.t);
                }
            });
            ui.add_enabled(anim_ready, egui::Checkbox::new(&mut v.animate, "Animate this layer over the clip"))
                .on_hover_text("From Start to End (see Animation)");
            if !job_progress(ui, &self.job, Some(v.info.fps)) {
                ui.horizontal(|ui| {
                    if ui.button("Play sorted").on_hover_text("Watch in a player window, sorted live").clicked() {
                        start = Some(true);
                    }
                    if ui.button("Export video…").on_hover_text("All layers; sound is copied").clicked() {
                        start = Some(false);
                    }
                });
            }
        });
        if let Some(t) = seek {
            self.show_video_frame(t);
        }
        if let Some(play) = start {
            let ctx = ui.ctx().clone();
            self.start_video(&ctx, play);
        }
    }

    fn layer_list(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.strong("Layers");
            if ui.small_button("+ Add").clicked() {
                self.add_layer();
            }
            ui.toggle_value(&mut self.show_original, "Show original");
        });
        let mut delete = None;
        for i in (0..self.layers.len()).rev() {
            let s = &self.layers[i].s;
            let text = format!(
                "{}. {} · {} · {} {:.0}%",
                i + 1,
                pretty(s.path.name()),
                pretty(s.algo.name()),
                pretty(s.blend.name()),
                s.opacity * 100.0
            );
            let deletable = self.layers.len() > 1;
            ui.horizontal(|ui| {
                let r = ui.selectable_label(self.cur == i, text);
                if r.clicked() {
                    self.cur = i;
                }
                if deletable && (r.middle_clicked() || ui.small_button("X").on_hover_text("Delete (or middle click)").clicked()) {
                    delete = Some(i);
                }
            });
        }
        if let Some(i) = delete {
            self.delete_layer(i);
        }
    }

    fn brush_section(&mut self, ui: &mut Ui) {
        let mut action = None;
        egui::CollapsingHeader::new(RichText::new("Brush").strong()).default_open(true).show(ui, |ui| {
            let b = &mut self.brush;
            egui::Grid::new("brush").num_columns(2).spacing([10.0, 6.0]).min_col_width(80.0).show(ui, |ui| {
                row(ui, "Brush", |ui| {
                    ui.horizontal(|ui| {
                        ui.toggle_value(&mut b.on, "On (B)");
                        ui.selectable_value(&mut b.tool, Tool::Mask, "Mask");
                        ui.selectable_value(&mut b.tool, Tool::Directions, "Directions");
                    })
                });
                row(ui, "Radius", |ui| ui.add(Slider::new(&mut b.radius, 1.0..=1000.0).logarithmic(true).suffix(" px")));
                if b.tool == Tool::Mask {
                    row(ui, "Hardness", |ui| ui.add(Slider::new(&mut b.hardness, 0.0..=1.0)));
                }
                row(ui, "Erase", |ui| ui.checkbox(&mut b.erase, "").on_hover_text("Left button erases, right paints"));
            });
            let l = &self.layers[self.cur];
            if b.tool == Tool::Directions {
                ui.label(if l.dirs.is_some() { "Directions drawn on this layer." } else { "No directions drawn." });
                if ui.button("Clear directions").clicked() {
                    action = Some(None);
                }
                return;
            }
            ui.label(if l.mask.is_some() { "Mask: painted (red = sorted)." } else { "Mask: none, the whole image is sorted." });
            ui.horizontal_wrapped(|ui| {
                for (text, act, tip) in [
                    ("Fill", MaskAction::Fill, "Sort everywhere"),
                    ("Clear", MaskAction::Clear, "Sort nowhere"),
                    ("Invert", MaskAction::Invert, ""),
                    ("Remove", MaskAction::Remove, "Drop the mask: sort everywhere"),
                    ("From threshold", MaskAction::FromThreshold, "What Show mask shows"),
                    ("From guide", MaskAction::FromGuide, "Brightness of the guide image"),
                    ("Load…", MaskAction::Load, "Grey image, scaled to fit"),
                    ("Save…", MaskAction::Save, "As PNG"),
                ] {
                    let r = ui.add_enabled(self.doc.is_some(), Button::new(text));
                    let r = if tip.is_empty() { r } else { r.on_hover_text(tip) };
                    if r.clicked() {
                        action = Some(Some(act));
                    }
                }
            });
        });
        match action {
            Some(Some(act)) => self.mask_action(act),
            Some(None) => self.layer().dirs = None,
            None => {}
        }
    }

    fn path_rows(&mut self, ui: &mut Ui) {
        let s = &mut self.layers[self.cur].s;
        row(ui, "Shape", |ui| combo(ui, "path", &mut s.path, P::ALL.iter().copied(), P::name));
        row(ui, "Reverse", |ui| ui.checkbox(&mut s.reverse, ""));
        if matches!(s.path, P::Rays | P::Circles | P::Spiral | P::SquareSpiral | P::RectSpiral | P::Star | P::Swirls) {
            row(ui, "Center x", |ui| ui.add(Slider::new(&mut s.cx, 0.0..=1.0)));
            row(ui, "Center y", |ui| ui.add(Slider::new(&mut s.cy, 0.0..=1.0)));
            row(ui, "", |ui| pick_button(ui, &mut self.pick, Pick::Center, "Pick center on image"));
        }
        match s.path {
            P::Circles | P::Spiral => {
                row(ui, "Rotation", |ui| ui.add(Slider::new(&mut s.rotation, -180.0..=180.0).suffix("°")));
                row(ui, "Aspect", |ui| ui.add(Slider::new(&mut s.aspect, 0.2..=5.0).logarithmic(true)));
            }
            P::Diagonal => {
                row(ui, "Angle", |ui| ui.add(Slider::new(&mut s.angle, 0.0..=360.0).suffix("°")));
            }
            P::Waves => {
                row(ui, "Amplitude", |ui| ui.add(Slider::new(&mut s.amplitude, 0.0..=500.0).suffix(" px")));
                row(ui, "Wavelength", |ui| ui.add(Slider::new(&mut s.wavelength, 2.0..=2000.0).logarithmic(true).suffix(" px")));
                row(ui, "Vertical", |ui| ui.checkbox(&mut s.wave_vertical, ""));
            }
            P::Flow | P::Curl => {
                row(ui, "Swirl size", |ui| ui.add(Slider::new(&mut s.noise_size, 4.0..=2000.0).logarithmic(true).suffix(" px")));
                row(ui, "Seed", |ui| ui.add(DragValue::new(&mut s.seed)));
            }
            P::Swirls => {
                row(ui, "Points", |ui| ui.add(Slider::new(&mut s.swirls, 1..=32)));
                row(ui, "Seed", |ui| ui.add(DragValue::new(&mut s.seed)));
            }
            P::Contours => {
                row(ui, "Smoothness", |ui| ui.add(Slider::new(&mut s.contour_smooth, 0..=64).suffix(" px")));
            }
            P::Drawn => {
                row(ui, "", |ui| ui.label("Paint directions with the brush (B)."));
            }
            P::Star => {
                row(ui, "Rays", |ui| ui.add(Slider::new(&mut s.star_points, 2..=32)));
                row(ui, "Inner", |ui| ui.add(Slider::new(&mut s.star_inner, 0.0..=1.0)));
            }
            P::Tiles => {
                row(ui, "Tile size", |ui| ui.add(Slider::new(&mut s.tile, 2..=1024).logarithmic(true).suffix(" px")));
            }
            _ => {}
        }
    }

    fn extra_rows(&mut self, ui: &mut Ui) {
        let s = &mut self.layers[self.cur].s;
        row(ui, "Alternate", |ui| ui.checkbox(&mut s.alternate, "").on_hover_text("Reverse every second span"));
        row(ui, "Channel", |ui| combo(ui, "channel", &mut s.channel, Channel::ALL.iter().copied(), Channel::name));
        row(ui, "Posterize", |ui| ui.add(Slider::new(&mut s.posterize, 0..=32)).on_hover_text("Levels per channel, 0 = off"));
        row(ui, "Soft ends", |ui| ui.add(Slider::new(&mut s.soft_ends, 0..=200).suffix(" px")));
        row(ui, "Fringes", |ui| ui.add(Slider::new(&mut s.fringes, -50..=50).suffix(" px")));
        let threshold = matches!(s.select, K::Threshold | K::Combined);
        let distance = Criterion::ColorDistance;
        let uses_target = s.by == distance || (threshold && s.crit == distance) || (s.select == K::Combined && s.crit2 == distance);
        if uses_target {
            row(ui, "Target", |ui| {
                ui.horizontal(|ui| {
                    ui.color_edit_button_srgb(&mut s.target).on_hover_text("Color for the Distance criterion");
                    pick_button(ui, &mut self.pick, Pick::Color, "Pick on image");
                })
            });
        }
        let l = &self.layers[self.cur];
        let guide = l.guide.as_ref().map(|_| l.guide_name.clone().unwrap_or_default());
        let has_doc = self.doc.is_some();
        let (mut load, mut clear) = (false, false);
        row(ui, "Guide", |ui| {
            ui.horizontal(|ui| {
                load = ui.add_enabled(has_doc, Button::new("Load…")).clicked();
                if let Some(name) = &guide {
                    clear = ui.small_button("Clear").clicked();
                    ui.add(egui::Label::new(name.as_str()).truncate());
                }
            })
        });
        if load {
            self.guide_dialog();
        }
        if clear {
            let l = self.layer();
            (l.guide, l.guide_name) = (None, None);
        }
    }
}

fn selection_rows(ui: &mut Ui, s: &mut Settings) {
    row(ui, "Selector", |ui| combo(ui, "select", &mut s.select, K::ALL.iter().copied(), K::name));
    match s.select {
        K::Fixed => {
            row(ui, "Length", |ui| ui.add(Slider::new(&mut s.length, 0..=2000).logarithmic(true).suffix(" px")));
        }
        K::Random => {
            row(ui, "Max length", |ui| ui.add(Slider::new(&mut s.max_length, 0..=2000).logarithmic(true).suffix(" px")));
        }
        K::Threshold => condition(ui, "a", &mut s.crit, &mut s.min, &mut s.max, s.target),
        K::Combined => {
            condition(ui, "a", &mut s.crit, &mut s.min, &mut s.max, s.target);
            row(ui, "Join", |ui| combo(ui, "combine", &mut s.combine, Combine::ALL.iter().copied(), Combine::name));
            condition(ui, "b", &mut s.crit2, &mut s.min2, &mut s.max2, s.target);
        }
        K::Edges => {
            row(ui, "Edge", |ui| ui.add(Slider::new(&mut s.edge, 0..=765)).on_hover_text("Color jump that ends a span"));
        }
        K::Full => {}
    }
    if pixlflow::select::looks_at_color(s.select) {
        row(ui, "Smooth", |ui| ui.add(Slider::new(&mut s.smooth, 0..=50).suffix(" px")).on_hover_text("Blur used only to find spans"));
    }
}

fn condition(ui: &mut Ui, id: &str, crit: &mut Criterion, min: &mut u32, max: &mut u32, target: Rgb) {
    let all = Criterion::ALL.iter().copied().filter(|&c| c != Criterion::Guide);
    row(ui, "Criterion", |ui| combo(ui, &format!("crit-{id}"), crit, all, Criterion::name));
    let top = color::max_value(*crit) as u32;
    row(ui, "Min", |ui| ui.add(Slider::new(min, 0..=top)).on_hover_text("Min above max wraps around (e.g. hue 340–20 = red)"));
    row(ui, "Max", |ui| ui.add(Slider::new(max, 0..=top)));
    row(ui, "", |ui| color_strip(ui, *crit, target));
}

fn sorting_rows(ui: &mut Ui, s: &mut Settings) {
    row(ui, "Sort by", |ui| combo(ui, "by", &mut s.by, Criterion::ALL.iter().copied(), Criterion::name));
    row(ui, "Algorithm", |ui| combo(ui, "algo", &mut s.algo, Algorithm::ALL.iter().copied(), Algorithm::name));
    match s.algo {
        Algorithm::Partial => {
            row(ui, "Passes", |ui| ui.add(Slider::new(&mut s.passes, 1..=500).logarithmic(true)));
        }
        Algorithm::Stretch | Algorithm::Shift => {
            row(ui, "Strength", |ui| ui.add(Slider::new(&mut s.strength, 0.0..=1.0)));
        }
        Algorithm::Bucket => {
            row(ui, "Levels", |ui| ui.add(Slider::new(&mut s.buckets, 2..=64)));
        }
        _ => {}
    }
    if matches!(s.algo, Algorithm::Shuffle | Algorithm::Debug | Algorithm::Shift) || s.select == K::Random {
        row(ui, "Seed", |ui| ui.add(DragValue::new(&mut s.seed)));
    }
}

/// Progress bar with Stop for a running background job; false when there is none.
/// With `fps` (video) the bar also shows the time reached.
fn job_progress(ui: &mut Ui, job: &Option<super::export::Export>, fps: Option<f64>) -> bool {
    let Some(ex) = job else { return false };
    ui.horizontal(|ui| {
        let done = ex.done.load(std::sync::atomic::Ordering::Relaxed);
        let frac = if ex.total > 0 { done as f32 / ex.total as f32 } else { 0.0 };
        let mut text = format!("frame {done} / {}", ex.total);
        if let Some(fps) = fps.filter(|f| *f > 0.0) {
            text += &format!(" · at {:.1} s", done as f64 / fps); // position in the video
        }
        let secs = ex.started.elapsed().as_secs();
        text += &format!(" · {}:{:02} elapsed", secs / 60, secs % 60);
        ui.add(egui::ProgressBar::new(frac).text(text).desired_width(ui.available_width() - 60.0));
        if ui.button("Stop").clicked() {
            ex.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    true
}

fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(true).show(ui, |ui| {
        egui::Grid::new(title).num_columns(2).spacing([10.0, 6.0]).min_col_width(80.0).show(ui, add);
    });
}

fn row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) {
    ui.label(label);
    add(ui);
    ui.end_row();
}

/// "square-spiral" → "Square spiral".
fn pretty(name: &str) -> String {
    let s = name.replace('-', " ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn combo<T: Copy + PartialEq>(ui: &mut Ui, id: &str, value: &mut T, all: impl Iterator<Item = T>, name: fn(T) -> &'static str) {
    egui::ComboBox::from_id_salt(id).selected_text(pretty(name(*value))).width(180.0).show_ui(ui, |ui| {
        for v in all {
            ui.selectable_value(value, v, pretty(name(v)));
        }
    });
}

fn pick_button(ui: &mut Ui, pick: &mut Option<Pick>, which: Pick, text: &str) {
    let on = *pick == Some(which);
    if ui.add(Button::new(text).selected(on)).on_hover_text("Then click the image").clicked() {
        *pick = if on { None } else { Some(which) };
    }
}

/// Colour bar under the threshold sliders, drawn in code (no images).
fn color_strip(ui: &mut Ui, crit: Criterion, target: Rgb) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.spacing().slider_width, 8.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let max = color::max_value(crit) as f32;
    const N: usize = 90;
    for i in 0..N {
        let (t0, t1) = (i as f32 / N as f32, (i + 1) as f32 / N as f32);
        let r = Rect::from_min_max(pos2(rect.lerp_inside(vec2(t0, 0.0)).x, rect.top()), pos2(rect.lerp_inside(vec2(t1, 0.0)).x, rect.bottom()));
        painter.rect_filled(r, 0.0, sample_color(crit, (t0 + t1) / 2.0 * max, target));
    }
}

/// A colour whose `crit` value is about `v`.
fn sample_color(crit: Criterion, v: f32, target: Rgb) -> Color32 {
    let t = (v / 255.0).clamp(0.0, 1.0);
    let b = v.clamp(0.0, 255.0) as u8;
    let mix = |a: [f32; 3], c: [f32; 3]| Color32::from_rgb((a[0] + (c[0] - a[0]) * t) as u8, (a[1] + (c[1] - a[1]) * t) as u8, (a[2] + (c[2] - a[2]) * t) as u8);
    match crit {
        Criterion::Hue => egui::ecolor::Hsva::new(v / 360.0, 1.0, 1.0, 1.0).into(),
        Criterion::Red => Color32::from_rgb(b, 0, 0),
        Criterion::Green => Color32::from_rgb(0, b, 0),
        Criterion::Blue => Color32::from_rgb(0, 0, b),
        Criterion::Saturation => mix([255.0; 3], [255.0, 0.0, 0.0]),
        Criterion::Chroma => mix([128.0; 3], [255.0, 0.0, 0.0]),
        Criterion::Warmth => mix([0.0, 0.0, 255.0], [255.0, 0.0, 0.0]),
        Criterion::ColorDistance => {
            let far = target.map(|c| if c < 128 { 255.0 } else { 0.0 });
            mix(target.map(|c| c as f32), far)
        }
        Criterion::Brightness | Criterion::Lightness | Criterion::MinChannel | Criterion::MaxChannel | Criterion::Guide => {
            Color32::from_gray(b)
        }
    }
}
