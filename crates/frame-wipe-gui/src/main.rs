//! Desktop UI for framewipe. Native Rust (egui + glow). No Electron.

mod theme;
mod vibrancy;
mod widgets;

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, Vec2,
};
use frame_wipe::device::{self, DeviceInfo, EnumerateOptions};
use frame_wipe::engine::{wipe, WipeRequest};
use frame_wipe::i18n::{self, Lang};
use frame_wipe::identity::{confirmation_matches, DeviceIdentity};
use frame_wipe::method::Method;
use frame_wipe::planner::{MediaKind, Scope, TargetSnapshot};
use frame_wipe::progress::{Phase, Progress};
use frame_wipe::report::{format_elapsed_ms, Outcome, Report};
use frame_wipe::safety::SafetyFlags;
use frame_wipe::units::{format_bytes, format_eta, format_throughput};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use theme::{
    Appearance, AppearancePref, FONT_BODY, FONT_CAPTION, FONT_DISPLAY, FONT_FOOTNOTE, FONT_HEAD,
    FONT_SMALL, FONT_TITLE, FONT_TITLE3, GRID, PANE_GAP, RADIUS_PILL, SIDEBAR_COLLAPSE_AT,
    SIDEBAR_RAIL, SIDEBAR_WIDTH, TOOLBAR_H, WINDOW_INSET,
};
use widgets::{truncated_label, Icon};

fn main() -> eframe::Result<()> {
    let args = Args::parse();
    let scale = args.scale;
    let size = [args.width, args.height];
    let transparent = vibrancy::supported();
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_min_inner_size([800.0, 600.0])
            .with_title("Frame Wipe")
            .with_decorations(!args.undecorated)
            .with_transparent(transparent)
            .with_maximized(false),
        ..Default::default()
    };
    #[cfg(windows)]
    {
        options.renderer = eframe::Renderer::Wgpu;
    }
    let smoke = args.smoke;
    let args_fallback = args.clone();
    match eframe::run_native(
        "Frame Wipe",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            let appearance = args.appearance_pref.resolve(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx, appearance);
            cc.egui_ctx.set_pixels_per_point(scale);
            if vibrancy::supported() {
                vibrancy::apply(cc, appearance == Appearance::Dark);
            }
            Ok(Box::new(App::new(args, appearance)))
        }),
    ) {
        Ok(()) => Ok(()),
        Err(err) if smoke => {
            eprintln!("native window failed: {err}");
            eprintln!("running headless smoke instead");
            headless_smoke(&args_fallback)
        }
        Err(err) => Err(err),
    }
}

fn headless_smoke(args: &Args) -> eframe::Result<()> {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let appearance = args.appearance_pref.resolve(&ctx);
    theme::apply(&ctx, appearance);
    ctx.set_pixels_per_point(args.scale);
    let mut app = App::new(args.clone(), appearance);
    app.smoke = false;
    let size = egui::vec2(args.width, args.height);
    for _ in 0..3 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let _ = ctx.run(raw.clone(), |ctx| {
            app.tick(ctx, None);
        });
        app.apply_demo(args.demo_state);
        theme::apply(&ctx, Appearance::Light);
        let _ = ctx.run(raw, |ctx| {
            app.tick(ctx, None);
        });
    }
    eprintln!("headless smoke ok");
    Ok(())
}

