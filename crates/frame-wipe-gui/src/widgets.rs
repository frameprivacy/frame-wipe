//! Reusable Liquid Glass controls, cards, and SF-style vector icons.

use crate::theme::{
    self, pal, FONT_BODY, FONT_CAPTION, FONT_FOOTNOTE, FONT_HEAD, GRID, RADIUS_MD, RADIUS_PILL,
    RADIUS_SM, SPACE_2, SPACE_4,
};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};
use frame_wipe::planner::MediaKind;

pub fn truncated_label(ui: &mut Ui, text: &str, font: FontId, color: Color32) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text).font(font).color(color)).truncate())
        .on_hover_text(text)
}

pub fn paint_focus_ring(ui: &Ui, rect: Rect, radius: f32) {
    let p = pal();
    ui.painter().rect(
        rect.expand(3.0),
        CornerRadius::same((radius + 3.0) as u8),
        Color32::TRANSPARENT,
        Stroke::new(3.0_f32, p.focus_ring),
        StrokeKind::Outside,
    );
}

pub fn paint_inner_highlight(ui: &Ui, rect: Rect, radius: f32) {
    theme::paint_glass_highlight(ui.painter(), rect, radius);
}

/// Standard content-layer card. `inner` receives the card's usable width.
pub fn content_card(ui: &mut Ui, width: f32, inner: impl FnOnce(&mut Ui)) {
    let w = width.max(0.0);
    ui.allocate_ui_with_layout(
        Vec2::new(w, 0.0),
        egui::Layout::top_down(Align::Min),
        |ui| {
            ui.set_width(w);
            theme::content_frame().show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    inner(ui);
                });
            });
        },
    );
}

#[allow(dead_code)]
pub fn glass_card(ui: &mut Ui, width: f32, inner: impl FnOnce(&mut Ui)) {
    let w = width.max(0.0);
    ui.allocate_ui_with_layout(
        Vec2::new(w, 0.0),
        egui::Layout::top_down(Align::Min),
        |ui| {
            ui.set_width(w);
            theme::glass_frame().show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    inner(ui);
                });
            });
        },
    );
}

pub fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(RADIUS_PILL as u8))
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .font(FontId::new(FONT_CAPTION, theme::medium()))
                    .color(fg),
            );
        });
}

/// macOS-style segmented control with a sliding thumb.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, current: &mut T, options: &[(T, &str)]) -> bool {
    let p = pal();
    let n = options.len().max(1);
    let height = 28.0;
    let pad = 3.0;
    let item_w = options
        .iter()
        .map(|(_, label)| {
            ui.fonts(|f| {
                f.layout_no_wrap(
                    (*label).to_string(),
                    FontId::new(FONT_FOOTNOTE, theme::medium()),
                    p.text,
                )
                .size()
                .x
            }) + SPACE_4
        })
        .fold(0.0_f32, f32::max)
        .max(44.0);
    let total_w = item_w * n as f32 + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(total_w, height), Sense::click());
    ui.painter().rect(
        rect,
        CornerRadius::same(RADIUS_SM as u8),
        p.track,
        Stroke::new(1.0_f32, p.glass_stroke),
        StrokeKind::Inside,
    );

    let selected = options
        .iter()
        .position(|(v, _)| *v == *current)
        .unwrap_or(0);
    let inner = rect.shrink(pad);
    let thumb_w = inner.width() / n as f32;
    let thumb = Rect::from_min_size(
        Pos2::new(inner.left() + thumb_w * selected as f32, inner.top()),
        Vec2::new(thumb_w, inner.height()),
    );
    ui.painter().rect(
        thumb,
        CornerRadius::same((RADIUS_SM - 2.0) as u8),
        p.glass_strong,
        Stroke::new(1.0_f32, p.glass_highlight),
        StrokeKind::Inside,
    );
    // Dark mode thumb should be a raised glass chip, not washed-out white.
    if p.appearance == theme::Appearance::Dark {
        ui.painter().rect(
            thumb,
            CornerRadius::same((RADIUS_SM - 2.0) as u8),
            Color32::from_rgba_unmultiplied(255, 255, 255, 36),
            Stroke::new(1.0_f32, p.glass_highlight),
            StrokeKind::Inside,
        );
    }

    let mut changed = false;
    for (i, (value, label)) in options.iter().enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(inner.left() + thumb_w * i as f32, inner.top()),
            Vec2::new(thumb_w, inner.height()),
        );
        let on = *current == *value;
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            *label,
            FontId::new(FONT_FOOTNOTE, theme::medium()),
            if on { p.text } else { p.text_secondary },
        );
        let id = ui.id().with(("seg", i, *label));
        let item = ui.interact(r, id, Sense::click());
        if item.hovered() && !on {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if item.clicked() {
            *current = *value;
            changed = true;
        }
    }
    let _ = resp;
    changed
}

