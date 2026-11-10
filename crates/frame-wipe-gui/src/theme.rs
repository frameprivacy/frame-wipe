#![allow(dead_code)]
//! Design tokens for the Frame Wipe Liquid Glass UI.
//!
//! Light and dark palettes, materials, radii, spacing, type scale, and motion.
//! Appearance is stored in a thread-local so widgets can read `pal()` after
//! `apply` runs at the start of a frame.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke,
    Style, TextStyle, Visuals,
};
use std::cell::Cell;
use std::sync::Arc;

pub const GRID: f32 = 8.0;

pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_5: f32 = 20.0;
pub const SPACE_6: f32 = 24.0;
pub const SPACE_8: f32 = 32.0;

pub const RADIUS_SM: f32 = 8.0;
pub const RADIUS_MD: f32 = 12.0;
pub const RADIUS_LG: f32 = 18.0;
pub const RADIUS_XL: f32 = 26.0;
pub const RADIUS_PILL: f32 = 999.0;

pub const FONT_CAPTION: f32 = 11.0;
pub const FONT_FOOTNOTE: f32 = 13.0;
pub const FONT_SMALL: f32 = 13.0;
pub const FONT_BODY: f32 = 15.0;
pub const FONT_HEAD: f32 = 17.0;
pub const FONT_TITLE3: f32 = 20.0;
pub const FONT_TITLE: f32 = 22.0;
pub const FONT_TITLE1: f32 = 28.0;
pub const FONT_DISPLAY: f32 = 44.0;

pub const MOTION_FAST: f32 = 0.15;
pub const MOTION_MED: f32 = 0.22;
pub const MOTION_SLOW: f32 = 0.32;

