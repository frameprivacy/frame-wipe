//! Frame dark palette, Geist typography, and 8px spacing.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Style,
    TextStyle, Visuals,
};
use std::sync::Arc;

pub const GRID: f32 = 8.0;

pub const BG: Color32 = Color32::from_rgb(0, 0, 0);
pub const SURFACE: Color32 = Color32::from_rgb(8, 8, 9);
pub const SURFACE_RAISED: Color32 = Color32::from_rgb(16, 16, 18);
pub const SUNKEN: Color32 = Color32::from_rgb(7, 7, 8);
pub const BORDER: Color32 = Color32::from_rgb(38, 38, 40);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(58, 58, 62);
pub const TEXT: Color32 = Color32::from_rgb(255, 255, 255);
pub const STEEL_50: Color32 = Color32::from_rgb(231, 238, 252);
pub const STEEL_200: Color32 = Color32::from_rgb(185, 194, 216);
pub const STEEL_400: Color32 = Color32::from_rgb(140, 153, 180);
pub const STEEL_700: Color32 = Color32::from_rgb(63, 79, 106);
pub const CRITICAL: Color32 = Color32::from_rgb(207, 138, 130);
pub const CRITICAL_BG: Color32 = Color32::from_rgb(48, 24, 22);
pub const DANGER: Color32 = Color32::from_rgb(196, 54, 48);
pub const DANGER_HOVER: Color32 = Color32::from_rgb(216, 70, 62);
pub const WARNING: Color32 = Color32::from_rgb(200, 165, 113);
pub const WARNING_BG: Color32 = Color32::from_rgb(42, 34, 18);
pub const RESOLVED: Color32 = Color32::from_rgb(147, 181, 168);
#[allow(dead_code)]
pub const RESOLVED_BG: Color32 = Color32::from_rgb(18, 32, 28);

pub const RADIUS: f32 = 8.0;

pub const FONT_BODY: f32 = 14.5;
pub const FONT_SMALL: f32 = 12.0;
pub const FONT_TITLE: f32 = 20.0;
pub const FONT_DISPLAY: f32 = 36.0;
pub const FONT_HEAD: f32 = 16.0;
#[allow(dead_code)]
pub const FONT_LABEL: f32 = 12.0;

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "geist".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../fonts/Geist-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "geist-medium".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../fonts/Geist-Medium.ttf"
        ))),
    );
    fonts.font_data.insert(
        "geist-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../fonts/Geist-SemiBold.ttf"
        ))),
    );
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .expect("proportional")
        .insert(0, "geist".into());
    fonts
        .families
        .entry(FontFamily::Name("medium".into()))
        .or_default()
        .insert(0, "geist-medium".into());
    fonts
        .families
        .entry(FontFamily::Name("semibold".into()))
        .or_default()
        .insert(0, "geist-semibold".into());
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .expect("mono")
        .insert(0, "geist".into());
    ctx.set_fonts(fonts);
}

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn apply(ctx: &egui::Context) {
    let mut style = Style {
        visuals: visuals(),
        ..(*ctx.style()).clone()
    };
    style.spacing.item_spacing = egui::vec2(GRID, GRID);
    style.spacing.button_padding = egui::vec2(GRID * 1.5, 7.0);
    style.spacing.window_margin = egui::Margin::same(0);
    style.spacing.indent = GRID * 2.0;
    style.spacing.icon_width = 14.0;
    style.spacing.icon_width_inner = 10.0;
    style.spacing.interact_size = egui::vec2(GRID * 3.0, GRID * 3.5);
    style.spacing.combo_width = 160.0;
    style.visuals.window_corner_radius = CornerRadius::same(RADIUS as u8);
    style.visuals.menu_corner_radius = CornerRadius::same(RADIUS as u8);
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same((RADIUS / 2.0) as u8);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same((RADIUS / 2.0) as u8);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same((RADIUS / 2.0) as u8);
    style.visuals.widgets.active.corner_radius = CornerRadius::same((RADIUS / 2.0) as u8);
    style.visuals.widgets.open.corner_radius = CornerRadius::same((RADIUS / 2.0) as u8);

    let mut text = style.text_styles;
    text.insert(
        TextStyle::Body,
        FontId::new(FONT_BODY, FontFamily::Proportional),
    );
    text.insert(
        TextStyle::Small,
        FontId::new(FONT_SMALL, FontFamily::Proportional),
    );
    text.insert(TextStyle::Button, FontId::new(FONT_BODY, medium()));
    text.insert(TextStyle::Heading, FontId::new(FONT_TITLE, semibold()));
    text.insert(
        TextStyle::Monospace,
        FontId::new(13.0, FontFamily::Monospace),
    );
    style.text_styles = text;

    ctx.set_style(style);
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = SUNKEN;
    v.faint_bg_color = SURFACE_RAISED;
    v.code_bg_color = SURFACE_RAISED;
    v.hyperlink_color = STEEL_200;
    v.warn_fg_color = WARNING;
    v.error_fg_color = CRITICAL;
    v.selection.bg_fill = STEEL_700;
    v.selection.stroke = Stroke::new(1.0_f32, STEEL_400);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, STEEL_200);
    v.widgets.noninteractive.bg_fill = SURFACE;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.inactive.bg_fill = SURFACE_RAISED;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.bg_fill = Color32::from_rgb(24, 24, 28);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_STRONG);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.active.bg_fill = Color32::from_rgb(32, 32, 36);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, STEEL_400);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.open.bg_fill = SURFACE_RAISED;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, STEEL_400);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(RADIUS as u8))
        .inner_margin(egui::Margin::same(12))
}

pub fn raised_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE_RAISED)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(RADIUS as u8))
        .inner_margin(egui::Margin::same(12))
}
