//! The image area: zoom to cursor, middle-button pan, fit, compare divider, picking.

use super::{App, Pick};
use eframe::egui::{self, pos2, vec2, Color32, PointerButton, Rect, Sense, Stroke};

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
        let img = Rect::from_min_size(rect.min + v.offset, size * v.zoom);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

        if self.compare {
            if resp.dragged_by(PointerButton::Primary) && self.pick.is_none() {
                if let Some(p) = resp.interact_pointer_pos() {
                    v.divider = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                }
            }
            let x = rect.left() + v.divider * rect.width();
            let (left, right) = rect.split_left_right_at_x(x);
            painter.with_clip_rect(left).image(doc.orig_tex.id(), img, uv, Color32::WHITE);
            painter.with_clip_rect(right).image(doc.result_tex.id(), img, uv, Color32::WHITE);
            painter.vline(x, rect.y_range(), Stroke::new(2.0, Color32::WHITE));
            painter.circle_filled(pos2(x, rect.center().y), 6.0, Color32::WHITE);
        } else {
            painter.image(doc.result_tex.id(), img, uv, Color32::WHITE);
        }

        if let Some(pick) = self.pick {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let ip = (p - img.min) / v.zoom;
                    if ip.x >= 0.0 && ip.y >= 0.0 && ip.x < size.x && ip.y < size.y {
                        match pick {
                            Pick::Center => (self.s.cx, self.s.cy) = (ip.x / size.x, ip.y / size.y),
                            Pick::Color => self.s.target = doc.src[ip.y as usize * doc.w + ip.x as usize],
                        }
                    }
                }
                self.pick = None;
            }
        }
    }
}
