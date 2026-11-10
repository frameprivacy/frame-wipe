//! Desktop UI for framewipe. Native Rust (egui + glow). No Electron.

mod theme;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Pos2, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2,
};
use frame_wipe::device::{self, DeviceInfo, EnumerateOptions};
use frame_wipe::engine::{wipe, WipeRequest};
use frame_wipe::i18n::{self, Lang};
use frame_wipe::identity::{confirmation_matches, DeviceIdentity};
use frame_wipe::method::Method;
use frame_wipe::planner::{MediaKind, Scope, TargetSnapshot};
use frame_wipe::progress::{Phase, Progress};
use frame_wipe::report::{Outcome, Report};
use frame_wipe::safety::SafetyFlags;
use frame_wipe::units::{format_bytes, format_eta, format_throughput};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use theme::{FONT_BODY, FONT_HEAD, FONT_SMALL, FONT_TITLE, GRID};

fn main() -> eframe::Result<()> {
    let args = Args::parse();
    let scale = args.scale;
    let size = [args.width, args.height];
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_min_inner_size([1024.0, 700.0])
            .with_title("Frame Wipe")
            .with_decorations(!args.undecorated)
            .with_maximized(false),
        ..Default::default()
    };
    #[cfg(windows)]
    {
        // Prefer DirectX / WARP; glow requires OpenGL 2.0, which Windows CI lacks.
        options.renderer = eframe::Renderer::Wgpu;
    }
    let smoke = args.smoke;
    let args_fallback = args.clone();
    match eframe::run_native(
        "Frame Wipe",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx);
            cc.egui_ctx.set_pixels_per_point(scale);
            Ok(Box::new(App::new(args)))
        }),
    ) {
        Ok(()) => Ok(()),
        Err(err) if smoke => {
            // Windows CI (and some other headless hosts) have no OpenGL or GPU
            // adapter. Still exercise layout, fonts, and screens without a window.
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
    theme::apply(&ctx);
    ctx.set_pixels_per_point(args.scale);
    let mut app = App::new(args.clone());
    app.smoke = false; // do not process::exit; return to the caller
    let size = egui::vec2(args.width, args.height);
    for _ in 0..3 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            app.tick(ctx);
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
}

impl App {
    fn new(args: Args) -> Self {
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
        };
        if args.demo {
            app.apply_demo(args.demo_state);
        } else {
            app.refresh();
        }
        app
    }

    fn t(&self, key: &str) -> &'static str {
        i18n::t(self.lang, key)
    }

    fn refresh(&mut self) {
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
        if matches!(self.screen, Screen::Report) {
            self.screen = Screen::Main;
            self.report = None;
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
    }

    fn start_wipe(&mut self) {
        let Some(dev) = self.selected.and_then(|i| self.devices.get(i)) else {
            return;
        };
        let name = dev.snapshot.identity.display_name.clone();
        if !confirmation_matches(&name, &self.confirm) {
            self.error = Some(format!("Type {name} to confirm."));
            return;
        }
        if self.demo {
            self.screen = Screen::Countdown;
            self.countdown = Some(5);
            self.running = true;
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
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick(ctx);
    }
}

impl App {
    fn tick(&mut self, ctx: &egui::Context) {
        ctx.set_pixels_per_point(self.scale);
        theme::apply(ctx);
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
                            self.screen = Screen::Main;
                            self.progress = None;
                        }
                    }
                }
            }
        }
        if self.running {
            ctx.request_repaint();
        }

        handle_keys(self, ctx);

        let header_h = GRID * 5.5;
        let pad = GRID * 2.0;
        let screen = ctx.screen_rect();

        // Full-window background (no leftover gaps).
        ctx.layer_painter(egui::LayerId::background())
            .rect_filled(screen, 0.0, theme::BG);

        egui::TopBottomPanel::top("header")
            .exact_height(header_h)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin {
                        left: pad as i8,
                        right: pad as i8,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ctx, |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), header_h),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.label(
                            RichText::new(self.t("app.name"))
                                .font(FontId::new(FONT_HEAD, theme::semibold()))
                                .color(theme::TEXT),
                        );
                        ui.add_space(GRID);
                        ui.label(
                            RichText::new(self.t("app.tagline"))
                                .font(FontId::new(FONT_SMALL, theme::medium()))
                                .color(theme::STEEL_200),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let t_refresh = self.t("ui.refresh");
                            if text_btn(ui, t_refresh).clicked() && !self.running {
                                self.refresh();
                            }
                            ui.add_space(GRID);
                            lang_toggle(ui, &mut self.lang);
                        });
                    },
                );
                let r = ui.max_rect();
                ui.painter().hline(
                    r.x_range(),
                    r.bottom() - 0.5,
                    Stroke::new(1.0_f32, theme::BORDER),
                );
            });

        let remain = ctx.available_rect().width();
        let left_w = (remain * 0.34).clamp(320.0, 560.0);
        egui::SidePanel::left("device-list")
            .exact_width(left_w)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin {
                        left: pad as i8,
                        right: GRID as i8,
                        top: GRID as i8,
                        bottom: pad as i8,
                    }),
            )
            .show(ctx, |ui| {
                self.left_column(ui);
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin {
                        left: (GRID * 2.0) as i8,
                        right: pad as i8,
                        top: GRID as i8,
                        bottom: pad as i8,
                    }),
            )
            .show(ctx, |ui| {
                self.right_column(ui);
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
}

impl App {
    fn left_column(&mut self, ui: &mut Ui) {
        let avail_h = ui.available_height();
        ui.vertical(|ui| {
            ui.set_min_height(avail_h);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(self.t("ui.devices"))
                        .font(FontId::new(FONT_HEAD, theme::semibold()))
                        .color(theme::TEXT),
                );
                ui.label(
                    RichText::new(format!("{}", self.devices.len()))
                        .font(FontId::new(FONT_SMALL, theme::medium()))
                        .color(theme::STEEL_400),
                );
            });
            ui.add_space(GRID);
            if let Some(err) = &self.list_error {
                theme::card_frame().show(ui, |ui| {
                    ui.label(RichText::new(self.t("ui.failed")).color(theme::CRITICAL));
                    ui.label(RichText::new(err).color(theme::STEEL_200).size(FONT_BODY));
                });
                return;
            }
            if self.devices.is_empty() {
                theme::card_frame().show(ui, |ui| {
                    ui.label(
                        RichText::new(self.t("ui.empty"))
                            .color(theme::STEEL_200)
                            .size(FONT_BODY),
                    );
                });
                return;
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(avail_h - GRID * 5.0)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = GRID;
                    let n = self.devices.len();
                    for i in 0..n {
                        self.device_card(ui, i);
                    }
                });
        });
    }

    fn device_card(&mut self, ui: &mut Ui, index: usize) {
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
        let kind = snap.kind.label();
        let size = format_bytes(snap.identity.size);
        let name = snap.identity.display_name.clone();

        let fill = if selected {
            theme::SURFACE_RAISED
        } else {
            theme::SURFACE
        };
        let stroke = if selected {
            Stroke::new(1.0_f32, theme::STEEL_400)
        } else {
            Stroke::new(1.0_f32, theme::BORDER)
        };
        let mut frame = egui::Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric((GRID * 1.5) as i8, GRID as i8));

        if blocked.is_some() {
            frame = frame.fill(Color32::from_rgb(10, 10, 11));
        }

        let sense = if blocked.is_some() {
            Sense::hover()
        } else {
            Sense::click()
        };
        let inner = frame.show(ui, |ui| {
            let inner_w = ui.available_width();
            ui.set_min_width(inner_w);
            ui.set_width(inner_w);
            if blocked.is_some() {
                ui.set_opacity(0.48);
            }
            // Title and size on one reserved row so right-to-left layout cannot
            // stretch the frame or push the tag row outside the border.
            let title_h = 20.0;
            let (title_rect, _) =
                ui.allocate_exact_size(Vec2::new(inner_w, title_h), Sense::hover());
            ui.painter().text(
                Pos2::new(title_rect.left(), title_rect.center().y),
                Align2::LEFT_CENTER,
                &model,
                FontId::new(FONT_BODY, theme::semibold()),
                theme::TEXT,
            );
            ui.painter().text(
                Pos2::new(title_rect.right(), title_rect.center().y),
                Align2::RIGHT_CENTER,
                &size,
                FontId::new(FONT_BODY, theme::medium()),
                theme::STEEL_50,
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!("{name}  ·  {serial}  ·  {kind}"))
                    .font(FontId::new(FONT_SMALL, font_family_prop()))
                    .color(theme::STEEL_400),
            );
            ui.add_space(6.0);
            // Reserved tag row: the frame min_rect always includes this height,
            // so the border encloses the pills even when they wrap.
            let tag_h = 22.0;
            ui.allocate_ui_with_layout(
                Vec2::new(inner_w, tag_h),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_min_size(Vec2::new(inner_w, tag_h));
                    ui.spacing_mut().item_spacing.x = 6.0;
                    pill(ui, kind, theme::STEEL_400, theme::SURFACE_RAISED);
                    if snap.is_system {
                        pill(ui, self.t("ui.system"), theme::CRITICAL, theme::CRITICAL_BG);
                    }
                    if snap.is_app_disk {
                        pill(ui, self.t("ui.app_disk"), theme::WARNING, theme::WARNING_BG);
                    }
                    if !snap.mounts.is_empty() {
                        pill(ui, self.t("ui.mounted"), theme::WARNING, theme::WARNING_BG);
                    }
                    if snap.in_use && snap.mounts.is_empty() {
                        pill(ui, self.t("ui.in_use"), theme::WARNING, theme::WARNING_BG);
                    }
                    if snap.is_raid_member || snap.is_lvm_member {
                        pill(ui, self.t("ui.raid"), theme::WARNING, theme::WARNING_BG);
                    }
                },
            );
        });
        let resp = inner.response.interact(sense);
        if blocked.is_some() && resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
        }

        if let Some(reason_key) = blocked {
            resp.on_hover_text(self.t(reason_key));
        } else if resp.clicked() && !self.running {
            self.selected = Some(index);
            self.error = None;
            if matches!(self.screen, Screen::Report) {
                self.screen = Screen::Main;
                self.report = None;
            }
        }
    }

    fn right_column(&mut self, ui: &mut Ui) {
        let h = ui.available_height();
        match self.screen {
            Screen::Work => self.progress_view(ui, h),
            Screen::Report => self.report_view(ui, h),
            _ => {
                if self.selected_snap().is_some() {
                    self.detail_selected(ui, h);
                } else {
                    self.detail_overview(ui, h);
                }
            }
        }
    }

    fn detail_overview(&self, ui: &mut Ui, h: f32) {
        let w = ui.available_width();
        ui.allocate_ui(Vec2::new(w, h), |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.set_width(w);
            ui.label(
                RichText::new(self.t("ui.drives_summary"))
                    .font(FontId::new(FONT_TITLE, theme::semibold()))
                    .color(theme::TEXT),
            );
            let total = self.devices.len();
            let open = self.selectable_indices().len();
            let protected = total.saturating_sub(open);
            if open > 0 {
                ui.label(
                    RichText::new(self.t("ui.select_hint"))
                        .color(theme::STEEL_400)
                        .size(FONT_BODY),
                );
            }
            ui.add_space(GRID);
            stat_row(
                ui,
                w,
                &[
                    (
                        format!("{open}"),
                        self.t("ui.can_overwrite"),
                        theme::RESOLVED,
                    ),
                    (
                        format!("{protected}"),
                        self.t("ui.protected_count"),
                        theme::WARNING,
                    ),
                    (format!("{total}"), self.t("ui.devices"), theme::STEEL_200),
                ],
            );
            ui.add_space(GRID * 2.0);
            if total == 0 && self.list_error.is_none() {
                ui.label(
                    RichText::new(self.t("ui.empty"))
                        .color(theme::STEEL_200)
                        .size(FONT_BODY),
                );
                ui.add_space(GRID);
            } else if open == 0 && total > 0 {
                ui.label(
                    RichText::new(self.t("ui.none_available"))
                        .color(theme::STEEL_200)
                        .size(FONT_BODY),
                );
                ui.add_space(GRID);
            }

            let two_col = w >= 720.0 && protected > 0;
            if two_col {
                let gap = GRID * 2.0;
                let col_w = ((w - gap) / 2.0).floor();
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui(Vec2::new(col_w, 0.0), |ui| {
                        fill_card(ui, col_w, 0.0, |ui| self.protected_list(ui));
                    });
                    ui.allocate_ui(Vec2::new(col_w, 0.0), |ui| {
                        fill_card(ui, col_w, 0.0, |ui| self.tips_list(ui));
                    });
                });
            } else {
                if protected > 0 {
                    fill_card(ui, w, 0.0, |ui| self.protected_list(ui));
                    ui.add_space(GRID * 2.0);
                }
                fill_card(ui, w, 0.0, |ui| self.tips_list(ui));
            }
        });
    }

    fn protected_list(&self, ui: &mut Ui) {
        ui.label(
            RichText::new(self.t("ui.protected")).font(FontId::new(FONT_BODY, theme::semibold())),
        );
        ui.add_space(GRID * 0.5);
        for d in &self.devices {
            if protection(&d.snapshot).is_some() {
                let model = display_model(&d.snapshot, self.lang);
                let why = protection(&d.snapshot).map(|k| self.t(k)).unwrap_or("");
                ui.label(
                    RichText::new(format!(
                        "{}  ·  {}",
                        d.snapshot.identity.display_name, model
                    ))
                    .color(theme::TEXT)
                    .size(FONT_BODY),
                );
                ui.label(RichText::new(why).color(theme::STEEL_400).size(FONT_SMALL));
                ui.add_space(GRID * 0.5);
            }
        }
    }

    fn tips_list(&self, ui: &mut Ui) {
        ui.label(RichText::new(self.t("ui.tips")).font(FontId::new(FONT_BODY, theme::semibold())));
        ui.add_space(GRID);
        for key in ["ui.tip1", "ui.tip2", "ui.tip3"] {
            // A plain horizontal label does not wrap and gets clipped in the card.
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("·").color(theme::STEEL_400));
                ui.add(
                    egui::Label::new(
                        RichText::new(self.t(key))
                            .color(theme::STEEL_200)
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
        let footer_h = GRID * 8.0;
        let body_h = (h - footer_h).max(120.0);
        let two_col = w >= 560.0;

        ui.allocate_ui(Vec2::new(w, h), |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.set_width(w);
            ui.allocate_ui(Vec2::new(w, body_h), |ui| {
                ui.set_width(w);
                ui.set_max_height(body_h);
                ui.label(
                    RichText::new(&model)
                        .font(FontId::new(FONT_TITLE, theme::semibold()))
                        .color(theme::TEXT),
                );
                ui.label(
                    RichText::new(format!(
                        "{}  ·  {}  ·  {}",
                        snap.identity.display_name,
                        snap.kind.label(),
                        format_bytes(snap.identity.size)
                    ))
                    .color(theme::STEEL_200)
                    .size(FONT_BODY),
                );
                ui.add_space(GRID);
                if mounted && self.scope == ScopeKind::Whole {
                    ui.label(
                        RichText::new(self.t("ui.mounted_hint"))
                            .color(theme::WARNING)
                            .size(FONT_BODY),
                    );
                    ui.add_space(GRID);
                }
                let size_txt = format!(
                    "{} ({} bytes)",
                    format_bytes(snap.identity.size),
                    snap.identity.size
                );
                let path_txt = snap.identity.path.display().to_string();
                let mut facts: Vec<(&str, &str)> = vec![
                    (t_serial, serial.as_str()),
                    (t_path, path_txt.as_str()),
                    (t_size, size_txt.as_str()),
                    (t_type, snap.kind.label()),
                ];
                let mounts_txt = snap.mounts.join(", ");
                if mounted {
                    facts.push((t_mounted, mounts_txt.as_str()));
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(ui.available_height())
                    .show(ui, |ui| {
                        ui.set_width(w);
                        fill_card(ui, w, 0.0, |ui| fact_strip(ui, &facts, two_col));
                        ui.add_space(GRID * 2.0);
                        let est = rough_estimate(snap.identity.size, self.method, self.verify);
                        let choices = |ui: &mut Ui| {
                            ui.label(
                                RichText::new(t_scope)
                                    .font(FontId::new(FONT_SMALL, theme::medium()))
                                    .color(theme::STEEL_400),
                            );
                            ui.horizontal(|ui| {
                                scope_choice(ui, &mut self.scope, ScopeKind::Whole, t_whole);
                                scope_choice(ui, &mut self.scope, ScopeKind::FreeSpace, t_free);
                            });
                            ui.add_space(GRID);
                            ui.label(
                                RichText::new(t_method)
                                    .font(FontId::new(FONT_SMALL, theme::medium()))
                                    .color(theme::STEEL_400),
                            );
                            ui.horizontal(|ui| {
                                method_choice(ui, &mut self.method, MethodKind::Zeros, t_zeros);
                                method_choice(ui, &mut self.method, MethodKind::Random, t_random);
                                method_choice(ui, &mut self.method, MethodKind::Multi, t_multi);
                            });
                            ui.add_space(GRID);
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.verify, t_verify);
                                ui.add_space(GRID * 2.0);
                                ui.checkbox(&mut self.dry_run, t_dry);
                            });
                        };
                        let estimate = |ui: &mut Ui| {
                            ui.label(
                                RichText::new(t_est)
                                    .font(FontId::new(FONT_SMALL, theme::medium()))
                                    .color(theme::STEEL_400),
                            );
                            ui.label(
                                RichText::new(&est)
                                    .font(FontId::new(FONT_TITLE, theme::semibold()))
                                    .color(theme::TEXT),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(t_est_note)
                                        .color(theme::STEEL_400)
                                        .size(FONT_SMALL),
                                )
                                .wrap(),
                            );
                        };
                        if two_col {
                            let gap = GRID * 2.0;
                            let left_w = ((w - gap) * 0.6).floor();
                            let right_w = w - gap - left_w;
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                ui.allocate_ui(Vec2::new(left_w, 0.0), |ui| {
                                    fill_card(ui, left_w, 0.0, choices);
                                });
                                ui.allocate_ui(Vec2::new(right_w, 0.0), |ui| {
                                    fill_card(ui, right_w, 0.0, estimate);
                                });
                            });
                        } else {
                            fill_card(ui, w, 0.0, choices);
                            ui.add_space(GRID * 2.0);
                            fill_card(ui, w, 0.0, estimate);
                        }
                    });
            });
            ui.allocate_ui_with_layout(
                Vec2::new(w, footer_h),
                Layout::bottom_up(Align::Min),
                |ui| {
                    ui.set_width(w);
                    let enabled =
                        !blocked && !self.running && !(mounted && self.scope == ScopeKind::Whole);
                    let btn = danger_button_width(ui, t_over, enabled, w);
                    if btn.clicked() {
                        self.try_open_confirm();
                    }
                    if let Some(err) = &self.error {
                        ui.add_space(GRID);
                        ui.label(RichText::new(err).color(theme::CRITICAL).size(FONT_BODY));
                    }
                    ui.add_space(GRID);
                    let r = ui.max_rect();
                    ui.painter().hline(
                        r.x_range(),
                        r.top() + 0.5,
                        Stroke::new(1.0_f32, theme::BORDER),
                    );
                },
            );
        });
    }

    fn progress_view(&mut self, ui: &mut Ui, h: f32) {
        let t_cancel = self.t("ui.cancel");
        let title = self
            .selected_snap()
            .map(|s| display_model(s, self.lang))
            .unwrap_or_else(|| self.t("ui.working").to_string());
        let w = ui.available_width();
        let footer_h = GRID * 8.0;
        ui.allocate_ui(Vec2::new(w, h), |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.label(RichText::new(&title).font(FontId::new(FONT_TITLE, theme::semibold())));
            ui.label(
                RichText::new(self.t("ui.working"))
                    .color(theme::STEEL_200)
                    .size(FONT_BODY),
            );
            ui.add_space(GRID * 2.0);
            if let Some(p) = &self.progress {
                let frac = p.fraction() as f32;
                wide_card(ui, |ui| {
                    let bar_w = ui.available_width();
                    let (bar_rect, _) =
                        ui.allocate_exact_size(Vec2::new(bar_w, 14.0), Sense::hover());
                    ui.painter()
                        .rect_filled(bar_rect, CornerRadius::same(7), theme::STEEL_700);
                    let mut fill_rect = bar_rect;
                    let fill_w = bar_rect.width() * frac.clamp(0.0, 1.0);
                    fill_rect.set_width(fill_w);
                    if fill_w > 0.5 {
                        ui.painter().rect_filled(
                            fill_rect,
                            CornerRadius::same(7),
                            theme::STEEL_400,
                        );
                    }
                    ui.add_space(GRID);
                    ui.label(
                        RichText::new(format!(
                            "{:.0}%  ·  {} of {}  ·  pass {}/{}",
                            frac * 100.0,
                            format_bytes(p.bytes_done),
                            format_bytes(p.bytes_total),
                            p.pass,
                            p.passes
                        ))
                        .size(FONT_BODY)
                        .color(theme::TEXT),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{}  ·  eta {}",
                            format_throughput(p.throughput_bps),
                            p.eta_secs.map(format_eta).unwrap_or_else(|| "--".into())
                        ))
                        .color(theme::STEEL_200)
                        .size(FONT_BODY),
                    );
                });
            }
            ui.add_space((ui.available_height() - footer_h).max(GRID));
            if text_btn(ui, t_cancel).clicked() {
                self.cancel.store(true, Ordering::SeqCst);
            }
        });
    }

    fn report_view(&mut self, ui: &mut Ui, h: f32) {
        let t_back = self.t("ui.back");
        let Some(r) = self.report.clone() else { return };
        let w = ui.available_width();
        let footer_h = GRID * 8.0;
        ui.allocate_ui(Vec2::new(w, h), |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            let title = match r.outcome {
                Outcome::Success | Outcome::DryRun => self.t("ui.report_title"),
                Outcome::Cancelled => self.t("ui.cancel"),
                Outcome::Failed => self.t("ui.failed"),
            };
            ui.label(
                RichText::new(title)
                    .font(FontId::new(FONT_TITLE, theme::semibold()))
                    .color(if r.outcome == Outcome::Failed {
                        theme::CRITICAL
                    } else {
                        theme::TEXT
                    }),
            );
            ui.add_space(GRID);
            let card_h = (ui.available_height() - footer_h).max(120.0);
            let inner_h = (card_h - 28.0).max(80.0);
            ui.allocate_ui(Vec2::new(w, card_h), |ui| {
                theme::card_frame().show(ui, |ui| {
                    ui.set_min_width((w - 26.0).max(0.0));
                    ui.set_min_height(inner_h);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .min_scrolled_height(inner_h)
                        .max_height(inner_h)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(r.to_text())
                                    .font(FontId::new(FONT_BODY, egui::FontFamily::Monospace))
                                    .color(theme::STEEL_200),
                            );
                        });
                });
            });
            ui.add_space((ui.available_height() - GRID * 5.0).max(GRID));
            if text_btn(ui, t_back).clicked() {
                self.screen = Screen::Main;
                self.report = None;
                self.progress = None;
            }
        });
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let name = self
            .selected_snap()
            .map(|s| s.identity.display_name.clone())
            .unwrap_or_default();
        let title = self.t("ui.confirm_title");
        let hint = self.t("ui.confirm_hint");
        let t_cancel = self.t("ui.cancel");
        let t_over = self.t("ui.overwrite");
        dim_background(ctx);
        egui::Window::new("confirm")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([440.0, 260.0])
            .frame(theme::raised_frame())
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(title)
                        .font(FontId::new(FONT_HEAD, theme::semibold()))
                        .color(theme::CRITICAL),
                );
                ui.add_space(GRID);
                ui.label(RichText::new(hint).color(theme::STEEL_200).size(FONT_BODY));
                ui.add_space(GRID);
                ui.label(
                    RichText::new(format!("{}: {name}", self.t("ui.confirm_name")))
                        .color(theme::TEXT)
                        .size(FONT_BODY),
                );
                ui.add_space(GRID);
                let hint = self.t("ui.confirm_name");
                ui.add(
                    egui::TextEdit::singleline(&mut self.confirm)
                        .hint_text(hint)
                        .desired_width(ui.available_width())
                        .font(FontId::new(FONT_BODY, theme::medium())),
                );
                if let Some(err) = &self.error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(err).color(theme::CRITICAL).size(FONT_SMALL));
                }
                ui.add_space(GRID * 2.0);
                ui.horizontal(|ui| {
                    if text_btn(ui, t_cancel).clicked() {
                        self.screen = Screen::Main;
                        self.confirm.clear();
                        self.error = None;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let ok = confirmation_matches(&name, &self.confirm);
                        if danger_button(ui, t_over, ok).clicked() {
                            self.start_wipe();
                        }
                    });
                });
            });
    }

    fn countdown_modal(&mut self, ctx: &egui::Context) {
        let remaining = self.countdown.unwrap_or(5);
        let t_cd = self.t("ui.countdown");
        let t_cancel = self.t("ui.cancel");
        dim_background(ctx);
        egui::Window::new("countdown")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([360.0, 180.0])
            .frame(theme::raised_frame())
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(t_cd).color(theme::STEEL_200).size(FONT_BODY));
                    ui.add_space(GRID);
                    ui.label(
                        RichText::new(format!("{remaining}"))
                            .font(FontId::new(56.0, theme::semibold()))
                            .color(theme::TEXT),
                    );
                    ui.add_space(GRID);
                    if text_btn(ui, t_cancel).clicked() {
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

/// Label/value pairs as one compact row (or a 2-column grid when narrow).
fn fact_strip(ui: &mut Ui, facts: &[(&str, &str)], wide: bool) {
    let per_row = if wide { facts.len().min(4).max(1) } else { 2 };
    let gap = GRID * 2.0;
    let total_w = ui.available_width();
    let cell_w = ((total_w - gap * (per_row as f32 - 1.0)) / per_row as f32).floor();
    for chunk in facts.chunks(per_row) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (k, v) in chunk {
                ui.allocate_ui(Vec2::new(cell_w, 0.0), |ui| {
                    ui.set_width(cell_w);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(*k)
                                .font(FontId::new(FONT_SMALL, theme::medium()))
                                .color(theme::STEEL_400),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(*v).color(theme::TEXT).size(FONT_BODY))
                                .wrap(),
                        );
                    });
                });
            }
        });
    }
}

