//! Desktop UI for framewipe. Native Rust (egui/eframe + glow), no Electron.

use eframe::egui;
use frame_wipe::device::{self, DeviceInfo, EnumerateOptions};
use frame_wipe::engine::{wipe, WipeRequest};
use frame_wipe::i18n::{self, Lang};
use frame_wipe::identity::confirmation_matches;
use frame_wipe::method::Method;
use frame_wipe::planner::Scope;
use frame_wipe::progress::{Phase, Progress};
use frame_wipe::report::Report;
use frame_wipe::safety::SafetyFlags;
use frame_wipe::units::{format_bytes, format_eta, format_throughput};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let smoke = args.iter().any(|a| a == "--smoke-test");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([720.0, 480.0])
            .with_title("Frame Wipe"),
        ..Default::default()
    };
    eframe::run_native(
        "Frame Wipe",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(smoke)))),
    )
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
    show_confirm: bool,
    countdown: Option<u32>,
    cancel: Arc<AtomicBool>,
    progress: Option<Progress>,
    report: Option<Report>,
    error: Option<String>,
    running: bool,
    ev_rx: Option<Receiver<UiEvent>>,
    ev_tx: Option<Sender<UiEvent>>,
    smoke: bool,
    smoke_frames: u32,
}

enum UiEvent {
    Progress(Progress),
    Done(Result<Report, String>),
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

impl App {
    fn new(smoke: bool) -> Self {
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
            show_confirm: false,
            countdown: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: None,
            report: None,
            error: None,
            running: false,
            ev_rx: Some(rx),
            ev_tx: Some(tx),
            smoke,
            smoke_frames: 0,
        };
        app.refresh();
        app
    }

