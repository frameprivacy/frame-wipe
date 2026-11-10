#![allow(clippy::format_in_format_args, clippy::map_identity)]

//! Argument parsing for `framewipe`. Kept in a library so tests and fuzzing can
//! call it without starting a process or exiting on error.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LangOpt {
    En,
    Fr,
}

#[derive(Debug, Parser)]
#[command(
    name = "framewipe",
    version,
    about = "Overwrite disks, partitions, free space, or files.",
    long_about = "framewipe overwrites storage on Linux, macOS, and Windows. It refuses the system disk, the disk it is running from, mounted volumes, and RAID or LVM members unless you pass an explicit flag. A typed confirmation of the device name is required. See SAFETY.md."
)]
pub struct Cli {
    #[arg(long, global = true, value_enum)]
    pub lang: Option<LangOpt>,
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List attached disks and volumes.
    List {
        /// Include loop, ram, and other virtual devices.
        #[arg(long)]
        all: bool,
    },
    /// Overwrite a disk, partition, or disk image file.
    Wipe {
        /// Path or device name (for example /dev/sdb, disk3, PhysicalDrive1, or an image file).
        target: PathBuf,
        /// zeros (default), random, ones, or multi.
        #[arg(long, default_value = "zeros")]
        method: String,
        /// Verify after write (default for single-pass methods).
        #[arg(long)]
        verify: bool,
        /// Disable the default verify pass.
        #[arg(long)]
        no_verify: bool,
        #[arg(long, value_name = "SIZE")]
        chunk_size: Option<String>,
        #[arg(long)]
        dry_run: bool,
        /// Do not prompt on a TTY. Still requires --confirm for block devices, and a named target.
        #[arg(long)]
        yes: bool,
        /// Type the device name (for example sdb, disk3, PhysicalDrive1).
        #[arg(long)]
        confirm: Option<String>,
        #[arg(long)]
        allow_system_disk: bool,
        #[arg(long)]
        allow_app_disk: bool,
        #[arg(long)]
        allow_mounted: bool,
        /// Also covers LVM and device-mapper members.
        #[arg(long)]
        allow_raid: bool,
        #[arg(long)]
        allow_in_use: bool,
        #[arg(long)]
        allow_unidentified: bool,
        #[arg(long)]
        skip_bad_sectors: bool,
        #[arg(long)]
        resume: bool,
        /// Seconds to wait before the first write. 0 is allowed only for image files.
        #[arg(long, default_value_t = 5)]
        countdown: u32,
        /// Write a JSON and text report to this path prefix.
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Overwrite regular files or a directory tree. Symlinks are not followed.
    Files {
        paths: Vec<PathBuf>,
        #[arg(long, default_value = "zeros")]
        method: String,
        #[arg(long)]
        verify: bool,
        #[arg(long)]
        no_verify: bool,
        #[arg(long, value_name = "SIZE")]
        chunk_size: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        confirm: Option<String>,
        #[arg(long, default_value_t = 5)]
        countdown: u32,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Overwrite free space on a mounted volume, then delete the filler files.
    #[command(name = "free-space")]
    FreeSpace {
        mount: PathBuf,
        #[arg(long, default_value = "zeros")]
        method: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        confirm: Option<String>,
        #[arg(long)]
        allow_system_disk: bool,
        #[arg(long, default_value_t = 5)]
        countdown: u32,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Report firmware sanitize support, or issue a Linux NVMe Sanitize / Format or ATA Secure Erase after the same safety checks and typed confirmation as a wipe.
    Sanitize {
        target: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        confirm: Option<String>,
        #[arg(long)]
        allow_system_disk: bool,
        #[arg(long)]
        allow_app_disk: bool,
        #[arg(long)]
        allow_raid: bool,
    },
}

/// Parse argv without exiting the process. Used by the binary, tests, and fuzzing.
pub fn parse_from<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    Cli::try_parse_from(args)
}

pub fn parse_os_args(args: &[String]) -> Result<Cli, clap::Error> {
    parse_from(args)
}

use frame_wipe::confirm::prompt_device_name;
use frame_wipe::device::{self, EnumerateOptions};
use frame_wipe::engine::{wipe, WipeRequest};
use frame_wipe::files::wipe_tree;
use frame_wipe::i18n::Lang;
use frame_wipe::method::Method;
use frame_wipe::planner::Scope;
use frame_wipe::progress::Progress;
use frame_wipe::report::{self, Report};
use frame_wipe::safety::SafetyFlags;
use frame_wipe::units::{format_bytes, format_eta, format_throughput};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub fn run() -> Result<(), String> {
    let mut argv: Vec<String> = std::env::args().collect();
    if argv.is_empty() {
        argv.push("framewipe".into());
    }
    let cli = match parse_os_args(&argv) {
        Ok(c) => c,
        Err(e) => {
            // clap already formats help/version/errors.
            let _ = e.print();
            if e.use_stderr() {
                std::process::exit(2);
            }
            std::process::exit(0);
        }
    };
    run_cli(cli)
}

pub fn run_cli(cli: Cli) -> Result<(), String> {
    let lang = cli
        .lang
        .map(|l| match l {
            LangOpt::En => Lang::En,
            LangOpt::Fr => Lang::Fr,
        })
        .unwrap_or_else(Lang::from_env);
    let _ = lang;
    let json = cli.json;
    match cli.command {
        Command::List { all } => {
            let list = device::enumerate(EnumerateOptions {
                include_virtual: all,
            })
            .map_err(|e| e.to_string())?;
            if json {
                let rows: Vec<serde_json::Value> = list
                    .iter()
                    .map(|d| {
                        let s = &d.snapshot;
                        serde_json::json!({
                            "name": s.identity.display_name,
                            "path": s.identity.path,
                            "model": s.identity.model,
                            "serial": s.identity.serial,
                            "size": s.identity.size,
                            "kind": format!("{:?}", s.kind),
                            "system": s.is_system,
                            "app_disk": s.is_app_disk,
                            "mounts": s.mounts,
                            "in_use": s.in_use,
                            "raid": s.is_raid_member,
                            "lvm": s.is_lvm_member,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?
                );
            } else {
                if list.is_empty() {
                    println!(
                        "No devices found. You may need to {}.",
                        device::elevation_hint()
                    );
                }
                for d in &list {
                    let s = &d.snapshot;
                    let mut tags = Vec::new();
                    if s.is_system {
                        tags.push("SYSTEM");
                    }
                    if s.is_app_disk {
                        tags.push("APP-DISK");
                    }
                    if !s.mounts.is_empty() {
                        tags.push("MOUNTED");
                    }
                    if s.in_use {
                        tags.push("IN-USE");
                    }
                    if s.is_raid_member {
                        tags.push("RAID");
                    }
                    if s.is_lvm_member {
                        tags.push("LVM");
                    }
                    println!(
                        "{:<16}  {:>10}  {}  serial={}  {}  {}",
                        s.identity.display_name,
                        format_bytes(s.identity.size),
                        s.identity.model.as_deref().unwrap_or("-"),
                        s.identity.serial.as_deref().unwrap_or("-"),
                        format!("{:?}", s.kind),
                        tags.join(",")
                    );
                    println!("    {}", s.identity.path.display());
                    if !s.mounts.is_empty() {
                        println!("    mounts: {}", s.mounts.join(", "));
                    }
                }
            }
            Ok(())
        }
        Command::Wipe {
            target,
            method,
            verify,
            no_verify,
            chunk_size,
            dry_run,
            yes,
            confirm,
            allow_system_disk,
            allow_app_disk,
            allow_mounted,
            allow_raid,
            allow_in_use,
            allow_unidentified,
            skip_bad_sectors,
            resume,
            countdown,
            report,
        } => {
            let method = Method::parse(&method, verify, no_verify).map_err(|e| e)?;
            let chunk = match chunk_size {
                Some(s) => Some(frame_wipe::units::parse_size(&s).map_err(|e| e)?),
                None => None,
            };
            let mut flags = SafetyFlags {
                allow_system_disk,
                allow_app_disk,
                allow_mounted,
                allow_raid,
                allow_in_use,
                allow_unidentified,
                assume_yes: yes,
                typed_confirm: confirm,
                explicit_target: true,
                countdown_secs: countdown,
                dry_run,
                skip_bad_sectors,
                resume,
            };
            confirm_if_needed(&target, &mut flags, yes)?;
            let cancel = install_cancel();
            let req = WipeRequest {
                path: target,
                scope: Scope::WholeDevice,
                method,
                chunk_size: chunk,
                flags,
            };
            let rec =
                wipe(&req, &cancel, |p| print_progress(p, json)).map_err(|e| e.to_string())?;
            finish_report(&rec, report.as_deref(), json)
        }
        Command::Files {
            paths,
            method,
            verify,
            no_verify,
            chunk_size,
            dry_run,
            yes,
            confirm,
            countdown,
            report,
        } => {
            if paths.is_empty() {
                return Err("files: pass at least one path".into());
            }
            let method = Method::parse(&method, verify, no_verify).map_err(|e| e)?;
            let chunk = match chunk_size {
                Some(s) => Some(frame_wipe::units::parse_size(&s).map_err(|e| e)?),
                None => None,
            };
            let cancel = install_cancel();
            let mut last: Option<Report> = None;
            for p in paths {
                let flags = SafetyFlags {
                    assume_yes: yes,
                    typed_confirm: confirm
                        .clone()
                        .or_else(|| p.file_name().map(|n| n.to_string_lossy().into_owned())),
                    explicit_target: true,
                    countdown_secs: countdown,
                    dry_run,
                    ..SafetyFlags::default()
                };
                let reports = wipe_tree(&p, method.clone(), chunk, flags, &cancel, |f| {
                    eprintln!("wiping {}", f.display());
                })
                .map_err(|e| e.to_string())?;
                last = reports.into_iter().last();
            }
            if let Some(rec) = last {
                finish_report(&rec, report.as_deref(), json)?;
            }
            Ok(())
        }
        Command::FreeSpace {
            mount,
            method,
            dry_run,
            yes,
            confirm,
            allow_system_disk,
            countdown,
            report,
        } => {
            let method = Method::parse(&method, false, false).map_err(|e| e)?;
            let mut flags = SafetyFlags {
                allow_system_disk,
                assume_yes: yes,
                typed_confirm: confirm,
                explicit_target: true,
                countdown_secs: countdown,
                dry_run,
                ..SafetyFlags::default()
            };
            confirm_if_needed(&mount, &mut flags, yes)?;
            let cancel = install_cancel();
            let rec =
                frame_wipe::free_space::wipe_free_space(&mount, method, flags, &cancel, |p| {
                    print_progress(p, json)
                })
                .map_err(|e| e.to_string())?;
            finish_report(&rec, report.as_deref(), json)
        }
        Command::Sanitize {
            target,
            dry_run,
            yes,
            confirm,
            allow_system_disk,
            allow_app_disk,
            allow_raid,
        } => {
            let caps = frame_wipe::sanitize::capabilities(&target).map_err(|e| e.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "supported": caps.supported,
                        "method": caps.method,
                        "detail": caps.detail,
                    })
                );
            } else {
                println!("{}", caps.detail);
            }
            let mut flags = SafetyFlags {
                allow_system_disk,
                allow_app_disk,
                allow_raid,
                assume_yes: yes,
                typed_confirm: confirm,
                explicit_target: true,
                dry_run,
                countdown_secs: 5,
                ..SafetyFlags::default()
            };
            if !dry_run {
                confirm_if_needed(&target, &mut flags, yes)?;
                let cancel = install_cancel();
                match frame_wipe::sanitize::sanitize(&target, flags, &cancel) {
                    Ok(rec) => finish_report(&rec, None, json),
                    Err(e) => Err(e.to_string()),
                }
            } else {
                Ok(())
            }
        }
    }
}

fn confirm_if_needed(
    target: &std::path::Path,
    flags: &mut SafetyFlags,
    yes: bool,
) -> Result<(), String> {
    let snap = device::inspect(target).map_err(|e| e.to_string())?;
    eprintln!("Target: {}", snap.identity.summary());
    for (label, yesv) in [
        ("SYSTEM DISK", snap.is_system),
        ("APP DISK", snap.is_app_disk),
        ("MOUNTED", !snap.mounts.is_empty()),
        ("IN USE", snap.in_use),
        ("RAID", snap.is_raid_member),
        ("LVM", snap.is_lvm_member),
    ] {
        if yesv {
            eprintln!("  warning: {label}");
        }
    }
    if flags.typed_confirm.is_none() {
        if yes && snap.is_regular_file {
            flags.typed_confirm = Some(snap.identity.display_name.clone());
            return Ok(());
        }
        if yes {
            return Err(format!(
                "block devices require --confirm {} (and a named target). --yes alone is not enough.",
                snap.identity.display_name
            ));
        }
        let stdin = io::stdin();
        let mut locked = stdin.lock();
        let mut out = io::stderr();
        prompt_device_name(&snap.identity.display_name, &mut out, &mut locked)
            .map_err(|e| e.to_string())?;
        flags.typed_confirm = Some(snap.identity.display_name.clone());
    }
    Ok(())
}

fn print_progress(p: Progress, json: bool) {
    if json {
        if let Ok(s) = serde_json::to_string(&p) {
            eprintln!("{s}");
        }
        return;
    }
    if let Some(c) = p.countdown_remaining {
        eprint!("\rstarting in {c}s    ");
        let _ = io::stderr().flush();
        return;
    }
    let pct = p.fraction() * 100.0;
    let eta = p.eta_secs.map(format_eta).unwrap_or_else(|| "--".into());
    eprint!(
        "\r{:?} pass {}/{}  {:>6.2}%  {}  {}  eta {eta}    ",
        p.phase,
        p.pass,
        p.passes,
        pct,
        format_bytes(p.bytes_done),
        format_throughput(p.throughput_bps),
    );
    let _ = io::stderr().flush();
}

fn finish_report(
    report: &Report,
    path: Option<&std::path::Path>,
    json: bool,
) -> Result<(), String> {
    eprintln!();
    if json {
        println!("{}", report.to_json().map_err(|e| e.to_string())?);
    } else {
        println!("{}", report.to_text());
    }
    if let Some(p) = path {
        report::write_report_files(report, p).map_err(|e| e.to_string())?;
        eprintln!("wrote report to {}", p.display());
    }
    match report.outcome {
        frame_wipe::Outcome::Success | frame_wipe::Outcome::DryRun => Ok(()),
        frame_wipe::Outcome::Cancelled => Err("cancelled".into()),
        frame_wipe::Outcome::Failed => Err("failed".into()),
    }
}

fn install_cancel() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let _ = CANCEL_SLOT.set(flag.clone());
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as usize);
    }
    flag
}