fn wide_card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    fill_card(ui, w, 0.0, add);
}

/// Card that occupies a fixed width and, when `h` is greater than 0, a fixed height.
fn fill_card(ui: &mut Ui, w: f32, h: f32, add: impl FnOnce(&mut Ui)) {
    let height = if h > 0.0 { h } else { 0.0 };
    ui.allocate_ui_with_layout(Vec2::new(w, height), Layout::top_down(Align::Min), |ui| {
        if h > 0.0 {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
        }
        ui.set_width(w);
        theme::card_frame().show(ui, |ui| {
            let inner_w = (w - 26.0).max(0.0);
            ui.set_min_width(inner_w);
            if h > 0.0 {
                ui.set_min_height((h - 26.0).max(0.0));
            }
            ui.set_width(ui.available_width());
            add(ui);
        });
    });
}

fn font_family_prop() -> egui::FontFamily {
    egui::FontFamily::Proportional
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
            Screen::Report => {
                app.screen = Screen::Main;
                app.report = None;
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

fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .font(FontId::new(11.0, theme::medium()))
                    .color(fg),
            );
        });
}

fn stat_row(ui: &mut Ui, w: f32, items: &[(String, &str, Color32)]) {
    let n = items.len().max(1) as f32;
    let gap = GRID;
    let chip_w = ((w - gap * (n - 1.0)) / n).floor().max(80.0);
    let chip_h = GRID * 11.0;
    ui.allocate_ui_with_layout(
        Vec2::new(w, chip_h),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.set_height(chip_h);
            for (val, lab, col) in items {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(chip_w, chip_h), Sense::hover());
                ui.painter().rect(
                    rect,
                    CornerRadius::same(8),
                    theme::SURFACE,
                    Stroke::new(1.0_f32, theme::BORDER),
                    StrokeKind::Inside,
                );
                let pad = 12.0;
                ui.painter().text(
                    Pos2::new(rect.left() + pad, rect.top() + pad),
                    Align2::LEFT_TOP,
                    val,
                    FontId::new(FONT_TITLE, theme::semibold()),
                    *col,
                );
                ui.painter().text(
                    Pos2::new(rect.left() + pad, rect.top() + pad + 28.0),
                    Align2::LEFT_TOP,
                    *lab,
                    FontId::new(FONT_SMALL, font_family_prop()),
                    theme::STEEL_400,
                );
            }
        },
    );
}