#[derive(Clone)]
struct Args {
    smoke: bool,
    demo: bool,
    demo_state: DemoState,
    width: f32,
    height: f32,
    scale: f32,
    undecorated: bool,
    ready_file: Option<PathBuf>,
    appearance_pref: AppearancePref,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DemoState {
    Live,
    Empty,
    Error,
    List,
    Selected,
    Confirm,
    Countdown,
    Progress,
    Done,
    Failed,
}

impl Args {
    fn parse() -> Self {
        let mut a = Args {
            smoke: false,
            demo: false,
            demo_state: DemoState::Live,
            width: 1280.0,
            height: 800.0,
            scale: 1.0,
            undecorated: false,
            ready_file: None,
            appearance_pref: AppearancePref::System,
        };
        let mut it = std::env::args().skip(1).peekable();
        while let Some(flag) = it.next() {
            match flag.as_str() {
                "--smoke-test" => a.smoke = true,
                "--demo" => a.demo = true,
                "--undecorated" => a.undecorated = true,
                "--demo-state" => {
                    a.demo = true;
                    a.demo_state = parse_demo_state(&it.next().unwrap_or_default());
                }
                "--width" => a.width = it.next().and_then(|s| s.parse().ok()).unwrap_or(1280.0),
                "--height" => a.height = it.next().and_then(|s| s.parse().ok()).unwrap_or(800.0),
                "--scale" => a.scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
                "--ready-file" => a.ready_file = it.next().map(PathBuf::from),
                "--appearance" => {
                    a.appearance_pref = AppearancePref::parse(&it.next().unwrap_or_default());
                }
                _ => {}
            }
        }
        if a.smoke {
            a.undecorated = true;
        }
        a
    }
}

fn parse_demo_state(s: &str) -> DemoState {
    match s {
        "empty" => DemoState::Empty,
        "error" => DemoState::Error,
        "list" => DemoState::List,
        "selected" => DemoState::Selected,
        "confirm" => DemoState::Confirm,
        "countdown" => DemoState::Countdown,
        "progress" => DemoState::Progress,
        "done" => DemoState::Done,
        "failed" => DemoState::Failed,
        _ => DemoState::List,
    }
}

enum UiEvent {
    Progress(Progress),
    Done(Box<Result<Report, String>>),
}

#[derive(Clone, Copy, PartialEq)]
enum MethodKind {
    Zeros,
    Random,
    Multi,
}

#[derive(Clone, Copy, PartialEq)]
enum ScopeKind {
    Whole,
    FreeSpace,
}

enum Screen {
    Main,
    Confirm,
    Countdown,
    Work,
    Report,
    Error,
}

struct App {
    lang: Lang,
    devices: Vec<DeviceInfo>,
    list_error: Option<String>,
    selected: Option<usize>,
    method: MethodKind,
    verify: bool,
    dry_run: bool,
    scope: ScopeKind,
    confirm: String,
    screen: Screen,
    countdown: Option<u32>,
    cancel: Arc<AtomicBool>,
    progress: Option<Progress>,
    report: Option<Report>,
    error: Option<String>,
    running: bool,
    ev_rx: Receiver<UiEvent>,
    ev_tx: Sender<UiEvent>,
    smoke: bool,
    smoke_frames: u32,
    ready_file: Option<PathBuf>,
    ready_written: bool,
    ready_frames: u32,
    demo: bool,
    scale: f32,
    window_w: f32,
    window_h: f32,
    appearance_pref: AppearancePref,
    appearance: Appearance,
    undecorated: bool,
    vibrancy: bool,
    vibrancy_theme: Option<Appearance>,
    modal_anim: f32,
    display_frac: f32,
    sidebar_user: Option<bool>,
    sidebar_w: f32,
    list_focus: bool,
    confirm_focus: bool,
    refresh_frames: u32,
}

impl App {
    fn new(args: Args, appearance: Appearance) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            lang: Lang::from_env(),
            devices: Vec::new(),
            list_error: None,
            selected: None,
            method: MethodKind::Zeros,
            verify: true,
            dry_run: false,
            scope: ScopeKind::Whole,
            confirm: String::new(),
            screen: Screen::Main,
            countdown: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: None,
            report: None,
            error: None,
            running: false,
            ev_rx: rx,
            ev_tx: tx,
            smoke: args.smoke,
            smoke_frames: 0,
            ready_file: args.ready_file,
            ready_written: false,
            ready_frames: 0,
            demo: args.demo,
            scale: args.scale.max(0.5),
            window_w: args.width.max(320.0),
            window_h: args.height.max(240.0),
            appearance_pref: args.appearance_pref,
            appearance,
            undecorated: args.undecorated,
            vibrancy: vibrancy::supported(),
            vibrancy_theme: None,
            modal_anim: if matches!(args.demo_state, DemoState::Confirm | DemoState::Countdown) {
                1.0
            } else {
                0.0
            },
            display_frac: 0.0,
            sidebar_user: None,
            sidebar_w: if args.width < SIDEBAR_COLLAPSE_AT {
                SIDEBAR_RAIL
            } else {
                SIDEBAR_WIDTH
            },
            list_focus: true,
            confirm_focus: true,
            refresh_frames: 0,
        };
        if args.demo {
            app.apply_demo(args.demo_state);
            if matches!(args.demo_state, DemoState::Progress) {
                if let Some(p) = &app.progress {
                    app.display_frac = p.fraction() as f32;
                }
            }
        } else {
            app.refresh();
        }
        app
    }

    fn t(&self, key: &str) -> &'static str {
        i18n::t(self.lang, key)
    }

    fn refresh(&mut self) {
        self.refresh_frames = 18;
        if self.demo {
            return;
        }
        match device::enumerate(EnumerateOptions {
            include_virtual: false,
        }) {
            Ok(list) => {
                self.devices = list;
                self.list_error = None;
                if self
                    .selected
                    .map(|i| i >= self.devices.len())
                    .unwrap_or(false)
                {
                    self.selected = None;
                }
            }
            Err(e) => {
                self.list_error = Some(e.to_string());
                self.devices.clear();
            }
        }
    }

    fn apply_demo(&mut self, state: DemoState) {
        self.devices = demo_devices();
        self.list_error = None;
        self.error = None;
        self.report = None;
        self.selected = None;
        self.screen = Screen::Main;
        self.running = false;
        self.confirm.clear();
        match state {
            DemoState::Live | DemoState::List => {}
            DemoState::Empty => self.devices.clear(),
            DemoState::Error => {
                self.devices.clear();
                self.list_error = Some("Could not list devices (demo).".into());
            }
            DemoState::Selected => self.selected = Some(0),
            DemoState::Confirm => {
                self.selected = Some(0);
                self.screen = Screen::Confirm;
                self.confirm_focus = true;
            }
            DemoState::Countdown => {
                self.selected = Some(0);
                self.screen = Screen::Countdown;
                self.countdown = Some(5);
                self.running = true;
            }
            DemoState::Progress => {
                self.selected = Some(0);
                self.screen = Screen::Work;
                self.running = true;
                self.progress = Some(Progress {
                    phase: Phase::Writing,
                    pass: 1,
                    passes: 1,
                    bytes_done: 900_000_000_000,
                    bytes_total: 2_000_000_000_000,
                    throughput_bps: 480 * 1024 * 1024,
                    eta_secs: Some(2_291),
                    countdown_remaining: None,
                    message: None,
                });
            }
            DemoState::Done | DemoState::Failed => {
                self.selected = Some(0);
                self.screen = Screen::Report;
                let ok = state == DemoState::Done;
                self.report = Some(demo_report(ok, self.devices[0].snapshot.identity.clone()));
            }
        }
    }

    fn selected_snap(&self) -> Option<&TargetSnapshot> {
        self.selected
            .and_then(|i| self.devices.get(i))
            .map(|d| &d.snapshot)
    }

    fn selectable_indices(&self) -> Vec<usize> {
        self.devices
            .iter()
            .enumerate()
            .filter(|(_, d)| protection(&d.snapshot).is_none())
            .map(|(i, _)| i)
            .collect()
    }

    fn move_selection(&mut self, dir: i32) {
        let opts = self.selectable_indices();
        if opts.is_empty() {
            return;
        }
        self.list_focus = true;
        let pos = self
            .selected
            .and_then(|s| opts.iter().position(|i| *i == s))
            .unwrap_or(if dir > 0 { usize::MAX } else { 0 });
        let next = if dir > 0 {
            if pos == usize::MAX {
                0
            } else {
                (pos + 1).min(opts.len() - 1)
            }
        } else if pos == 0 || pos == usize::MAX {
            0
        } else {
            pos - 1
        };
        self.selected = Some(opts[next]);
        self.error = None;
        if matches!(self.screen, Screen::Report | Screen::Error) {
            self.screen = Screen::Main;
            self.report = None;
            self.error = None;
        }
    }

    fn try_open_confirm(&mut self) {
        let Some(snap) = self.selected_snap() else {
            return;
        };
        if protection(snap).is_some() {
            return;
        }
        if self.scope == ScopeKind::Whole && !snap.mounts.is_empty() {
            self.error = Some(self.t("ui.mounted_hint").to_string());
            return;
        }
        self.confirm.clear();
        self.error = None;
        self.screen = Screen::Confirm;
        self.modal_anim = 0.0;
        self.confirm_focus = true;
    }

    fn start_wipe(&mut self) {
        let Some(dev) = self.selected.and_then(|i| self.devices.get(i)) else {
            return;
        };
        let name = dev.snapshot.identity.display_name.clone();
        if !confirmation_matches(&name, &self.confirm) {
            self.error = Some(self.t("ui.confirm_mismatch").replace("{name}", &name));
            return;
        }
        if self.demo {
            self.screen = Screen::Countdown;
            self.countdown = Some(5);
            self.running = true;
            self.modal_anim = 0.0;
            return;
        }
        let mut method = match self.method {
            MethodKind::Zeros => Method::zeros(),
            MethodKind::Random => Method::random(),
            MethodKind::Multi => Method::multi_default(),
        };
        if !self.verify {
            if let Method::Single { verify, .. } = &mut method {
                *verify = false;
            }
        }
        let path = dev.snapshot.identity.path.clone();
        let scope = match self.scope {
            ScopeKind::Whole => Scope::WholeDevice,
            ScopeKind::FreeSpace => Scope::FreeSpace,
        };
        let flags = SafetyFlags {
            explicit_target: true,
            typed_confirm: Some(name),
            assume_yes: true,
            countdown_secs: if dev.snapshot.is_virtual() { 0 } else { 5 },
            dry_run: self.dry_run,
            ..SafetyFlags::default()
        };
        self.cancel.store(false, Ordering::SeqCst);
        self.running = true;
        self.error = None;
        self.report = None;
        self.screen = Screen::Countdown;
        self.modal_anim = 0.0;
        let cancel = self.cancel.clone();
        let tx = self.ev_tx.clone();
        thread::spawn(move || {
            let req = WipeRequest {
                path: path.clone(),
                scope,
                method,
                chunk_size: None,
                flags,
            };
            let result = if scope == Scope::FreeSpace {
                frame_wipe::free_space::wipe_free_space(
                    &req.path,
                    req.method,
                    req.flags,
                    &cancel,
                    |p| {
                        let _ = tx.send(UiEvent::Progress(p));
                    },
                )
                .map_err(|e| e.to_string())
            } else {
                wipe(&req, &cancel, |p| {
                    let _ = tx.send(UiEvent::Progress(p));
                })
                .map_err(|e| e.to_string())
            };
            let _ = tx.send(UiEvent::Done(Box::new(result)));
        });
    }

    fn sidebar_collapsed(&self, width: f32) -> bool {
        self.sidebar_user.unwrap_or(width < SIDEBAR_COLLAPSE_AT)
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.vibrancy {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            let c = theme::Palette::for_appearance(self.appearance).wallpaper;
            [
                c.r() as f32 / 255.0,
                c.g() as f32 / 255.0,
                c.b() as f32 / 255.0,
                1.0,
            ]
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.tick(ctx, Some(frame));
    }
}

