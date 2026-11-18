//! The image area: zoom to cursor, middle-button pan, fit, compare divider, picking, brushes.

use super::{tex_options, App, Pick, Tool};
use eframe::egui::{self, pos2, vec2, Color32, ColorImage, PointerButton, Rect, Sense, Stroke};
use pixlflow::{brush, settings::PathKind};
use std::sync::Arc;

pub struct View {
    /// Screen pixels per image pixel.
    zoom: f32,
    /// Position of the image's top-left corner inside the view.
    offset: egui::Vec2,
    pub fit_pending: bool,
    /// Compare divider, as a fraction of the view width.
    divider: f32,
}

impl Default for View {
    fn default() -> Self {
        View { zoom: 1.0, offset: egui::Vec2::ZERO, fit_pending: true, divider: 0.5 }
    }
}

impl App {
    pub(super) fn image_view(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(24));
        let Some(doc) = &self.doc else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Open an image (Ctrl+O) or drop a file here",
                egui::FontId::proportional(18.0),
                Color32::GRAY,
            );
            return;
        };
        let size = vec2(doc.w as f32, doc.h as f32);
        let fit = (rect.width() / size.x).min(rect.height() / size.y);
        let v = &mut self.view;
        if v.fit_pending || resp.double_clicked() {
            v.zoom = fit;
            v.offset = (rect.size() - size * fit) / 2.0;
            v.fit_pending = false;
        }
        if let Some(p) = resp.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                let zoom = (v.zoom * (scroll * 0.002).exp()).clamp(fit.min(1.0), 64.0);
                let local = p - rect.min;
                v.offset = local - (local - v.offset) * (zoom / v.zoom); // keep the point under the cursor
                v.zoom = zoom;
            }
        }
        if resp.dragged_by(PointerButton::Middle) {
            v.offset += resp.drag_delta();
        }
        let zoom = v.zoom;
        let img = Rect::from_min_size(rect.min + v.offset, size * zoom);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let shown = if self.show_original { doc.orig_tex.id() } else { doc.result_tex.id() };

        if self.compare {
            if resp.dragged_by(PointerButton::Primary) && self.pick.is_none() && !self.brush.on {
                if let Some(p) = resp.interact_pointer_pos() {
                    v.divider = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                }
            }
            let x = rect.left() + v.divider * rect.width();
            let (left, right) = rect.split_left_right_at_x(x);
            painter.with_clip_rect(left).image(doc.orig_tex.id(), img, uv, Color32::WHITE);
            painter.with_clip_rect(right).image(shown, img, uv, Color32::WHITE);
            painter.vline(x, rect.y_range(), Stroke::new(2.0, Color32::WHITE));
            painter.circle_filled(pos2(x, rect.center().y), 6.0, Color32::WHITE);
        } else {
            painter.image(shown, img, uv, Color32::WHITE);
        }

        let to_image = |p: egui::Pos2| ((p.x - img.min.x) / zoom, (p.y - img.min.y) / zoom);
        if let Some(pick) = self.pick {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if resp.clicked() {
                if let Some((x, y)) = resp.interact_pointer_pos().map(to_image) {
                    if x >= 0.0 && y >= 0.0 && x < size.x && y < size.y {
                        let target = doc.src[y as usize * doc.w + x as usize];
                        let s = &mut self.layers[self.cur].s;
                        match pick {
                            Pick::Center => (s.cx, s.cy) = (x / size.x, y / size.y),
                            Pick::Color => s.target = target,
                        }
                    }
                }
                self.pick = None;
            }
        } else if self.brush.on {
            let dirty = self.paint(ui, &resp, to_image);
            self.draw_overlay(ui.ctx(), &painter, img, dirty);
            if let Some(p) = resp.hover_pos() {
                painter.circle_stroke(p, self.brush.radius * zoom, Stroke::new(1.0, Color32::WHITE));
                painter.circle_stroke(p, self.brush.radius * zoom + 1.0, Stroke::new(1.0, Color32::BLACK));
            }
        }
    }

    /// Applies the brush for this frame; returns the changed area.
    fn paint(&mut self, ui: &egui::Ui, resp: &egui::Response, to_image: impl Fn(egui::Pos2) -> (f32, f32)) -> Option<brush::Area> {
        let (pos, primary, secondary) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down(), i.pointer.secondary_down()));
        let b = &mut self.brush;
        if !resp.is_pointer_button_down_on() || !(primary || secondary) {
            b.last = None;
            return None;
        }
        let (doc, p) = (self.doc.as_ref()?, to_image(pos?));
        let (w, h) = (doc.w, doc.h);
        if b.last.is_some_and(|l| (l.0 - p.0).hypot(l.1 - p.1) < 0.5) {
            return None; // not moved
        }
        let points = match b.last {
            Some(last) => brush::stroke(last, p, (b.radius * 0.25).max(1.0)),
            None => vec![p],
        };
        let erase = secondary != b.erase;
        let layer = &mut self.layers[self.cur];
        let mut dirty: Option<brush::Area> = None;
        let mut grow = |a: Option<brush::Area>| {
            if let Some(a) = a {
                dirty = Some(dirty.map_or(a, |d| [d[0].min(a[0]), d[1].min(a[1]), d[2].max(a[2]), d[3].max(a[3])]));
            }
        };
        match b.tool {
            Tool::Mask => {
                // a new mask starts empty when painting and full when erasing, so the first stroke shows
                let m = Arc::make_mut(layer.mask.get_or_insert_with(|| Arc::new(vec![if erase { 255 } else { 0 }; w * h])));
                for q in points {
                    grow(brush::stamp_mask(m, w, h, q.0, q.1, b.radius, b.hardness, erase));
                }
            }
            Tool::Directions => {
                // the angle comes from the mouse movement, so a stroke starts drawing on its second point
                let angle = b.last.map(|l| (p.1 - l.1).atan2(p.0 - l.0));
                if erase || angle.is_some() {
                    let d = Arc::make_mut(layer.dirs.get_or_insert_with(|| Arc::new(vec![f32::NAN; w * h])));
                    for q in points {
                        grow(brush::stamp_dirs(d, w, h, q.0, q.1, b.radius, if erase { None } else { angle }));
                    }
                    if !erase {
                        layer.s.path = PathKind::Drawn;
                    }
                }
            }
        }
        b.last = Some(p);
        dirty
    }

    /// Mask as translucent red (where sorted), directions coloured by angle. Uploads only the changed area.
    fn draw_overlay(&mut self, ctx: &egui::Context, painter: &egui::Painter, img: Rect, dirty: Option<brush::Area>) {
        let tool = self.brush.tool;
        let l = &self.layers[self.cur];
        let Some(doc) = &mut self.doc else { return };
        let (w, h) = (doc.w, doc.h);
        let ptr = match tool {
            Tool::Mask => l.mask.as_ref().map(|m| Arc::as_ptr(m) as usize),
            Tool::Directions => l.dirs.as_ref().map(|d| Arc::as_ptr(d) as usize),
        };
        let Some(ptr) = ptr else { return };
        let key = (self.cur, tool, ptr);
        let pixel = |i: usize| match tool {
            Tool::Mask => Color32::from_rgba_unmultiplied(255, 40, 40, (l.mask.as_ref().unwrap()[i] as f32 * 0.45) as u8),
            Tool::Directions => {
                let a = l.dirs.as_ref().unwrap()[i];
                if a.is_nan() {
                    Color32::TRANSPARENT
                } else {
                    egui::ecolor::Hsva::new((a / std::f32::consts::TAU).rem_euclid(1.0), 0.9, 1.0, 0.6).into()
                }
            }
        };
        let region = |a: brush::Area| {
            let pixels = (a[1]..a[3]).flat_map(|y| (a[0]..a[2]).map(move |x| y * w + x)).map(&pixel).collect();
            ColorImage::new([a[2] - a[0], a[3] - a[1]], pixels)
        };
        match &mut doc.overlay {
            Some((tex, k)) if *k == key => {
                if let Some(a) = dirty {
                    tex.set_partial([a[0], a[1]], region(a), tex_options());
                }
            }
            slot => *slot = Some((ctx.load_texture("overlay", region([0, 0, w, h]), tex_options()), key)),
        }
        if let Some((tex, _)) = &doc.overlay {
            painter.image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    }
}