fn text_btn(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(theme::STEEL_50).size(FONT_BODY))
            .fill(theme::SURFACE_RAISED)
            .stroke(Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(CornerRadius::same(8))
            .min_size(Vec2::new(0.0, GRID * 4.0)),
    )
}

fn danger_button(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    let w = ui.available_width().max(160.0);
    danger_button_width(ui, label, enabled, w)
}

fn danger_button_width(ui: &mut Ui, label: &str, enabled: bool, w: f32) -> egui::Response {
    let h = GRID * 5.5;
    // Allocate the exact panel width first so the button cannot shrink to
    // content size or sit at half width.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Center)),
        |ui| {
            ui.add_enabled(
                enabled,
                egui::Button::new(
                    RichText::new(label)
                        .font(FontId::new(FONT_BODY, theme::semibold()))
                        .color(if enabled { theme::BG } else { theme::STEEL_400 }),
                )
                .fill(if enabled {
                    theme::CRITICAL
                } else {
                    theme::SURFACE_RAISED
                })
                .corner_radius(CornerRadius::same(8))
                .min_size(Vec2::new(w, h)),
            )
        },
    )
    .inner
}

fn scope_choice(ui: &mut Ui, current: &mut ScopeKind, value: ScopeKind, label: &str) {
    let on = *current == value;
    let r = ui.add(
        egui::Button::new(RichText::new(label).color(if on {
            theme::TEXT
        } else {
            theme::STEEL_200
        }))
        .fill(if on {
            theme::STEEL_700
        } else {
            theme::SURFACE_RAISED
        })
        .stroke(Stroke::new(
            1.0_f32,
            if on { theme::STEEL_400 } else { theme::BORDER },
        ))
        .corner_radius(CornerRadius::same(8)),
    );
    if r.clicked() {
        *current = value;
    }
}