impl App {
    fn tick(&mut self, ctx: &egui::Context, frame: Option<&eframe::Frame>) {
        ctx.set_pixels_per_point(self.scale);
        self.appearance = self.appearance_pref.resolve(ctx);
        theme::apply(ctx, self.appearance);
        if self.vibrancy {
            if let Some(frame) = frame {
                if self.vibrancy_theme != Some(self.appearance) {
                    vibrancy::apply(frame, self.appearance == Appearance::Dark);
                    self.vibrancy_theme = Some(self.appearance);
                }
            }
        }
        if self.demo {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                self.window_w,
                self.window_h,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::ZERO));
        }
        while let Ok(ev) = self.ev_rx.try_recv() {
            match ev {
                UiEvent::Progress(p) => {
                    self.countdown = p.countdown_remaining;
                    if p.countdown_remaining.is_some() {
                        self.screen = Screen::Countdown;
                    } else if !matches!(self.screen, Screen::Report) {
                        self.screen = Screen::Work;
                    }
                    self.progress = Some(p);
                }
                UiEvent::Done(boxed) => {
                    self.running = false;
                    self.countdown = None;
                    match *boxed {
                        Ok(r) => {
                            self.report = Some(r);
                            self.screen = Screen::Report;
                            self.progress = None;
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.screen = Screen::Error;
                            self.progress = None;
                        }
                    }
                }
            }
        }

        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.05);
        let modal = matches!(self.screen, Screen::Confirm | Screen::Countdown);
        if modal {
            self.modal_anim = theme::lerp(self.modal_anim, 1.0, dt, theme::MOTION_MED);
        } else {
            self.modal_anim = 0.0;
        }
        let target_frac = self
            .progress
            .as_ref()
            .map(|p| p.fraction() as f32)
            .unwrap_or(0.0);
        self.display_frac = theme::lerp(self.display_frac, target_frac, dt, theme::MOTION_SLOW);
        let width = ctx.screen_rect().width();
        let collapsed = self.sidebar_collapsed(width);
        let target_w = if collapsed {
            SIDEBAR_RAIL
        } else {
            SIDEBAR_WIDTH
        };
        self.sidebar_w = theme::lerp(self.sidebar_w, target_w, dt, theme::MOTION_MED);
        let animating = (self.sidebar_w - target_w).abs() > 0.4
            || (modal && self.modal_anim < 0.999)
            || (self.display_frac - target_frac).abs() > 0.002
            || self.running
            || self.refresh_frames > 0;
        if animating {
            ctx.request_repaint();
        }
        if self.refresh_frames > 0 {
            self.refresh_frames = self.refresh_frames.saturating_sub(1);
        }

        handle_keys(self, ctx);

        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
        if self.vibrancy {
            painter.rect_filled(screen, 0.0, Color32::TRANSPARENT);
        } else {
            theme::paint_wallpaper(&painter, screen);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin::ZERO))
            .show(ctx, |ui| {
                let work = ui.max_rect().shrink(WINDOW_INSET);
                let toolbar =
                    Rect::from_min_max(work.min, Pos2::new(work.max.x, work.min.y + TOOLBAR_H));
                let body =
                    Rect::from_min_max(Pos2::new(work.min.x, toolbar.bottom() + 8.0), work.max);
                let sidebar =
                    Rect::from_min_size(body.min, Vec2::new(self.sidebar_w, body.height()));
                let content =
                    Rect::from_min_max(Pos2::new(sidebar.right() + PANE_GAP, body.min.y), body.max);
                let collapsed = self.sidebar_collapsed(ui.max_rect().width());
                self.draw_toolbar(ui, toolbar, collapsed);
                self.draw_sidebar(ui, sidebar, collapsed);
                self.draw_content(ui, content);
            });

        match self.screen {
            Screen::Confirm => self.confirm_modal(ctx),
            Screen::Countdown => self.countdown_modal(ctx),
            _ => {}
        }

        if self.smoke {
            self.smoke_frames += 1;
            if self.smoke_frames > 2 {
                std::process::exit(0);
            }
        }
        if !self.ready_written {
            self.ready_frames += 1;
            if self.ready_frames > 2 {
                if let Some(path) = &self.ready_file {
                    let rect = ctx.screen_rect();
                    let ppp = ctx.pixels_per_point();
                    let body = format!(
                        "ready\nlogical={:.0}x{:.0}\nppp={}\nphysical={:.0}x{:.0}\n",
                        rect.width(),
                        rect.height(),
                        ppp,
                        rect.width() * ppp,
                        rect.height() * ppp
                    );
                    let _ = std::fs::write(path, body);
                }
                self.ready_written = true;
            }
        }
    }

    fn draw_toolbar(&mut self, ui: &mut Ui, rect: Rect, collapsed: bool) {
        let p = theme::pal();
        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.set_height(rect.height());
                ui.spacing_mut().item_spacing.x = 10.0;
                if self.undecorated {
                    widgets::traffic_lights(ui);
                    ui.add_space(6.0);
                }
                let tip = if collapsed {
                    self.t("ui.show_sidebar")
                } else {
                    self.t("ui.hide_sidebar")
                };
                let icon = if collapsed {
                    Icon::SidebarExpand
                } else {
                    Icon::Sidebar
                };
                if widgets::icon_tool_button(ui, icon, tip, !self.running).clicked() {
                    self.sidebar_user = Some(!collapsed);
                }
                ui.label(
                    RichText::new(self.t("app.name"))
                        .font(FontId::new(FONT_HEAD, theme::semibold()))
                        .color(p.text),
                );
                if rect.width() >= 720.0 {
                    ui.label(
                        RichText::new(self.t("app.tagline"))
                            .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                            .color(p.text_tertiary),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let refreshing = self.refresh_frames > 0;
                    let tip = if refreshing {
                        self.t("ui.refreshing")
                    } else {
                        self.t("ui.refresh")
                    };
                    if widgets::icon_tool_button(ui, Icon::Refresh, tip, !self.running).clicked() {
                        self.refresh();
                    }
                    let mut lang = self.lang;
                    widgets::segmented(ui, &mut lang, &[(Lang::Fr, "FR"), (Lang::En, "EN")]);
                    self.lang = lang;
                });
            },
        );
    }

    fn draw_sidebar(&mut self, ui: &mut Ui, rect: Rect, collapsed: bool) {
        let p = theme::pal();
        ui.painter().rect(
            rect,
            CornerRadius::same(theme::RADIUS_LG as u8),
            p.glass,
            Stroke::new(1.0_f32, p.glass_stroke),
            StrokeKind::Inside,
        );
        theme::paint_glass_highlight(ui.painter(), rect, theme::RADIUS_LG);
        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(rect.shrink(10.0))
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(rect.shrink(2.0));
                ui.set_width(ui.available_width());
                if collapsed {
                    ui.vertical_centered(|ui| {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("{}", self.devices.len()))
                                .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                                .color(p.text_tertiary),
                        );
                    });
                    ui.add_space(6.0);
                    if self.list_error.is_some() {
                        widgets::status_glyph(ui, Icon::Failure, p.danger, 28.0);
                        return;
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("sidebar-rail")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            let n = self.devices.len();
                            for i in 0..n {
                                self.device_rail_item(ui, i);
                            }
                        });
                    return;
                }
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.t("ui.devices"))
                            .font(FontId::new(FONT_FOOTNOTE, theme::semibold()))
                            .color(p.text_tertiary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{}", self.devices.len()))
                                .font(FontId::new(FONT_CAPTION, theme::medium()))
                                .color(p.text_quaternary),
                        );
                    });
                });
                ui.add_space(6.0);
                if self.list_error.is_some() {
                    ui.label(
                        RichText::new(self.t("ui.failed"))
                            .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                            .color(p.danger),
                    );
                    return;
                }
                if self.devices.is_empty() {
                    ui.label(
                        RichText::new(self.t("ui.empty_title"))
                            .color(p.text_tertiary)
                            .size(FONT_FOOTNOTE),
                    );
                    return;
                }
                let avail_h = ui.available_height();
                egui::ScrollArea::vertical()
                    .id_salt("sidebar-list")
                    .auto_shrink([false, false])
                    .max_height(avail_h)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let available: Vec<usize> = self
                            .devices
                            .iter()
                            .enumerate()
                            .filter(|(_, d)| protection(&d.snapshot).is_none())
                            .map(|(i, _)| i)
                            .collect();
                        let protected: Vec<usize> = self
                            .devices
                            .iter()
                            .enumerate()
                            .filter(|(_, d)| protection(&d.snapshot).is_some())
                            .map(|(i, _)| i)
                            .collect();
                        if !available.is_empty() {
                            section_header(ui, self.t("ui.available"));
                            for i in available {
                                self.device_row(ui, i);
                            }
                        }
                        if !protected.is_empty() {
                            ui.add_space(8.0);
                            section_header(ui, self.t("ui.protected"));
                            for i in protected {
                                self.device_row(ui, i);
                            }
                        }
                    });
            },
        );
    }

    fn device_rail_item(&mut self, ui: &mut Ui, index: usize) {
        let snap = self.devices[index].snapshot.clone();
        let blocked = protection(&snap).is_some();
        let selected = self.selected == Some(index);
        let model = display_model(&snap, self.lang);
        let p = theme::pal();
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 36.0).max(0.0) * 0.5);
            let (rect, resp) = ui.allocate_exact_size(
                Vec2::splat(36.0),
                if blocked {
                    Sense::hover()
                } else {
                    Sense::click()
                },
            );
            let fill = if selected {
                p.selection
            } else if resp.hovered() {
                p.hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect(
                rect,
                CornerRadius::same(10),
                fill,
                Stroke::NONE,
                StrokeKind::Inside,
            );
            if selected && self.list_focus {
                widgets::paint_focus_ring(ui, rect, 10.0);
            }
            let icon_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::centered_and_justified(egui::Direction::TopDown)),
            );
            // paint glyph directly
            let color = if blocked {
                p.text_tertiary
            } else if selected {
                p.accent
            } else {
                p.text_secondary
            };
            widgets::paint_icon(
                ui.painter(),
                Icon::Drive(snap.kind),
                rect.shrink(8.0),
                color,
                1.4,
            );
            let _ = icon_ui;
            let tip = format!(
                "{model}\n{}  ·  {}",
                format_bytes(snap.identity.size),
                snap.kind.label()
            );
            let resp = resp.on_hover_text(tip);
            if blocked && resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
            }
            if !blocked && resp.clicked() && !self.running {
                self.select_device(index);
            }
        });
    }

    fn device_row(&mut self, ui: &mut Ui, index: usize) {
        let snap = self.devices[index].snapshot.clone();
        let blocked = protection(&snap);
        let selected = self.selected == Some(index);
        let model = display_model(&snap, self.lang);
        let serial = snap
            .identity
            .serial
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(self.t("ui.no_serial"));
        let size = format_bytes(snap.identity.size);
        let name = snap.identity.display_name.clone();
        let p = theme::pal();
        let w = ui.available_width();
        let h = 84.0;
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(w, h),
            if blocked.is_some() {
                Sense::hover()
            } else {
                Sense::click()
            },
        );
        let fill = if selected {
            p.selection
        } else if resp.hovered() {
            p.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(12),
            fill,
            Stroke::NONE,
            StrokeKind::Inside,
        );
        if selected && self.list_focus {
            widgets::paint_focus_ring(ui, rect, 12.0);
        }
        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(8.0, 6.0)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.set_clip_rect(rect.shrink(1.0));
                ui.spacing_mut().item_spacing.x = 8.0;
                widgets::drive_glyph(ui, snap.kind, selected, blocked.is_some(), 28.0);
                let rest = ui.available_width();
                ui.vertical(|ui| {
                    ui.set_width(rest);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let title_color = if blocked.is_some() {
                        p.text_secondary
                    } else {
                        p.text
                    };
                    truncated_label(
                        ui,
                        &model,
                        FontId::new(FONT_BODY, theme::semibold()),
                        title_color,
                    );
                    ui.horizontal(|ui| {
                        ui.set_width(rest);
                        ui.allocate_ui(Vec2::new((rest - 64.0).max(40.0), 16.0), |ui| {
                            truncated_label(
                                ui,
                                &format!("{name}  ·  {serial}"),
                                FontId::new(FONT_CAPTION, theme::medium()),
                                p.text_tertiary,
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(&size)
                                    .font(FontId::new(FONT_CAPTION, theme::medium()))
                                    .color(if blocked.is_some() {
                                        p.text_tertiary
                                    } else {
                                        p.text_secondary
                                    }),
                            );
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.set_width(rest);
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if snap.is_system {
                            widgets::pill(ui, self.t("ui.system"), p.danger, p.danger_bg);
                        }
                        if snap.is_app_disk {
                            widgets::pill(ui, self.t("ui.app_disk"), p.warning, p.warning_bg);
                        }
                        if !snap.mounts.is_empty() {
                            widgets::pill(ui, self.t("ui.mounted"), p.warning, p.warning_bg);
                        } else if snap.in_use {
                            widgets::pill(ui, self.t("ui.in_use"), p.warning, p.warning_bg);
                        }
                        if snap.is_raid_member || snap.is_lvm_member {
                            widgets::pill(ui, self.t("ui.raid"), p.warning, p.warning_bg);
                        }
                    });
                });
            },
        );
        if let Some(reason_key) = blocked {
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
            }
            resp.on_hover_text(self.t(reason_key));
        } else if resp.clicked() && !self.running {
            self.select_device(index);
        }
    }

    fn select_device(&mut self, index: usize) {
        self.selected = Some(index);
        self.error = None;
        self.list_focus = true;
        if matches!(self.screen, Screen::Report | Screen::Error) {
            self.screen = Screen::Main;
            self.report = None;
            self.error = None;
        }
    }

    fn draw_content(&mut self, ui: &mut Ui, rect: Rect) {
        let p = theme::pal();
        ui.painter().rect(
            rect,
            CornerRadius::same(theme::RADIUS_LG as u8),
            p.content_fill,
            Stroke::new(1.0_f32, p.glass_stroke),
            StrokeKind::Inside,
        );
        theme::paint_glass_highlight(ui.painter(), rect, theme::RADIUS_LG);
        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(rect.shrink(20.0))
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_width(ui.available_width());
                let h = ui.available_height();
                match self.screen {
                    Screen::Work => self.progress_view(ui, h),
                    Screen::Report => self.report_view(ui, h),
                    Screen::Error => self.error_view(ui, h),
                    _ => {
                        if self.selected_snap().is_some() {
                            self.detail_selected(ui, h);
                        } else {
                            self.detail_overview(ui, h);
                        }
                    }
                }
            },
        );
    }

    fn detail_overview(&self, ui: &mut Ui, h: f32) {
        let p = theme::pal();
        let w = ui.available_width();
        ui.allocate_ui(Vec2::new(w, h), |ui| {
            ui.set_width(w);
            ui.set_max_height(h);
            let total = self.devices.len();
            let open = self.selectable_indices().len();
            let protected = total.saturating_sub(open);
            if let Some(err) = &self.list_error {
                header_block(ui, Icon::Failure, p.danger, self.t("ui.error_list"), err);
                ui.add_space(GRID * 2.0);
                widgets::content_card(ui, w, |ui| self.tips_list(ui));
                return;
            }
            if total == 0 {
                header_block(
                    ui,
                    Icon::Empty,
                    p.text_secondary,
                    self.t("ui.empty_title"),
                    self.t("ui.empty"),
                );
                ui.add_space(GRID * 2.0);
                widgets::content_card(ui, w, |ui| self.tips_list(ui));
                return;
            }
            ui.label(
                RichText::new(self.t("ui.drives_summary"))
                    .font(FontId::new(FONT_TITLE, theme::semibold()))
                    .color(p.text),
            );
            ui.label(
                RichText::new(if open > 0 {
                    self.t("ui.select_hint")
                } else {
                    self.t("ui.none_available")
                })
                .color(p.text_secondary)
                .size(FONT_BODY),
            );
            ui.add_space(GRID);
            widgets::stat_row(
                ui,
                w,
                &[
                    (format!("{open}"), self.t("ui.can_overwrite"), p.success),
                    (
                        format!("{protected}"),
                        self.t("ui.protected_count"),
                        p.warning,
                    ),
                    (format!("{total}"), self.t("ui.devices"), p.text_secondary),
                ],
            );
            ui.add_space(GRID * 2.0);
            let two_col = w >= 640.0 && protected > 0;
            if two_col {
                let gap = GRID * 2.0;
                let col_w = ((w - gap) / 2.0).floor();
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui_with_layout(
                        Vec2::new(col_w, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(col_w);
                            widgets::content_card(ui, col_w, |ui| self.protected_list(ui));
                        },
                    );
                    ui.allocate_ui_with_layout(
                        Vec2::new(col_w, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(col_w);
                            widgets::content_card(ui, col_w, |ui| self.tips_list(ui));
                        },
                    );
                });
            } else {
                if protected > 0 {
                    widgets::content_card(ui, w, |ui| self.protected_list(ui));
                    ui.add_space(GRID * 2.0);
                }
                widgets::content_card(ui, w, |ui| self.tips_list(ui));
            }
        });
    }

    fn protected_list(&self, ui: &mut Ui) {
        let p = theme::pal();
        ui.label(
            RichText::new(self.t("ui.protected"))
                .font(FontId::new(FONT_HEAD, theme::semibold()))
                .color(p.text),
        );
        ui.add_space(GRID * 0.5);
        for d in &self.devices {
            if protection(&d.snapshot).is_some() {
                let model = display_model(&d.snapshot, self.lang);
                let why = protection(&d.snapshot).map(|k| self.t(k)).unwrap_or("");
                truncated_label(
                    ui,
                    &format!("{}  ·  {}", d.snapshot.identity.display_name, model),
                    FontId::new(FONT_BODY, theme::medium()),
                    p.text,
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(why)
                            .color(p.text_tertiary)
                            .size(FONT_FOOTNOTE),
                    )
                    .wrap(),
                );
                ui.add_space(GRID * 0.5);
            }
        }
    }

    fn tips_list(&self, ui: &mut Ui) {
        let p = theme::pal();
        ui.label(
            RichText::new(self.t("ui.tips"))
                .font(FontId::new(FONT_HEAD, theme::semibold()))
                .color(p.text),
        );
        ui.add_space(GRID);
        for key in ["ui.tip1", "ui.tip2", "ui.tip3"] {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("·").color(p.text_tertiary));
                ui.add(
                    egui::Label::new(
                        RichText::new(self.t(key))
                            .color(p.text_secondary)
                            .size(FONT_BODY),
                    )
                    .wrap(),
                );
            });
            ui.add_space(6.0);
        }
    }
    fn detail_selected(&mut self, ui: &mut Ui, h: f32) {
        let Some(snap) = self.selected_snap().cloned() else {
            return;
        };
        let p = theme::pal();
        let t_scope = self.t("ui.scope");
        let t_whole = self.t("ui.whole");
        let t_free = self.t("ui.freespace");
        let t_method = self.t("ui.method");
        let t_zeros = self.t("ui.zeros");
        let t_random = self.t("ui.random");
        let t_multi = self.t("ui.multi");
        let t_verify = self.t("ui.verify");
        let t_dry = self.t("ui.dry_run");
        let t_over = self.t("ui.overwrite");
        let t_est = self.t("ui.estimate");
        let t_est_note = self.t("ui.estimate_note");
        let t_serial = self.t("ui.serial");
        let t_path = self.t("ui.path");
        let t_size = self.t("ui.size");
        let t_type = self.t("ui.type");
        let t_mounted = self.t("ui.mounted");
        let model = display_model(&snap, self.lang);
        let serial = snap
            .identity
            .serial
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.t("ui.no_serial").to_string());
        let mounted = !snap.mounts.is_empty();
        let blocked = protection(&snap).is_some();
        let w = ui.available_width();
        let modal_open = matches!(self.screen, Screen::Confirm | Screen::Countdown);
        let two_col = w >= 560.0;

        ui.allocate_ui(Vec2::new(w, h), |ui| {
            ui.set_width(w);
            ui.set_max_height(h);
            egui::ScrollArea::vertical()
                .id_salt("detail-selected")
                .auto_shrink([false, true])
                .max_height(h)
                .show(ui, |ui| {
                    ui.set_width(w);
                    ui.label(
                        RichText::new(&model)
                            .font(FontId::new(FONT_TITLE, theme::semibold()))
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{}  ·  {}  ·  {}",
                            snap.identity.display_name,
                            snap.kind.label(),
                            format_bytes(snap.identity.size)
                        ))
                        .color(p.text_secondary)
                        .size(FONT_BODY),
                    );
                    ui.add_space(GRID);
                    if mounted && self.scope == ScopeKind::Whole {
                        ui.label(
                            RichText::new(self.t("ui.mounted_hint"))
                                .color(p.warning)
                                .size(FONT_BODY),
                        );
                        ui.add_space(GRID);
                    }
                    let size_txt = format_bytes(snap.identity.size);
                    let size_bytes = format!("{} bytes", snap.identity.size);
                    let path_txt = snap.identity.path.display().to_string();
                    let type_txt = snap.kind.label().to_string();
                    let mut facts: Vec<(&str, &str, Option<&str>)> = vec![
                        (t_serial, serial.as_str(), None),
                        (t_path, path_txt.as_str(), None),
                        (t_size, size_txt.as_str(), Some(size_bytes.as_str())),
                        (t_type, type_txt.as_str(), None),
                    ];
                    let mounts_txt = snap.mounts.join(", ");
                    if mounted {
                        facts.push((t_mounted, mounts_txt.as_str(), None));
                    }
                    widgets::content_card(ui, w, |ui| fact_strip(ui, &facts, two_col));
                    ui.add_space(GRID * 2.0);
                    let est = rough_estimate(snap.identity.size, self.method, self.verify);
                    let choices = |ui: &mut Ui| {
                        ui.label(
                            RichText::new(t_scope)
                                .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                                .color(p.text_tertiary),
                        );
                        widgets::segmented(
                            ui,
                            &mut self.scope,
                            &[(ScopeKind::Whole, t_whole), (ScopeKind::FreeSpace, t_free)],
                        );
                        ui.add_space(GRID);
                        ui.label(
                            RichText::new(t_method)
                                .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                                .color(p.text_tertiary),
                        );
                        widgets::segmented(
                            ui,
                            &mut self.method,
                            &[
                                (MethodKind::Zeros, t_zeros),
                                (MethodKind::Random, t_random),
                                (MethodKind::Multi, t_multi),
                            ],
                        );
                        ui.add_space(GRID);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = GRID * 2.0;
                            widgets::toggle(ui, &mut self.verify, t_verify);
                            widgets::toggle(ui, &mut self.dry_run, t_dry);
                        });
                    };
                    let estimate = |ui: &mut Ui| {
                        ui.label(
                            RichText::new(t_est)
                                .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                                .color(p.text_tertiary),
                        );
                        ui.label(
                            RichText::new(&est)
                                .font(FontId::new(FONT_TITLE3, theme::semibold()))
                                .color(p.text),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(t_est_note)
                                    .color(p.text_tertiary)
                                    .size(FONT_SMALL),
                            )
                            .wrap(),
                        );
                    };
                    if two_col {
                        let gap = GRID * 2.0;
                        let left_w = ((w - gap) * 0.62).floor();
                        let right_w = w - gap - left_w;
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            ui.allocate_ui_with_layout(
                                Vec2::new(left_w, 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_width(left_w);
                                    widgets::content_card(ui, left_w, choices);
                                },
                            );
                            ui.allocate_ui_with_layout(
                                Vec2::new(right_w, 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_width(right_w);
                                    widgets::content_card(ui, right_w, estimate);
                                },
                            );
                        });
                    } else {
                        widgets::content_card(ui, w, choices);
                        ui.add_space(GRID * 2.0);
                        widgets::content_card(ui, w, estimate);
                    }
                    if !modal_open {
                        let enabled = !blocked
                            && !self.running
                            && !(mounted && self.scope == ScopeKind::Whole);
                        let err = self.error.clone();
                        ui.add_space(GRID * 3.0);
                        let btn =
                            widgets::danger_button(ui, t_over, enabled, w.min(280.0).max(180.0));
                        if btn.clicked() {
                            self.try_open_confirm();
                        }
                        if let Some(err) = err {
                            ui.add_space(GRID);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(err).color(p.danger).size(FONT_BODY),
                                )
                                .wrap(),
                            );
                        }
                    }
                });
        });
    }

    fn progress_view(&mut self, ui: &mut Ui, h: f32) {
        let p = theme::pal();
        let t_cancel = self.t("ui.cancel");
        let title = self
            .selected_snap()
            .map(|s| display_model(s, self.lang))
            .unwrap_or_else(|| self.t("ui.working").to_string());
        let subtitle = self
            .selected_snap()
            .map(|s| s.identity.display_name.clone())
            .unwrap_or_default();
        let phase_label = self
            .progress
            .as_ref()
            .map(|pr| self.t(phase_key(pr.phase)))
            .unwrap_or_else(|| self.t("ui.working"));
        let w = ui.available_width();
        ui.allocate_ui(Vec2::new(w, h), |ui| {
            ui.set_width(w);
            ui.set_max_height(h);
            egui::ScrollArea::vertical()
                .id_salt("progress-view")
                .auto_shrink([false, true])
                .max_height(h)
                .show(ui, |ui| {
                    ui.set_width(w);
                    ui.label(
                        RichText::new(&title)
                            .font(FontId::new(FONT_TITLE, theme::semibold()))
                            .color(p.text),
                    );
                    let meta = if subtitle.is_empty() {
                        phase_label.to_string()
                    } else {
                        format!("{phase_label}  ·  {subtitle}")
                    };
                    ui.label(RichText::new(meta).color(p.text_secondary).size(FONT_BODY));
                    ui.add_space(GRID * 2.0);
                    if let Some(pr) = &self.progress {
                        let frac = self.display_frac;
                        let show_verify =
                            self.verify || matches!(pr.phase, Phase::Verifying | Phase::Syncing);
                        stage_rail(ui, pr, show_verify, self.lang);
                        ui.add_space(GRID * 2.0);
                        ui.horizontal(|ui| {
                            let ring_w = 180.0;
                            ui.allocate_ui(Vec2::new(ring_w, 180.0), |ui| {
                                widgets::progress_ring(
                                    ui,
                                    frac,
                                    &format!("{:.0}%", (frac * 100.0).clamp(0.0, 100.0)),
                                    phase_label,
                                );
                            });
                            ui.add_space(GRID * 2.0);
                            ui.vertical(|ui| {
                                ui.add_space(16.0);
                                let bar_w = (w - ring_w - GRID * 2.0).max(160.0);
                                let (bar_rect, _) =
                                    ui.allocate_exact_size(Vec2::new(bar_w, 8.0), Sense::hover());
                                ui.painter()
                                    .rect_filled(bar_rect, CornerRadius::same(4), p.track);
                                let mut fill_rect = bar_rect;
                                let fill_w = bar_rect.width() * frac.clamp(0.0, 1.0);
                                fill_rect.set_width(fill_w);
                                if fill_w > 0.5 {
                                    ui.painter().rect_filled(
                                        fill_rect,
                                        CornerRadius::same(4),
                                        p.accent,
                                    );
                                }
                                ui.add_space(GRID);
                                let total = if pr.bytes_total == 0 {
                                    format_bytes(pr.bytes_done)
                                } else {
                                    format!(
                                        "{} / {}",
                                        format_bytes(pr.bytes_done),
                                        format_bytes(pr.bytes_total)
                                    )
                                };
                                ui.label(
                                    RichText::new(total).size(FONT_BODY).color(p.text_secondary),
                                );
                            });
                        });
                        ui.add_space(GRID * 2.0);
                        let written = if pr.bytes_total == 0 {
                            format_bytes(pr.bytes_done)
                        } else {
                            format!(
                                "{} / {}",
                                format_bytes(pr.bytes_done),
                                format_bytes(pr.bytes_total)
                            )
                        };
                        let eta = pr.eta_secs.map(format_eta).unwrap_or_else(|| "--".into());
                        let pass = format!("{}/{}", pr.pass, pr.passes);
                        let chips = [
                            (self.t("ui.written"), written),
                            (
                                self.t("ui.throughput"),
                                format_throughput(pr.throughput_bps),
                            ),
                            (self.t("ui.eta"), eta),
                            (self.t("ui.pass"), pass),
                        ];
                        widgets::metric_row(ui, w, &chips);
                    }
                    ui.add_space(GRID);
                    if widgets::glass_button(ui, t_cancel, true).clicked() {
                        self.cancel.store(true, Ordering::SeqCst);
                    }
                });
        });
    }

    fn report_view(&mut self, ui: &mut Ui, h: f32) {
        let p = theme::pal();
        let t_back = self.t("ui.back");
        let Some(r) = self.report.clone() else { return };
        let w = ui.available_width();
        let failed = r.outcome == Outcome::Failed;
        let (title, color, icon) = match r.outcome {
            Outcome::Success | Outcome::DryRun => {
                (self.t("ui.report_title"), p.success, Icon::Success)
            }
            Outcome::Cancelled => (self.t("ui.cancel"), p.warning, Icon::Warning),
            Outcome::Failed => (self.t("ui.failed"), p.danger, Icon::Failure),
        };
        ui.allocate_ui(Vec2::new(w, h), |ui| {
            ui.set_width(w);
            ui.set_max_height(h);
            egui::ScrollArea::vertical()
                .id_salt("report-view")
                .auto_shrink([false, true])
                .max_height(h)
                .show(ui, |ui| {
                    ui.set_width(w);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        widgets::status_glyph(ui, icon, color, 36.0);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(title)
                                    .font(FontId::new(FONT_TITLE, theme::semibold()))
                                    .color(color),
                            );
                            ui.label(
                                RichText::new(format!("{}  ·  {}", r.display_name, r.target_path))
                                    .color(p.text_secondary)
                                    .size(FONT_BODY),
                            );
                        });
                    });
                    ui.add_space(GRID * 2.0);
                    let elapsed = r
                        .elapsed_ms
                        .map(format_elapsed_ms)
                        .unwrap_or_else(|| "--".into());
                    let written = format_bytes(r.bytes_written);
                    let verified = if r.verified {
                        self.t("ui.verified")
                    } else {
                        self.t("ui.not_verified")
                    };
                    let chips = [
                        (self.t("ui.elapsed"), elapsed),
                        (self.t("ui.written"), written),
                        (self.t("ui.scope"), r.scope.clone()),
                        (self.t("ui.method"), r.method.clone()),
                        (self.t("ui.stage.verify"), verified.to_string()),
                    ];
                    widgets::metric_row(ui, w, &chips);
                    ui.add_space(GRID);
                    if !r.notes.is_empty() {
                        let (bg, stroke, lab) = if failed {
                            (p.danger_bg, p.danger, p.danger)
                        } else {
                            (p.content_fill, p.glass_stroke, p.text_tertiary)
                        };
                        egui::Frame::new()
                            .fill(bg)
                            .stroke(Stroke::new(1.0_f32, stroke))
                            .corner_radius(CornerRadius::same(theme::RADIUS_MD as u8))
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                ui.set_width((w - 26.0).max(0.0));
                                ui.label(
                                    RichText::new(self.t("ui.notes"))
                                        .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                                        .color(lab),
                                );
                                for n in &r.notes {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(n).color(p.text).size(FONT_BODY),
                                        )
                                        .wrap(),
                                    );
                                }
                            });
                        ui.add_space(GRID);
                    }
                    if !r.warnings.is_empty() {
                        egui::Frame::new()
                            .fill(p.warning_bg)
                            .stroke(Stroke::new(1.0_f32, p.warning))
                            .corner_radius(CornerRadius::same(theme::RADIUS_MD as u8))
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                ui.set_width((w - 26.0).max(0.0));
                                ui.label(
                                    RichText::new(self.t("ui.warnings"))
                                        .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                                        .color(p.warning),
                                );
                                for n in &r.warnings {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(n)
                                                .color(p.text_secondary)
                                                .size(FONT_BODY),
                                        )
                                        .wrap(),
                                    );
                                }
                            });
                        ui.add_space(GRID);
                    }
                    if widgets::glass_button(ui, t_back, true).clicked() {
                        self.screen = Screen::Main;
                        self.report = None;
                        self.progress = None;
                    }
                });
        });
    }

    fn error_view(&mut self, ui: &mut Ui, h: f32) {
        let p = theme::pal();
        let t_back = self.t("ui.back");
        let w = ui.available_width();
        let msg = self.error.clone().unwrap_or_default();
        ui.allocate_ui(Vec2::new(w, h), |ui| {
            ui.set_width(w);
            ui.set_max_height(h);
            egui::ScrollArea::vertical()
                .id_salt("error-view")
                .auto_shrink([false, true])
                .max_height(h)
                .show(ui, |ui| {
                    ui.set_width(w);
                    header_block(
                        ui,
                        Icon::Failure,
                        p.danger,
                        self.t("ui.failed"),
                        self.t("ui.wipe_error"),
                    );
                    ui.add_space(GRID * 2.0);
                    widgets::content_card(ui, w, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(msg).color(p.text).size(FONT_BODY))
                                .wrap(),
                        );
                    });
                    ui.add_space(GRID * 2.0);
                    if widgets::glass_button(ui, t_back, true).clicked() {
                        self.screen = Screen::Main;
                        self.error = None;
                        self.progress = None;
                    }
                });
        });
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let snap = self.selected_snap().cloned();
        let name = snap
            .as_ref()
            .map(|s| s.identity.display_name.clone())
            .unwrap_or_default();
        let model = snap
            .as_ref()
            .map(|s| display_model(s, self.lang))
            .unwrap_or_default();
        let size = snap
            .as_ref()
            .map(|s| format_bytes(s.identity.size))
            .unwrap_or_default();
        let path = snap
            .as_ref()
            .map(|s| s.identity.path.display().to_string())
            .unwrap_or_default();
        let title = self.t("ui.confirm_overwrite");
        let hint = self.t("ui.confirm_hint");
        let t_cancel = self.t("ui.cancel");
        let t_over = self.t("ui.overwrite");
        let p = theme::pal();
        let t = theme::ease_out_cubic(self.modal_anim);
        dim_background(ctx, (t * 160.0) as u8);
        let offset = (1.0 - t) * 16.0;
        egui::Window::new("confirm")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, offset])
            .default_width(520.0)
            .min_width(440.0)
            .max_width(560.0)
            .frame(theme::sheet_frame())
            .show(ctx, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(title)
                        .font(FontId::new(FONT_HEAD, theme::semibold()))
                        .color(p.danger),
                );
                ui.add_space(GRID);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{model}  ·  {name}  ·  {size}"))
                            .color(p.text)
                            .size(FONT_BODY),
                    )
                    .wrap(),
                );
                truncated_label(
                    ui,
                    &path,
                    FontId::new(FONT_FOOTNOTE, theme::medium()),
                    p.text_tertiary,
                );
                ui.add_space(GRID);
                ui.add(
                    egui::Label::new(RichText::new(hint).color(p.text_secondary).size(FONT_BODY))
                        .wrap(),
                );
                ui.add_space(GRID);
                ui.label(
                    RichText::new(format!("{}: {name}", self.t("ui.confirm_name")))
                        .color(p.text_secondary)
                        .size(FONT_FOOTNOTE),
                );
                let name_hint = self.t("ui.confirm_name");
                let edit = egui::TextEdit::singleline(&mut self.confirm)
                    .hint_text(name_hint)
                    .desired_width(ui.available_width())
                    .font(FontId::new(FONT_BODY, theme::medium()));
                let resp = ui.add(edit);
                if self.confirm_focus {
                    resp.request_focus();
                    self.confirm_focus = false;
                }
                if let Some(err) = &self.error {
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(RichText::new(err).color(p.danger).size(FONT_FOOTNOTE))
                            .wrap(),
                    );
                }
                ui.add_space(GRID * 2.0);
                ui.horizontal(|ui| {
                    if widgets::glass_button(ui, t_cancel, true).clicked() {
                        self.screen = Screen::Main;
                        self.confirm.clear();
                        self.error = None;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let ok = confirmation_matches(&name, &self.confirm);
                        if widgets::danger_button(ui, t_over, ok, 160.0).clicked() {
                            self.start_wipe();
                        }
                    });
                });
            });
    }

    fn countdown_modal(&mut self, ctx: &egui::Context) {
        let remaining = self.countdown.unwrap_or(5);
        let name = self
            .selected_snap()
            .map(|s| s.identity.display_name.clone())
            .unwrap_or_default();
        let model = self
            .selected_snap()
            .map(|s| display_model(s, self.lang))
            .unwrap_or_default();
        let heading = self.t("ui.countdown_of").replace("{name}", &name);
        let t_cancel = self.t("ui.cancel");
        let p = theme::pal();
        let t = theme::ease_out_cubic(self.modal_anim);
        dim_background(ctx, (t * 160.0) as u8);
        let offset = (1.0 - t) * 16.0;
        egui::Window::new("countdown")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, offset])
            .default_width(420.0)
            .min_width(360.0)
            .frame(theme::sheet_frame())
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            RichText::new(&heading)
                                .color(p.text_secondary)
                                .size(FONT_BODY),
                        )
                        .wrap(),
                    );
                    if !model.is_empty() {
                        ui.label(
                            RichText::new(&model)
                                .font(FontId::new(FONT_BODY, theme::medium()))
                                .color(p.text),
                        );
                    }
                    ui.add_space(GRID);
                    ui.label(
                        RichText::new(format!("{remaining}"))
                            .font(FontId::new(FONT_DISPLAY, theme::semibold()))
                            .color(p.text),
                    );
                    ui.add_space(GRID);
                    if widgets::glass_button(ui, t_cancel, true).clicked() {
                        self.cancel.store(true, Ordering::SeqCst);
                        if self.demo {
                            self.running = false;
                            self.screen = Screen::Main;
                            self.countdown = None;
                        }
                    }
                });
            });
    }
}