    fn t(&self, key: &str) -> &'static str {
        i18n::t(self.lang, key)
    }

    fn refresh(&mut self) {
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

    fn selected_device(&self) -> Option<&DeviceInfo> {
        self.selected.and_then(|i| self.devices.get(i))
    }

    fn start_wipe(&mut self) {
        let Some(dev) = self.selected_device() else {
            return;
        };
        if !confirmation_matches(&dev.snapshot.identity.display_name, &self.confirm) {
            self.error = Some(format!(
                "type the device name '{}' to confirm",
                dev.snapshot.identity.display_name
            ));
            return;
        }
        let method = match self.method {
            MethodKind::Zeros => {
                let mut m = Method::zeros();
                if !self.verify {
                    if let Method::Single { verify, .. } = &mut m {
                        *verify = false;
                    }
                }
                m
            }
            MethodKind::Random => {
                let mut m = Method::random();
                if !self.verify {
                    if let Method::Single { verify, .. } = &mut m {
                        *verify = false;
                    }
                }
                m
            }
            MethodKind::Multi => Method::multi_default(),
        };
        // Method::Single is not mutably matched if it's an enum without import - I used Method::Single
        // which needs to be in scope. I'll fix if compile fails by rebuilding method after parse.

        let path = dev.snapshot.identity.path.clone();
        let name = dev.snapshot.identity.display_name.clone();
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
        self.show_confirm = false;
        let cancel = self.cancel.clone();
        let tx = self.ev_tx.clone().unwrap();
        let method_for_thread = method;
        thread::spawn(move || {
            let req = WipeRequest {
                path,
                scope,
                method: method_for_thread,
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
            let _ = tx.send(UiEvent::Done(result));
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_theme(ctx);
        if self.smoke {
            self.smoke_frames += 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            if self.smoke_frames > 2 {
                // Guarantee CI can finish even if the close command is ignored.
                std::process::exit(0);
            }
        }
        if let Some(rx) = &self.ev_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    UiEvent::Progress(p) => {
                        self.countdown = p.countdown_remaining;
                        self.progress = Some(p);
                    }
                    UiEvent::Done(Ok(r)) => {
                        self.running = false;
                        self.report = Some(r);
                        self.progress = None;
                        self.countdown = None;
                    }
                    UiEvent::Done(Err(e)) => {
                        self.running = false;
                        self.error = Some(e);
                        self.progress = None;
                        self.countdown = None;
                    }
                }
            }
        }
        if self.running {
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.heading(self.t("app.name"));
                ui.label(egui::RichText::new(self.t("app.tagline")).color(STEEL_200));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    if ui.selectable_label(self.lang == Lang::Fr, "FR").clicked() {
                        self.lang = Lang::Fr;
                    }
                    if ui.selectable_label(self.lang == Lang::En, "EN").clicked() {
                        self.lang = Lang::En;
                    }
                    ui.label(self.t("ui.language"));
                });
            });
            ui.add_space(8.0);
        });

        egui::SidePanel::left("devices")
            .resizable(true)
            .min_width(280.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading(self.t("ui.devices"));
                    if ui.button(self.t("ui.refresh")).clicked() && !self.running {
                        self.refresh();
                    }
                });
                ui.add_space(6.0);
                if let Some(err) = &self.list_error {
                    ui.colored_label(CRITICAL, err);
                }
                if self.devices.is_empty() {
                    ui.label(self.t("ui.no_devices"));
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, d) in self.devices.iter().enumerate() {
                        let s = &d.snapshot;
                        let selected = self.selected == Some(i);
                        let label = format!(
                            "{}\n{}  {}",
                            s.identity.display_name,
                            s.identity.model.as_deref().unwrap_or("-"),
                            format_bytes(s.identity.size)
                        );
                        if ui.selectable_label(selected, label).clicked() && !self.running {
                            self.selected = Some(i);
                            self.confirm.clear();
                            self.error = None;
                            self.report = None;
                        }
                        ui.horizontal(|ui| {
                            badge(ui, &format!("{:?}", s.kind), STEEL_400);
                            if s.is_system {
                                badge(ui, self.t("ui.system"), CRITICAL);
                            }
                            if s.is_app_disk {
                                badge(ui, self.t("ui.app_disk"), WARNING);
                            }
                            if !s.mounts.is_empty() {
                                badge(ui, self.t("ui.mounted"), WARNING);
                            }
                            if s.in_use {
                                badge(ui, self.t("ui.in_use"), WARNING);
                            }
                            if s.is_raid_member || s.is_lvm_member {
                                badge(ui, self.t("ui.raid"), WARNING);
                            }
                        });
                        ui.add_space(6.0);
                    }
                });
            });

        let t_scope = self.t("ui.scope");
        let t_whole = self.t("ui.whole");
        let t_free = self.t("ui.freespace");
        let t_method = self.t("ui.method");
        let t_zeros = self.t("ui.zeros");
        let t_random = self.t("ui.random");
        let t_multi = self.t("ui.multi");
        let t_verify = self.t("ui.verify");
        let t_dry = self.t("ui.dry_run");
        let t_start = self.t("ui.start");
        let t_cancel = self.t("ui.cancel");
        let t_confirm_title = self.t("ui.confirm_title");
        let t_confirm_hint = self.t("ui.confirm_hint");
        let t_countdown = self.t("ui.countdown");
        let t_done = self.t("ui.done");
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            let selected = self.selected.and_then(|i| self.devices.get(i)).map(|d| {
                let s = &d.snapshot;
                (
                    s.identity.display_name.clone(),
                    s.identity.model.clone(),
                    s.identity.serial.clone(),
                    s.identity.size,
                    s.identity.path.display().to_string(),
                    s.mounts.clone(),
                )
            });
            if let Some((display_name, model, serial, size, path, mounts)) = selected {
                ui.heading(&display_name);
                ui.label(format!(
                    "{}    serial {}    {}",
                    model.as_deref().unwrap_or("unknown model"),
                    serial.as_deref().unwrap_or("none"),
                    format_bytes(size)
                ));
                ui.label(path);
                if !mounts.is_empty() {
                    ui.label(format!("mounts: {}", mounts.join(", ")));
                }
                ui.add_space(12.0);
                ui.label(t_scope);
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.scope, ScopeKind::Whole, t_whole);
                    ui.radio_value(&mut self.scope, ScopeKind::FreeSpace, t_free);
                });
                ui.add_space(8.0);
                ui.label(t_method);
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.method, MethodKind::Zeros, t_zeros);
                    ui.radio_value(&mut self.method, MethodKind::Random, t_random);
                    ui.radio_value(&mut self.method, MethodKind::Multi, t_multi);
                });
                ui.checkbox(&mut self.verify, t_verify);
                ui.checkbox(&mut self.dry_run, t_dry);
                ui.add_space(16.0);

                if self.running {
                    if let Some(c) = self.countdown {
                        ui.label(format!("{} {c}s", t_countdown));
                    }
                    if let Some(p) = &self.progress {
                        let frac = p.fraction() as f32;
                        ui.add(egui::ProgressBar::new(frac).show_percentage());
                        ui.label(format!(
                            "{:?}  {}  eta {}",
                            p.phase,
                            format_throughput(p.throughput_bps),
                            p.eta_secs.map(format_eta).unwrap_or_else(|| "--".into())
                        ));
                    }
                    if ui.button(t_cancel).clicked() {
                        self.cancel.store(true, Ordering::SeqCst);
                    }
                } else if self.show_confirm {
                    ui.colored_label(CRITICAL, t_confirm_title);
                    ui.label(t_confirm_hint);
                    ui.label(format!("name: {display_name}"));
                    ui.text_edit_singleline(&mut self.confirm);
                    ui.horizontal(|ui| {
                        if ui.button(t_cancel).clicked() {
                            self.show_confirm = false;
                            self.confirm.clear();
                        }
                        let ready = confirmation_matches(&display_name, &self.confirm);
                        if ui.add_enabled(ready, egui::Button::new(t_start)).clicked() {
                            self.start_wipe();
                        }
                    });
                } else {
                    if ui.button(t_start).clicked() {
                        self.show_confirm = true;
                        self.confirm.clear();
                    }
                }
            } else {
                ui.label("Select a device.");
            }

            if let Some(err) = &self.error {
                ui.add_space(12.0);
                ui.colored_label(CRITICAL, err);
            }
            if let Some(r) = &self.report {
                ui.add_space(12.0);
                ui.heading(t_done);
                ui.label(format!("{:?}    verified={}", r.outcome, r.verified));
                ui.monospace(r.to_text());
            }
        });
        let _ = Phase::Done;
    }
}

const BG: egui::Color32 = egui::Color32::from_rgb(0, 0, 0);
const SURFACE: egui::Color32 = egui::Color32::from_rgb(8, 8, 9);
const TEXT: egui::Color32 = egui::Color32::from_rgb(255, 255, 255);
const STEEL_200: egui::Color32 = egui::Color32::from_rgb(185, 194, 216);
const STEEL_400: egui::Color32 = egui::Color32::from_rgb(140, 153, 180);
const CRITICAL: egui::Color32 = egui::Color32::from_rgb(207, 138, 130);
const WARNING: egui::Color32 = egui::Color32::from_rgb(200, 165, 113);

fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.dark_mode = true;
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(16, 16, 18);
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(28, 28, 32);
    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(36, 36, 40);
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(63, 79, 106);
    style.visuals.widgets.noninteractive.fg_stroke.color = STEEL_200;
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    ctx.set_style(style);
}

fn badge(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.label(egui::RichText::new(text).small().color(color));
}