/// macOS switch. Returns true if the value changed.
pub fn toggle(ui: &mut Ui, value: &mut bool, label: &str) -> bool {
    let p = pal();
    let height = 22.0;
    let width = 40.0;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
        if resp.clicked() {
            *value = !*value;
            changed = true;
        }
        if resp.has_focus() {
            paint_focus_ring(ui, rect, height / 2.0);
        }
        let fill = if *value { p.accent } else { p.track };
        let fill = if resp.is_pointer_button_down_on() {
            if *value {
                p.accent.gamma_multiply(0.85)
            } else {
                p.press
            }
        } else {
            fill
        };
        ui.painter().rect(
            rect,
            CornerRadius::same((height / 2.0) as u8),
            fill,
            Stroke::new(1.0_f32, if *value { p.accent } else { p.glass_stroke }),
            StrokeKind::Inside,
        );
        let knob_d = height - 4.0;
        let knob_x = if *value {
            rect.right() - 2.0 - knob_d
        } else {
            rect.left() + 2.0
        };
        let knob = Rect::from_min_size(Pos2::new(knob_x, rect.top() + 2.0), Vec2::splat(knob_d));
        ui.painter().circle_filled(
            knob.center(),
            knob_d / 2.0,
            if resp.is_pointer_button_down_on() {
                p.glass_clear
            } else {
                Color32::WHITE
            },
        );
        ui.painter().circle_stroke(
            knob.center(),
            knob_d / 2.0,
            Stroke::new(0.5_f32, Color32::from_black_alpha(30)),
        );
        ui.add(egui::Label::new(
            RichText::new(label)
                .font(FontId::new(FONT_BODY, FontFamily::Proportional))
                .color(p.text),
        ));
    });
    changed
}

pub fn glass_button(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    let p = pal();
    let color = if enabled { p.text } else { p.text_tertiary };
    ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(label)
                .font(FontId::new(FONT_BODY, theme::medium()))
                .color(color),
        )
        .fill(p.glass)
        .stroke(Stroke::new(1.0_f32, p.glass_stroke))
        .corner_radius(CornerRadius::same(RADIUS_MD as u8))
        .min_size(Vec2::new(96.0, 32.0)),
    )
}

pub fn icon_tool_button(ui: &mut Ui, kind: Icon, tip: &str, enabled: bool) -> egui::Response {
    let p = pal();
    let size = Vec2::splat(32.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if !enabled {
        resp = resp.on_disabled_hover_text(tip);
    } else {
        resp = resp.on_hover_text(tip);
    }
    let hovered = resp.hovered() && enabled;
    let pressed = resp.is_pointer_button_down_on() && enabled;
    let fill = if pressed {
        p.press
    } else if hovered {
        p.hover
    } else {
        p.glass
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(RADIUS_SM as u8),
        fill,
        Stroke::new(1.0_f32, p.glass_stroke),
        StrokeKind::Inside,
    );
    let color = if enabled { p.text } else { p.text_tertiary };
    paint_icon(ui.painter(), kind, rect.shrink(7.0), color, 1.5);
    if resp.has_focus() {
        paint_focus_ring(ui, rect, RADIUS_SM);
    }
    if enabled {
        resp
    } else {
        resp.on_hover_cursor(egui::CursorIcon::NotAllowed)
    }
}

pub fn danger_button(ui: &mut Ui, label: &str, enabled: bool, width: f32) -> egui::Response {
    let p = pal();
    let h = 36.0;
    let w = width.max(160.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(Align::Center)),
        |ui| {
            ui.add_enabled(
                enabled,
                egui::Button::new(
                    RichText::new(label)
                        .font(FontId::new(FONT_BODY, theme::semibold()))
                        .color(if enabled {
                            p.danger_text
                        } else {
                            p.text_tertiary
                        }),
                )
                .fill(if enabled { p.danger } else { p.danger_muted })
                .stroke(Stroke::new(
                    1.0_f32,
                    if enabled {
                        p.danger_hover
                    } else {
                        p.glass_stroke
                    },
                ))
                .corner_radius(CornerRadius::same(RADIUS_MD as u8))
                .min_size(Vec2::new(w, h)),
            )
        },
    )
    .inner
}