fn section_header(ui: &mut Ui, label: &str) {
    let p = theme::pal();
    ui.add_space(4.0);
    ui.label(
        RichText::new(label)
            .font(FontId::new(FONT_CAPTION, theme::semibold()))
            .color(p.text_tertiary),
    );
    ui.add_space(4.0);
}

fn header_block(ui: &mut Ui, icon: Icon, color: Color32, title: &str, body: &str) {
    let p = theme::pal();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        widgets::status_glyph(ui, icon, color, 36.0);
        ui.vertical(|ui| {
            ui.label(
                RichText::new(title)
                    .font(FontId::new(FONT_TITLE, theme::semibold()))
                    .color(if color == p.danger { p.danger } else { p.text }),
            );
            ui.add(
                egui::Label::new(RichText::new(body).color(p.text_secondary).size(FONT_BODY))
                    .wrap(),
            );
        });
    });
}

fn fact_strip(ui: &mut Ui, facts: &[(&str, &str, Option<&str>)], wide: bool) {
    let p = theme::pal();
    let per_row = if wide { facts.len().min(4).max(1) } else { 2 };
    let gap = GRID * 2.0;
    let total_w = ui.available_width();
    let cell_w = ((total_w - gap * (per_row as f32 - 1.0)) / per_row as f32).floor();
    for chunk in facts.chunks(per_row) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (k, v, sub) in chunk {
                ui.allocate_ui(Vec2::new(cell_w, 0.0), |ui| {
                    ui.set_width(cell_w);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(*k)
                                .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                                .color(p.text_tertiary),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(*v).color(p.text).size(FONT_BODY))
                                .truncate(),
                        )
                        .on_hover_text(*v);
                        if let Some(sub) = sub {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(*sub)
                                        .color(p.text_tertiary)
                                        .size(FONT_FOOTNOTE),
                                )
                                .truncate(),
                            );
                        }
                    });
                });
            }
        });
    }
}