pub const SIDEBAR_WIDTH: f32 = 292.0;
pub const SIDEBAR_RAIL: f32 = 72.0;
pub const SIDEBAR_COLLAPSE_AT: f32 = 1000.0;
pub const TOOLBAR_H: f32 = 52.0;
pub const WINDOW_INSET: f32 = 12.0;
pub const PANE_GAP: f32 = 12.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Appearance {
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppearancePref {
    System,
    Light,
    Dark,
}

impl AppearancePref {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    pub fn resolve(self, ctx: &egui::Context) -> Appearance {
        match self {
            Self::Light => Appearance::Light,
            Self::Dark => Appearance::Dark,
            Self::System => match ctx.system_theme() {
                Some(egui::Theme::Light) => Appearance::Light,
                _ => Appearance::Dark,
            },
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub appearance: Appearance,
    pub wallpaper: Color32,
    pub orb_a: Color32,
    pub orb_b: Color32,
    pub orb_c: Color32,
    pub content_fill: Color32,
    pub glass: Color32,
    pub glass_strong: Color32,
    pub glass_clear: Color32,
    pub glass_stroke: Color32,
    pub glass_highlight: Color32,
    pub shadow: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    pub text_quaternary: Color32,
    pub accent: Color32,
    pub accent_fill: Color32,
    pub accent_soft: Color32,
    pub danger: Color32,
    pub danger_hover: Color32,
    pub danger_muted: Color32,
    pub danger_bg: Color32,
    pub danger_text: Color32,
    pub warning: Color32,
    pub warning_bg: Color32,
    pub success: Color32,
    pub success_bg: Color32,
    pub hover: Color32,
    pub press: Color32,
    pub selection: Color32,
    pub focus_ring: Color32,
    pub separator: Color32,
    pub track: Color32,
    pub thumb: Color32,
    pub badge_kind_fg: Color32,
    pub badge_kind_bg: Color32,
}

impl Palette {
    pub fn for_appearance(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Dark => Self::dark(),
            Appearance::Light => Self::light(),
        }
    }

    fn dark() -> Self {
        Self {
            appearance: Appearance::Dark,
            // Deep navy, not pure black, so frost and orbs have room.
            wallpaper: Color32::from_rgb(18, 20, 28),
            orb_a: Color32::from_rgba_unmultiplied(70, 110, 200, 70),
            orb_b: Color32::from_rgba_unmultiplied(140, 80, 180, 50),
            orb_c: Color32::from_rgba_unmultiplied(40, 150, 160, 40),
            content_fill: Color32::from_rgba_unmultiplied(28, 30, 38, 230),
            glass: Color32::from_rgba_unmultiplied(36, 38, 48, 200),
            glass_strong: Color32::from_rgba_unmultiplied(42, 44, 56, 230),
            glass_clear: Color32::from_rgba_unmultiplied(255, 255, 255, 18),
            glass_stroke: Color32::from_rgba_unmultiplied(255, 255, 255, 36),
            glass_highlight: Color32::from_rgba_unmultiplied(255, 255, 255, 48),
            shadow: Color32::from_black_alpha(80),
            text: Color32::from_rgb(245, 245, 247),
            // Stronger than 60% white so secondary copy stays readable on frost.
            text_secondary: Color32::from_rgb(198, 200, 210),
            text_tertiary: Color32::from_rgb(154, 158, 172),
            text_quaternary: Color32::from_rgb(110, 114, 128),
            accent: Color32::from_rgb(10, 132, 255),
            accent_fill: Color32::from_rgb(10, 132, 255),
            accent_soft: Color32::from_rgba_unmultiplied(10, 132, 255, 48),
            danger: Color32::from_rgb(255, 69, 58),
            danger_hover: Color32::from_rgb(255, 99, 88),
            danger_muted: Color32::from_rgba_unmultiplied(255, 69, 58, 70),
            danger_bg: Color32::from_rgba_unmultiplied(255, 69, 58, 48),
            danger_text: Color32::from_rgb(255, 255, 255),
            warning: Color32::from_rgb(255, 214, 10),
            warning_bg: Color32::from_rgba_unmultiplied(255, 214, 10, 48),
            success: Color32::from_rgb(48, 209, 88),
            success_bg: Color32::from_rgba_unmultiplied(48, 209, 88, 28),
            hover: Color32::from_rgba_unmultiplied(255, 255, 255, 18),
            press: Color32::from_rgba_unmultiplied(255, 255, 255, 28),
            selection: Color32::from_rgba_unmultiplied(10, 132, 255, 64),
            focus_ring: Color32::from_rgba_unmultiplied(10, 132, 255, 200),
            separator: Color32::from_rgba_unmultiplied(255, 255, 255, 22),
            track: Color32::from_rgba_unmultiplied(255, 255, 255, 22),
            thumb: Color32::from_rgb(245, 245, 247),
            badge_kind_fg: Color32::from_rgb(210, 220, 240),
            badge_kind_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 16),
        }
    }

    fn light() -> Self {
        Self {
            appearance: Appearance::Light,
            wallpaper: Color32::from_rgb(214, 222, 234),
            orb_a: Color32::from_rgba_unmultiplied(160, 196, 255, 110),
            orb_b: Color32::from_rgba_unmultiplied(220, 180, 240, 90),
            orb_c: Color32::from_rgba_unmultiplied(170, 230, 220, 80),
            content_fill: Color32::from_rgba_unmultiplied(252, 252, 254, 230),
            glass: Color32::from_rgba_unmultiplied(255, 255, 255, 188),
            glass_strong: Color32::from_rgba_unmultiplied(255, 255, 255, 230),
            glass_clear: Color32::from_rgba_unmultiplied(255, 255, 255, 140),
            glass_stroke: Color32::from_rgba_unmultiplied(0, 0, 0, 28),
            glass_highlight: Color32::from_rgba_unmultiplied(255, 255, 255, 220),
            shadow: Color32::from_black_alpha(36),
            text: Color32::from_rgb(28, 28, 30),
            text_secondary: Color32::from_rgb(58, 58, 62),
            text_tertiary: Color32::from_rgb(88, 88, 94),
            text_quaternary: Color32::from_rgb(122, 122, 128),
            accent: Color32::from_rgb(0, 113, 227),
            accent_fill: Color32::from_rgb(0, 113, 227),
            accent_soft: Color32::from_rgba_unmultiplied(0, 113, 227, 32),
            danger: Color32::from_rgb(215, 0, 21),
            danger_hover: Color32::from_rgb(196, 0, 18),
            danger_muted: Color32::from_rgba_unmultiplied(215, 0, 21, 50),
            danger_bg: Color32::from_rgba_unmultiplied(215, 0, 21, 18),
            danger_text: Color32::from_rgb(255, 255, 255),
            warning: Color32::from_rgb(162, 110, 0),
            warning_bg: Color32::from_rgba_unmultiplied(255, 204, 0, 48),
            success: Color32::from_rgb(0, 125, 50),
            success_bg: Color32::from_rgba_unmultiplied(48, 200, 88, 32),
            hover: Color32::from_rgba_unmultiplied(0, 0, 0, 10),
            press: Color32::from_rgba_unmultiplied(0, 0, 0, 18),
            selection: Color32::from_rgba_unmultiplied(0, 113, 227, 36),
            focus_ring: Color32::from_rgba_unmultiplied(0, 113, 227, 220),
            separator: Color32::from_rgba_unmultiplied(0, 0, 0, 22),
            track: Color32::from_rgba_unmultiplied(0, 0, 0, 16),
            thumb: Color32::from_rgb(255, 255, 255),
            badge_kind_fg: Color32::from_rgb(50, 60, 80),
            badge_kind_bg: Color32::from_rgba_unmultiplied(0, 0, 0, 10),
        }
    }
}

thread_local! {
    static CURRENT: Cell<Appearance> = const { Cell::new(Appearance::Dark) };
}

pub fn set_appearance(appearance: Appearance) {
    CURRENT.with(|c| c.set(appearance));
}

pub fn current_appearance() -> Appearance {
    CURRENT.with(|c| c.get())
}

pub fn pal() -> Palette {
    Palette::for_appearance(current_appearance())
}

// Compatibility aliases used by leftover call sites during the rebuild.
#[allow(dead_code)]
pub fn bg() -> Color32 {
    pal().wallpaper
}

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn lerp(current: f32, target: f32, dt: f32, duration: f32) -> f32 {
    if (current - target).abs() < 0.001 {
        return target;
    }
    let speed = if duration <= 0.0 {
        1.0
    } else {
        (dt / duration).clamp(0.0, 1.0)
    };
    let t = ease_out_cubic(speed.max(0.12));
    current + (target - current) * t
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let (regular, medium_bytes, semibold_bytes, family_name) = load_font_bytes();

    fonts.font_data.insert(
        family_name.to_string(),
        Arc::new(FontData::from_owned(regular)),
    );
    fonts.font_data.insert(
        format!("{family_name}-medium"),
        Arc::new(FontData::from_owned(medium_bytes)),
    );
    fonts.font_data.insert(
        format!("{family_name}-semibold"),
        Arc::new(FontData::from_owned(semibold_bytes)),
    );
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .expect("proportional")
        .insert(0, family_name.to_string());
    fonts
        .families
        .entry(FontFamily::Name("medium".into()))
        .or_default()
        .insert(0, format!("{family_name}-medium"));
    fonts
        .families
        .entry(FontFamily::Name("semibold".into()))
        .or_default()
        .insert(0, format!("{family_name}-semibold"));
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .expect("mono")
        .insert(0, family_name.to_string());
    ctx.set_fonts(fonts);
}

fn load_font_bytes() -> (Vec<u8>, Vec<u8>, Vec<u8>, &'static str) {
    if let Some(loaded) = try_system_sf() {
        return loaded;
    }
    (
        include_bytes!("../fonts/Geist-Regular.ttf").to_vec(),
        include_bytes!("../fonts/Geist-Medium.ttf").to_vec(),
        include_bytes!("../fonts/Geist-SemiBold.ttf").to_vec(),
        "geist",
    )
}

fn try_system_sf() -> Option<(Vec<u8>, Vec<u8>, Vec<u8>, &'static str)> {
    // SF Pro is not redistributed. On macOS we try well-known system paths.
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            (
                "/System/Library/Fonts/SFNS.ttf",
                "/System/Library/Fonts/SFNS.ttf",
                "/System/Library/Fonts/SFNS.ttf",
            ),
            (
                "/System/Library/Fonts/SFCompact.ttf",
                "/System/Library/Fonts/SFCompact.ttf",
                "/System/Library/Fonts/SFCompact.ttf",
            ),
        ];
        for (r, m, s) in candidates {
            if let (Ok(a), Ok(b), Ok(c)) = (std::fs::read(r), std::fs::read(m), std::fs::read(s)) {
                if !a.is_empty() {
                    return Some((a, b, c, "sf"));
                }
            }
        }
    }
    None
}