pub fn progress_ring(ui: &mut Ui, fraction: f32, label: &str, sub: &str) {
    let p = pal();
    let frac = fraction.clamp(0.0, 1.0);
    let d = 168.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(d, d), Sense::hover());
    let center = rect.center();
    let radius = 74.0;
    let stroke_bg = 10.0_f32;
    ui.painter()
        .circle_stroke(center, radius, Stroke::new(stroke_bg, p.track));
    if frac > 0.001 {
        let steps = ((frac * 64.0).ceil() as usize).clamp(2, 64);
        let start = -std::f32::consts::FRAC_PI_2;
        let sweep = frac * std::f32::consts::TAU;
        let mut pts: Vec<Pos2> = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let a = start + sweep * (i as f32 / steps as f32);
            pts.push(Pos2::new(
                center.x + radius * a.cos(),
                center.y + radius * a.sin(),
            ));
        }
        ui.painter()
            .add(egui::Shape::line(pts, Stroke::new(stroke_bg, p.accent)));
    }
    ui.painter().text(
        center - Vec2::new(0.0, 10.0),
        Align2::CENTER_CENTER,
        label,
        FontId::new(theme::FONT_TITLE1, theme::semibold()),
        p.text,
    );
    ui.painter().text(
        center + Vec2::new(0.0, 18.0),
        Align2::CENTER_CENTER,
        sub,
        FontId::new(FONT_FOOTNOTE, theme::medium()),
        p.text_secondary,
    );
}

pub fn stat_tile(ui: &mut Ui, w: f32, h: f32, label: &str, value: &str) {
    let p = pal();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(RADIUS_MD as u8),
        p.content_fill,
        Stroke::new(1.0_f32, p.glass_stroke),
        StrokeKind::Inside,
    );
    paint_inner_highlight(ui, rect, RADIUS_MD);
    let pad = 12.0;
    ui.painter().text(
        Pos2::new(rect.left() + pad, rect.top() + 10.0),
        Align2::LEFT_TOP,
        label,
        FontId::new(FONT_CAPTION, theme::medium()),
        p.text_tertiary,
    );
    ui.painter().text(
        Pos2::new(rect.left() + pad, rect.top() + 26.0),
        Align2::LEFT_TOP,
        value,
        FontId::new(FONT_HEAD, theme::semibold()),
        p.text,
    );
}

pub fn stat_row(ui: &mut Ui, width: f32, items: &[(String, &str, Color32)]) {
    let n = items.len().max(1) as f32;
    let gap = GRID;
    let chip_w = ((width - gap * (n - 1.0)) / n).floor().max(80.0);
    let chip_h = 80.0;
    let p = pal();
    ui.allocate_ui_with_layout(
        Vec2::new(width, chip_h),
        egui::Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.set_height(chip_h);
            for (val, lab, col) in items {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(chip_w, chip_h), Sense::hover());
                ui.painter().rect(
                    rect,
                    CornerRadius::same(RADIUS_MD as u8),
                    p.content_fill,
                    Stroke::new(1.0_f32, p.glass_stroke),
                    StrokeKind::Inside,
                );
                let pad = 14.0;
                ui.painter().text(
                    Pos2::new(rect.left() + pad, rect.top() + pad),
                    Align2::LEFT_TOP,
                    val,
                    FontId::new(theme::FONT_TITLE, theme::semibold()),
                    *col,
                );
                ui.painter().text(
                    Pos2::new(rect.left() + pad, rect.top() + pad + 30.0),
                    Align2::LEFT_TOP,
                    *lab,
                    FontId::new(FONT_FOOTNOTE, FontFamily::Proportional),
                    p.text_tertiary,
                );
            }
        },
    );
}

pub fn metric_row(ui: &mut Ui, width: f32, items: &[(&str, String)]) {
    let gap = GRID;
    let per_row = if width >= 640.0 {
        items.len().max(1)
    } else {
        items.len().div_ceil(2).max(1)
    };
    let per_row = per_row.min(items.len().max(1));
    for chunk in items.chunks(per_row) {
        let cols = chunk.len().max(1) as f32;
        let chip_w = ((width - gap * (cols - 1.0)) / cols).floor().max(80.0);
        let chip_h = 64.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (lab, val) in chunk {
                stat_tile(ui, chip_w, chip_h, lab, val);
            }
        });
        ui.add_space(GRID);
    }
}