fn stage_rail(ui: &mut Ui, p: &Progress, show_verify: bool, lang: Lang) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GRID;
        let writing = matches!(
            p.phase,
            Phase::Writing | Phase::Planning | Phase::Rechecking
        );
        let verifying = matches!(p.phase, Phase::Verifying | Phase::Syncing);
        let write_label = if p.passes > 1 {
            format!(
                "{} {}/{}",
                i18n::t(lang, "ui.stage.write"),
                p.pass,
                p.passes
            )
        } else {
            i18n::t(lang, "ui.stage.write").to_string()
        };
        stage_chip(ui, &write_label, writing, !verifying && !writing);
        if show_verify {
            let verify_label = i18n::t(lang, "ui.stage.verify");
            stage_chip(ui, verify_label, verifying, !verifying);
        }
    });
}

fn stage_chip(ui: &mut Ui, label: &str, active: bool, upcoming: bool) {
    let p = theme::pal();
    let (fg, bg, stroke) = if active {
        (p.text, p.accent_soft, p.accent)
    } else if upcoming {
        (p.text_tertiary, p.track, p.glass_stroke)
    } else {
        (p.text_secondary, p.glass, p.glass_stroke)
    };
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0_f32, stroke))
        .corner_radius(CornerRadius::same(RADIUS_PILL as u8))
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label)
                    .font(FontId::new(FONT_FOOTNOTE, theme::medium()))
                    .color(fg),
            );
        });
}