pub fn apply(ctx: &egui::Context, appearance: Appearance) {
    set_appearance(appearance);
    let p = Palette::for_appearance(appearance);
    let mut style = Style {
        visuals: visuals(&p),
        ..(*ctx.style()).clone()
    };
    style.spacing.item_spacing = egui::vec2(GRID, GRID);
    style.spacing.button_padding = egui::vec2(SPACE_3, 7.0);
    style.spacing.window_margin = egui::Margin::same(0);
    style.spacing.indent = SPACE_4;
    style.spacing.icon_width = 16.0;
    style.spacing.icon_width_inner = 12.0;
    style.spacing.interact_size = egui::vec2(SPACE_4, 28.0);
    style.spacing.combo_width = 160.0;
    style.visuals.window_corner_radius = CornerRadius::same(RADIUS_XL as u8);
    style.visuals.menu_corner_radius = CornerRadius::same(RADIUS_MD as u8);
    style.visuals.window_shadow = Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: p.shadow,
    };
    let r = RADIUS_MD as u8;
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(r);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(r);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(r);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(r);
    style.visuals.widgets.open.corner_radius = CornerRadius::same(r);

    let mut text = style.text_styles;
    text.insert(
        TextStyle::Body,
        FontId::new(FONT_BODY, FontFamily::Proportional),
    );
    text.insert(
        TextStyle::Small,
        FontId::new(FONT_FOOTNOTE, FontFamily::Proportional),
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

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.appearance == Appearance::Dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.dark_mode = p.appearance == Appearance::Dark;
    v.override_text_color = Some(p.text);
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = p.glass_strong;
    v.extreme_bg_color = p.track;
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.content_fill;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text_secondary);
    v.widgets.noninteractive.bg_fill = p.glass;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.glass_stroke);
    v.widgets.inactive.bg_fill = p.glass;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.glass_stroke);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.glass_highlight);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    v.widgets.active.bg_fill = p.press;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, p.text);
    v.widgets.open.bg_fill = p.glass_strong;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, p.accent);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, p.text);
    v
}