pub fn traffic_lights(ui: &mut Ui) {
    let colors = [
        Color32::from_rgb(255, 95, 86),
        Color32::from_rgb(255, 189, 46),
        Color32::from_rgb(39, 201, 63),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for c in colors {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 6.0, c);
            ui.painter().circle_stroke(
                rect.center(),
                6.0,
                Stroke::new(0.5_f32, Color32::from_black_alpha(40)),
            );
        }
    });
}

#[derive(Clone, Copy)]
pub enum Icon {
    Refresh,
    Sidebar,
    SidebarExpand,
    Drive(MediaKind),
    Warning,
    Success,
    Failure,
    Empty,
}

pub fn paint_icon(painter: &egui::Painter, kind: Icon, rect: Rect, color: Color32, stroke: f32) {
    let s = Stroke::new(stroke, color);
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    match kind {
        Icon::Refresh => {
            // Almost-complete circular arrows.
            let rad = r * 0.72;
            let start = -0.2_f32;
            let sweep = std::f32::consts::TAU * 0.72;
            let mut pts = Vec::new();
            let steps = 18;
            for i in 0..=steps {
                let a = start + sweep * (i as f32 / steps as f32);
                pts.push(Pos2::new(c.x + rad * a.cos(), c.y + rad * a.sin()));
            }
            painter.add(egui::Shape::line(pts, s));
            let tip = Pos2::new(
                c.x + rad * (start + sweep).cos(),
                c.y + rad * (start + sweep).sin(),
            );
            let ang = start + sweep;
            let left = Pos2::new(
                tip.x - r * 0.38 * (ang + 0.9).cos(),
                tip.y - r * 0.38 * (ang + 0.9).sin(),
            );
            let right = Pos2::new(
                tip.x - r * 0.38 * (ang - 0.9).cos(),
                tip.y - r * 0.38 * (ang - 0.9).sin(),
            );
            painter.line_segment([tip, left], s);
            painter.line_segment([tip, right], s);
        }
        Icon::Sidebar | Icon::SidebarExpand => {
            let body = rect.shrink(r * 0.15);
            painter.rect_stroke(body, CornerRadius::same(3), s, StrokeKind::Inside);
            let split = body.left() + body.width() * 0.38;
            painter.line_segment(
                [
                    Pos2::new(split, body.top()),
                    Pos2::new(split, body.bottom()),
                ],
                s,
            );
            if matches!(kind, Icon::Sidebar) {
                let fill_r = Rect::from_min_max(body.min, Pos2::new(split, body.bottom()));
                painter.rect_filled(
                    fill_r.shrink(1.0),
                    CornerRadius {
                        nw: 2,
                        sw: 2,
                        ne: 0,
                        se: 0,
                    },
                    color.gamma_multiply(0.35),
                );
            }
        }
        Icon::Warning => {
            let top = Pos2::new(c.x, rect.top() + r * 0.1);
            let bl = Pos2::new(rect.left() + r * 0.1, rect.bottom() - r * 0.1);
            let br = Pos2::new(rect.right() - r * 0.1, rect.bottom() - r * 0.1);
            painter.add(egui::Shape::closed_line(vec![top, br, bl], s));
            painter.line_segment(
                [
                    Pos2::new(c.x, c.y - r * 0.25),
                    Pos2::new(c.x, c.y + r * 0.15),
                ],
                s,
            );
            painter.circle_filled(Pos2::new(c.x, c.y + r * 0.42), stroke, color);
        }
        Icon::Success => {
            painter.circle_stroke(c, r * 0.85, s);
            painter.add(egui::Shape::line(
                vec![
                    Pos2::new(c.x - r * 0.38, c.y + 0.02 * r),
                    Pos2::new(c.x - r * 0.08, c.y + r * 0.32),
                    Pos2::new(c.x + r * 0.42, c.y - r * 0.28),
                ],
                s,
            ));
        }
        Icon::Failure => {
            painter.circle_stroke(c, r * 0.85, s);
            let d = r * 0.32;
            painter.line_segment(
                [Pos2::new(c.x - d, c.y - d), Pos2::new(c.x + d, c.y + d)],
                s,
            );
            painter.line_segment(
                [Pos2::new(c.x + d, c.y - d), Pos2::new(c.x - d, c.y + d)],
                s,
            );
        }
        Icon::Empty => {
            let body = Rect::from_center_size(c, Vec2::new(r * 1.4, r * 1.1));
            painter.rect_stroke(body, CornerRadius::same(3), s, StrokeKind::Inside);
            painter.line_segment(
                [
                    Pos2::new(body.left() + 3.0, body.top() + body.height() * 0.35),
                    Pos2::new(body.right() - 3.0, body.top() + body.height() * 0.35),
                ],
                s,
            );
        }
        Icon::Drive(kind) => paint_drive(painter, kind, rect, color, stroke),
    }
}