fn dim_background(ctx: &egui::Context, alpha: u8) {
    let rect = ctx.screen_rect();
    egui::Area::new(egui::Id::new("dim"))
        .fixed_pos(rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            let r = ui.allocate_rect(rect, Sense::click());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_black_alpha(alpha));
            let _ = r;
        });
}

fn handle_keys(app: &mut App, ctx: &egui::Context) {
    ctx.input(|i| {
        if i.key_pressed(egui::Key::Escape) {
            // handled below, need mut
        }
    });
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
    let up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::K));
    let down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::J));
    let refresh = ctx.input(|i| i.key_pressed(egui::Key::R) && i.modifiers.command);
    if refresh && !app.running {
        app.refresh();
    }
    if escape {
        match app.screen {
            Screen::Confirm => {
                app.screen = Screen::Main;
                app.confirm.clear();
            }
            Screen::Countdown | Screen::Work => {
                app.cancel.store(true, Ordering::SeqCst);
            }
            Screen::Report | Screen::Error => {
                app.screen = Screen::Main;
                app.report = None;
                app.error = None;
            }
            Screen::Main => {}
        }
    }
    if down && matches!(app.screen, Screen::Main) {
        app.move_selection(1);
    }
    if up && matches!(app.screen, Screen::Main) {
        app.move_selection(-1);
    }
    if enter && matches!(app.screen, Screen::Main) {
        app.try_open_confirm();
    }
    if enter && matches!(app.screen, Screen::Confirm) {
        if let Some(name) = app.selected_snap().map(|s| s.identity.display_name.clone()) {
            if confirmation_matches(&name, &app.confirm) {
                app.start_wipe();
            }
        }
    }
}