#[allow(dead_code)]
pub fn glass_frame() -> egui::Frame {
    let p = pal();
    egui::Frame::new()
        .fill(p.glass)
        .stroke(Stroke::new(1.0_f32, p.glass_stroke))
        .corner_radius(CornerRadius::same(RADIUS_LG as u8))
        .shadow(Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: p.shadow,
        })
        .inner_margin(egui::Margin::same(SPACE_4 as i8))
}

pub fn content_frame() -> egui::Frame {
    let p = pal();
    egui::Frame::new()
        .fill(p.content_fill)
        .stroke(Stroke::new(1.0_f32, p.glass_stroke))
        .corner_radius(CornerRadius::same(RADIUS_LG as u8))
        .shadow(Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: p.shadow,
        })
        .inner_margin(egui::Margin::same(SPACE_4 as i8))
}

pub fn sheet_frame() -> egui::Frame {
    let p = pal();
    egui::Frame::new()
        .fill(p.glass_strong)
        .stroke(Stroke::new(1.0_f32, p.glass_stroke))
        .corner_radius(CornerRadius::same(RADIUS_XL as u8))
        .shadow(Shadow {
            offset: [0, 18],
            blur: 48,
            spread: 0,
            color: p.shadow,
        })
        .inner_margin(egui::Margin::same(SPACE_6 as i8))
}

#[allow(dead_code)]
pub fn card_frame() -> egui::Frame {
    content_frame()
}

#[allow(dead_code)]
pub fn raised_frame() -> egui::Frame {
    glass_frame()
}

/// Soft radial orbs that stand in for a desktop behind emulated glass.
pub fn paint_wallpaper(painter: &egui::Painter, rect: egui::Rect) {
    let p = pal();
    painter.rect_filled(rect, 0.0, p.wallpaper);
    paint_orb(
        painter,
        rect.lerp_inside(egui::vec2(0.18, 0.12)),
        rect.width() * 0.55,
        p.orb_a,
    );
    paint_orb(
        painter,
        rect.lerp_inside(egui::vec2(0.86, 0.22)),
        rect.width() * 0.48,
        p.orb_b,
    );
    paint_orb(
        painter,
        rect.lerp_inside(egui::vec2(0.55, 0.92)),
        rect.width() * 0.58,
        p.orb_c,
    );
    // Soft vignette so edges of floating chrome stay readable.
    let edge = Color32::from_black_alpha(if p.appearance == Appearance::Dark {
        40
    } else {
        12
    });
    painter.rect(
        rect,
        0.0,
        Color32::TRANSPARENT,
        Stroke::new(48.0_f32, edge),
        egui::StrokeKind::Inside,
    );
}

fn paint_orb(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    // Concentric fades approximate a gaussian without a custom shader.
    let steps = 14;
    for i in 0..steps {
        let t = (i as f32 + 1.0) / steps as f32;
        let r = radius * (1.0 - t * 0.08);
        let a = (color.a() as f32 * (1.0 - t).powf(1.6) / steps as f32 * 6.0) as u8;
        let c = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a);
        painter.circle_filled(center, r, c);
    }
}

pub fn paint_glass_highlight(painter: &egui::Painter, rect: egui::Rect, radius: f32) {
    let p = pal();
    let top = rect.shrink2(egui::vec2(radius * 0.4, 1.0));
    let y = rect.top() + 1.0;
    painter.line_segment(
        [egui::pos2(top.left(), y), egui::pos2(top.right(), y)],
        Stroke::new(1.0_f32, p.glass_highlight),
    );
}