fn method_choice(ui: &mut Ui, current: &mut MethodKind, value: MethodKind, label: &str) {
    let on = *current == value;
    let r = ui.add(
        egui::Button::new(RichText::new(label).color(if on {
            theme::TEXT
        } else {
            theme::STEEL_200
        }))
        .fill(if on {
            theme::STEEL_700
        } else {
            theme::SURFACE_RAISED
        })
        .stroke(Stroke::new(
            1.0_f32,
            if on { theme::STEEL_400 } else { theme::BORDER },
        ))
        .corner_radius(CornerRadius::same(8)),
    );
    if r.clicked() {
        *current = value;
    }
}

fn lang_toggle(ui: &mut Ui, lang: &mut Lang) {
    for (l, label) in [(Lang::En, "EN"), (Lang::Fr, "FR")] {
        let on = *lang == l;
        if ui
            .add(
                egui::Button::new(RichText::new(label).size(FONT_SMALL).color(if on {
                    theme::TEXT
                } else {
                    theme::STEEL_400
                }))
                .fill(if on {
                    theme::STEEL_700
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(CornerRadius::same(6))
                .min_size(Vec2::new(36.0, 24.0)),
            )
            .clicked()
        {
            *lang = l;
        }
    }
}

fn dim_background(ctx: &egui::Context) {
    let rect = ctx.screen_rect();
    egui::Area::new(egui::Id::new("dim"))
        .fixed_pos(rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            let r = ui.allocate_rect(rect, Sense::click());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_black_alpha(180));
            let _ = r;
        });
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
            MediaKind::Ssd,
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
        scope: "wholedevice".into(),
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