fn protection(snap: &TargetSnapshot) -> Option<&'static str> {
    if snap.is_system {
        Some("ui.protected_reason_system")
    } else if snap.is_app_disk {
        Some("ui.protected_reason_app")
    } else if snap.is_raid_member || snap.is_lvm_member {
        Some("ui.protected_reason_raid")
    } else {
        None
    }
}

fn display_model(snap: &TargetSnapshot, lang: Lang) -> String {
    snap.identity
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "-")
        .unwrap_or(i18n::t(lang, "ui.unknown_model"))
        .to_string()
}

fn rough_estimate(size: u64, method: MethodKind, verify: bool) -> String {
    let passes = match method {
        MethodKind::Zeros | MethodKind::Random => {
            if verify {
                2.0
            } else {
                1.0
            }
        }
        MethodKind::Multi => {
            if verify {
                4.0
            } else {
                3.0
            }
        }
    };
    let bps = 500.0 * 1024.0 * 1024.0;
    let secs = ((size as f64 * passes) / bps).ceil() as u64;
    format_eta(secs.max(1))
}

fn phase_key(phase: Phase) -> &'static str {
    match phase {
        Phase::Planning => "ui.phase.planning",
        Phase::Countdown => "ui.phase.countdown",
        Phase::Rechecking => "ui.phase.rechecking",
        Phase::Writing => "ui.phase.writing",
        Phase::Verifying => "ui.phase.verifying",
        Phase::Syncing => "ui.phase.syncing",
        Phase::Sanitizing => "ui.phase.sanitizing",
        Phase::Cancelling => "ui.phase.cancelling",
        Phase::Done => "ui.phase.done",
        Phase::Failed => "ui.phase.failed",
    }
}