use std::sync::OnceLock;
static CANCEL_SLOT: OnceLock<Arc<AtomicBool>> = OnceLock::new();

#[cfg(unix)]
extern "C" fn on_sigint(_: i32) {
    if let Some(f) = CANCEL_SLOT.get() {
        f.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_does_not_panic() {
        let err = parse_from(["framewipe", "--help"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    }

    #[test]
    fn wipe_requires_target() {
        let err = parse_from(["framewipe", "wipe"]).unwrap_err();
        assert!(err.kind() != clap::error::ErrorKind::DisplayHelp);
    }

    #[test]
    fn wipe_accepts_named_target_and_confirm() {
        let cli = parse_from([
            "framewipe",
            "wipe",
            "/tmp/disk.img",
            "--yes",
            "--confirm",
            "disk.img",
            "--method",
            "zeros",
            "--countdown",
            "0",
        ])
        .expect("parse");
        match cli.command {
            Command::Wipe {
                target,
                yes,
                confirm,
                countdown,
                ..
            } => {
                assert!(yes);
                assert_eq!(confirm.as_deref(), Some("disk.img"));
                assert_eq!(countdown, 0);
                assert_eq!(target, std::path::PathBuf::from("/tmp/disk.img"));
            }
            _ => panic!("wrong command"),
        }
    }
}