fn paint_drive(painter: &egui::Painter, kind: MediaKind, rect: Rect, color: Color32, stroke: f32) {
    let s = Stroke::new(stroke, color);
    let c = rect.center();
    let w = rect.width();
    let h = rect.height();
    match kind {
        MediaKind::Usb => {
            let body =
                Rect::from_center_size(c + Vec2::new(0.0, h * 0.08), Vec2::new(w * 0.62, h * 0.62));
            painter.rect_stroke(body, CornerRadius::same(2), s, StrokeKind::Inside);
            let neck = Rect::from_center_size(
                Pos2::new(c.x, body.top() - h * 0.12),
                Vec2::new(w * 0.28, h * 0.22),
            );
            painter.rect_stroke(neck, CornerRadius::ZERO, s, StrokeKind::Inside);
            painter.line_segment(
                [
                    Pos2::new(body.left() + 3.0, body.center().y),
                    Pos2::new(body.right() - 3.0, body.center().y),
                ],
                s,
            );
        }
        MediaKind::Hdd => {
            let body = Rect::from_center_size(c, Vec2::new(w * 0.86, h * 0.72));
            painter.rect_stroke(body, CornerRadius::same(3), s, StrokeKind::Inside);
            painter.circle_stroke(c, w * 0.18, s);
            painter.circle_filled(c, stroke, color);
        }
        MediaKind::Optical => {
            painter.circle_stroke(c, w * 0.42, s);
            painter.circle_stroke(c, w * 0.14, s);
        }
        MediaKind::Ram | MediaKind::Loop | MediaKind::Image | MediaKind::Virtual => {
            let a = Rect::from_center_size(c + Vec2::new(-2.0, 3.0), Vec2::new(w * 0.7, h * 0.48));
            let b = Rect::from_center_size(c + Vec2::new(2.0, -3.0), Vec2::new(w * 0.7, h * 0.48));
            painter.rect_stroke(a, CornerRadius::same(2), s, StrokeKind::Inside);
            painter.rect_stroke(b, CornerRadius::same(2), s, StrokeKind::Inside);
        }
        MediaKind::Nvme | MediaKind::Ssd | MediaKind::Unknown => {
            let body = Rect::from_center_size(c, Vec2::new(w * 0.86, h * 0.52));
            painter.rect_stroke(body, CornerRadius::same(2), s, StrokeKind::Inside);
            // Connector notches.
            for i in 0..3 {
                let x = body.left() + 4.0 + i as f32 * (body.width() - 8.0) / 3.0;
                painter.line_segment(
                    [
                        Pos2::new(x, body.bottom()),
                        Pos2::new(x + 4.0, body.bottom()),
                    ],
                    s,
                );
            }
        }
    }
}

pub fn drive_glyph(ui: &mut Ui, kind: MediaKind, selected: bool, protected: bool, size: f32) {
    let p = pal();
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let bg = if protected {
        p.badge_kind_bg
    } else if selected {
        p.accent_soft
    } else {
        p.badge_kind_bg
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(RADIUS_SM as u8),
        bg,
        Stroke::new(1.0_f32, p.glass_stroke),
        StrokeKind::Inside,
    );
    let color = if protected {
        p.text_tertiary
    } else if selected {
        p.accent
    } else {
        p.text_secondary
    };
    paint_icon(
        ui.painter(),
        Icon::Drive(kind),
        rect.shrink(size * 0.22),
        color,
        1.4,
    );
}

pub fn status_glyph(ui: &mut Ui, kind: Icon, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = pal();
    ui.painter()
        .circle_filled(rect.center(), size * 0.5, p.glass);
    ui.painter().circle_stroke(
        rect.center(),
        size * 0.5,
        Stroke::new(1.0_f32, p.glass_stroke),
    );
    paint_icon(ui.painter(), kind, rect.shrink(size * 0.28), color, 2.0);
}