fn demo_devices() -> Vec<DeviceInfo> {
    vec![
        demo_dev(
            "nvme1n1",
            "/dev/nvme1n1",
            "Samsung SSD 990 PRO 2TB",
            Some("S6NNXJ0T123456"),
            2_000_398_934_016,
            MediaKind::Nvme,
            false,
            false,
            vec![],
            false,
            false,
        ),
        demo_dev(
            "sdb",
            "/dev/sdb",
            "SanDisk USB 3.2",
            Some("AA0102030405"),
            124_000_000_000,
            MediaKind::Usb,
            false,
            false,
            vec![],
            false,
            false,
        ),
        demo_dev(
            "nvme0n1",
            "/dev/nvme0n1",
            "WD Blue SN580 1TB",
            Some("E823_8FA6_BF53"),
            1_000_204_886_016,
            MediaKind::Nvme,
            false,
            true,
            vec!["/".into()],
            true,
            false,
        ),
        demo_dev(
            "sda",
            "/dev/sda",
            "TOSHIBA MQ04ABF100",
            Some("Z8C0A1B2"),
            1_000_204_886_016,
            MediaKind::Hdd,
            true,
            false,
            vec!["/boot".into()],
            true,
            false,
        ),
    ]
}

fn demo_dev(
    name: &str,
    path: &str,
    model: &str,
    serial: Option<&str>,
    size: u64,
    kind: MediaKind,
    system: bool,
    app: bool,
    mounts: Vec<String>,
    in_use: bool,
    raid: bool,
) -> DeviceInfo {
    DeviceInfo {
        snapshot: TargetSnapshot {
            identity: DeviceIdentity {
                path: PathBuf::from(path),
                canonical_path: PathBuf::from(path),
                display_name: name.into(),
                model: Some(model.into()),
                serial: serial.map(|s| s.to_string()),
                wwn: None,
                size,
                file_id: Some((1, 1)),
                media: kind.label().to_ascii_lowercase(),
            },
            kind,
            is_system: system,
            is_app_disk: app,
            mounts,
            in_use,
            is_raid_member: raid,
            is_lvm_member: false,
            backing_file: None,
            is_symlink: false,
            is_regular_file: false,
            is_directory: false,
            is_block_device: true,
            logical_block_size: 512,
        },
    }
}

fn demo_report(ok: bool, id: DeviceIdentity) -> Report {
    Report {
        tool: "framewipe".into(),
        version: frame_wipe::VERSION.into(),
        started_utc: "2026-11-09T18:00:00Z".into(),
        finished_utc: "2026-11-09T18:38:12Z".into(),
        elapsed_ms: Some(2_292_000),
        target_path: id.path.display().to_string(),
        display_name: id.display_name,
        model: id.model,
        serial: id.serial,
        size: id.size,
        method: "zeros (verify)".into(),
        scope: "whole device".into(),
        bytes_written: if ok { id.size } else { id.size / 3 },
        verified: ok,
        outcome: if ok {
            Outcome::Success
        } else {
            Outcome::Failed
        },
        host: "demo".into(),
        bad_sectors: vec![],
        warnings: vec![],
        notes: if ok {
            vec![]
        } else {
            vec!["Stopped after a media error at offset 68719476736.".into()]
        },
    }
}
