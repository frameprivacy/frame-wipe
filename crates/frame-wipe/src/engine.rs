//! Double-buffered overwrite engine. No unwrap on I/O paths. Identity is re-checked
//! immediately before the first write.

use crate::confirm::countdown;
use crate::device::{self, recapture};
use crate::error::Error;
use crate::identity::assert_same;
use crate::journal::{self, Journal};
use crate::method::{Pass, Pattern};
use crate::open::{self, alloc_aligned, fill_pattern, RawIo};
use crate::planner::{plan, Plan, Scope, TargetSnapshot};
use crate::progress::{Meter, Phase, Progress};
use crate::report::{self, Outcome, Report, SectorError};
use crate::safety::{self, SafetyFlags};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Instant;
use time::OffsetDateTime;

/// Test hook: run once after planning and the countdown, before the identity re-check.
/// Used to simulate a hot-plug. Not for production callers.
pub static BETWEEN_PLAN_AND_RECHECK: Mutex<Option<fn()>> = Mutex::new(None);

pub struct WipeRequest {
    pub path: PathBuf,
    pub scope: Scope,
    pub method: crate::method::Method,
    pub chunk_size: Option<u64>,
    pub flags: SafetyFlags,
}

pub fn wipe(
    req: &WipeRequest,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(Progress),
) -> Result<Report, Error> {
    let started = OffsetDateTime::now_utc();
    let started_instant = Instant::now();
    // Refuse raw physical paths before inspect so restricted-mode tests never
    // open a real disk. Windows \\.\PhysicalDriveN is not a filesystem path,
    // and symlink_metadata can fail with "Incorrect function". Virtual loop
    // devices are not classified as raw, so their inspect still runs.
    safety::assert_not_raw_device_in_restricted_mode(&req.path)?;
    let snap = device::inspect(&req.path)?;
    safety::assert_permitted_in_restricted_mode(
        &req.path,
        snap.backing_file.as_deref(),
        snap.is_virtual(),
    )?;
    if safety::is_current_exe(&req.path) {
        return Err(Error::refused(
            crate::error::Guard::AppDisk,
            format!(
                "refusing to overwrite the running executable {}",
                req.path.display()
            ),
        ));
    }

    let planned = plan(
        &snap,
        req.scope,
        req.method.clone(),
        req.chunk_size,
        &req.flags,
    )?;
    emit(
        &mut on_progress,
        Phase::Planning,
        0,
        &planned,
        0,
        0,
        None,
        Some("plan ready"),
    );

    for w in &planned.warnings {
        emit(
            &mut on_progress,
            Phase::Planning,
            0,
            &planned,
            0,
            0,
            None,
            Some(w.as_str()),
        );
    }

    if planned.dry_run {
        return Ok(report::from_plan(
            &planned,
            Outcome::DryRun,
            started,
            OffsetDateTime::now_utc(),
            0,
            Vec::new(),
            None,
        ));
    }

    emit(
        &mut on_progress,
        Phase::Countdown,
        0,
        &planned,
        0,
        0,
        Some(planned.countdown_secs),
        None,
    );
    countdown(planned.countdown_secs, cancel, |remaining| {
        emit(
            &mut on_progress,
            Phase::Countdown,
            0,
            &planned,
            0,
            0,
            Some(remaining),
            None,
        );
        Ok(())
    })?;

    if cancel.load(Ordering::SeqCst) {
        return Err(Error::Cancelled);
    }

    if let Ok(mut hook) = BETWEEN_PLAN_AND_RECHECK.lock() {
        if let Some(f) = hook.take() {
            f();
        }
    }

    // Re-check identity (serial + size + path) right before the first write.
    emit(
        &mut on_progress,
        Phase::Rechecking,
        0,
        &planned,
        0,
        0,
        None,
        Some("re-checking device identity"),
    );
    let fresh = recapture(&req.path)?;
    assert_same(&planned.identity, &fresh)?;
    // Also re-run the planner against a fresh snapshot in case a mount or holder appeared.
    let snap2 = device::inspect(&req.path)?;
    assert_same(&planned.identity, &snap2.identity)?;
    if snap2.is_system != snap.is_system
        || snap2.is_app_disk != snap.is_app_disk
        || snap2.is_raid_member != snap.is_raid_member
        || snap2.is_lvm_member != snap.is_lvm_member
        || snap2.mounts != snap.mounts
        || snap2.in_use != snap.in_use
    {
        return Err(Error::IdentityMismatch {
            expected: format!(
                "state at plan time: system={} app={} raid={} lvm={} mounts={:?} in_use={}",
                snap.is_system,
                snap.is_app_disk,
                snap.is_raid_member,
                snap.is_lvm_member,
                snap.mounts,
                snap.in_use
            ),
            found: format!(
                "state before write: system={} app={} raid={} lvm={} mounts={:?} in_use={}",
                snap2.is_system,
                snap2.is_app_disk,
                snap2.is_raid_member,
                snap2.is_lvm_member,
                snap2.mounts,
                snap2.in_use
            ),
        });
    }

    let io_path = io_path_for(&req.path, &snap2);
    safety::assert_permitted_in_restricted_mode(
        &io_path,
        snap2.backing_file.as_deref(),
        snap2.is_virtual(),
    )?;

    let mut io = open::open_rw(&io_path, planned.chunk_size as usize)?;
    // Recheck size after open, in case of a hot-plug between recapture and open.
    let opened_size = snap2.identity.size;
    if opened_size != planned.size {
        return Err(Error::IdentityMismatch {
            expected: format!("size {}", planned.size),
            found: format!("size {opened_size} after open"),
        });
    }

    let journal_path = journal::journal_path_for(&req.path);
    let existing_journal = journal::load_for_resume(&journal_path, &planned.identity)?;
    if let Some(existing) = existing_journal.as_ref() {
        if !req.flags.resume {
            return Err(Error::Journal(format!(
                "an incomplete wipe journal exists at {} (offset {} of {}, started {}). Pass --resume if you want to continue, or delete the journal if that run is finished.",
                journal_path.display(),
                existing.offset,
                existing.size,
                existing.started_utc
            )));
        }
        let expected_method = planned.method.label();
        if existing.method != expected_method {
            return Err(Error::Journal(format!(
                "existing journal method '{}' does not match '{}' for this run; refusing to resume. Delete {} only if you know the previous run is finished.",
                existing.method,
                expected_method,
                journal_path.display()
            )));
        }
        if existing.size != planned.size {
            return Err(Error::Journal(format!(
                "existing journal size {} does not match current size {}",
                existing.size, planned.size
            )));
        }
        if existing.pass == 0 || existing.pass > planned.passes.len() as u32 {
            return Err(Error::Journal(format!(
                "existing journal pass {} is out of range for a {}-pass method",
                existing.pass,
                planned.passes.len()
            )));
        }
    }

    let mut journal = if let Some(existing) = existing_journal.clone() {
        existing
    } else {
        Journal::new(
            &planned.identity,
            &planned.method.label(),
            planned.passes.len() as u32,
            planned.size,
            &started
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| started.to_string()),
        )
    };
    journal::checkpoint(&journal_path, &journal)?;

    let mut bad: Vec<SectorError> = Vec::new();
    let mut bytes_written: u64 = 0;
    let mut outcome = Outcome::Success;
    let passes = planned.passes.clone();
    let total_passes = passes.len() as u32;
    let resume_state = existing_journal;

    'passes: for (idx, pass) in passes.iter().enumerate() {
        let pass_no = (idx as u32) + 1;
        if let Some(ref prev) = resume_state {
            if pass_no < prev.pass {
                continue;
            }
        }

        let resume_this_pass = resume_state
            .as_ref()
            .is_some_and(|prev| prev.pass == pass_no);
        let write_already_done = resume_this_pass
            && resume_state.as_ref().is_some_and(|prev| {
                prev.phase == "verifying"
                    || prev.phase == "verify_failed"
                    || prev.offset >= planned.size
            });
        let start_offset = if resume_this_pass && !write_already_done {
            crate::units::align_down(
                resume_state.as_ref().map(|prev| prev.offset).unwrap_or(0),
                planned.chunk_size.max(1),
            )
        } else {
            0
        };

        journal.pass = pass_no;
        if !write_already_done {
            journal.phase = "writing".into();
            journal.offset = start_offset;
        }
        let pass_seed = if resume_this_pass {
            resume_state
                .as_ref()
                .and_then(|prev| prev.random_seed)
                .or_else(|| seed_for_pass(pass.pattern, pass_no))
        } else {
            seed_for_pass(pass.pattern, pass_no)
        };
        journal.random_seed = pass_seed;
        journal::checkpoint(&journal_path, &journal)?;

        if !write_already_done {
            match write_pass(
                &mut io,
                &planned,
                *pass,
                pass_no,
                total_passes,
                pass_seed,
                start_offset,
                cancel,
                &mut on_progress,
                &journal_path,
                &mut journal,
                &mut bad,
            ) {
                Ok(n) => bytes_written = bytes_written.saturating_add(n),
                Err(Error::Cancelled) => {
                    outcome = Outcome::Cancelled;
                    let _ = io.sync();
                    journal.phase = "cancelled".into();
                    let _ = journal::checkpoint(&journal_path, &journal);
                    break 'passes;
                }
                Err(e) => {
                    journal.phase = "failed".into();
                    let _ = journal::checkpoint(&journal_path, &journal);
                    let _ = io.sync();
                    return Err(e);
                }
            }
        }

        if pass.verify {
            journal.phase = "verifying".into();
            journal.offset = 0;
            journal::checkpoint(&journal_path, &journal)?;
            if let Err(e) = verify_pass(
                &mut io,
                &planned,
                *pass,
                pass_no,
                total_passes,
                pass_seed,
                cancel,
                &mut on_progress,
                &mut bad,
            ) {
                if matches!(e, Error::Cancelled) {
                    outcome = Outcome::Cancelled;
                    journal.phase = "cancelled".into();
                    let _ = journal::checkpoint(&journal_path, &journal);
                    let _ = io.sync();
                    break 'passes;
                }
                journal.phase = "verify_failed".into();
                let _ = journal::checkpoint(&journal_path, &journal);
                let _ = io.sync();
                return Err(e);
            }
        }
    }

    emit(
        &mut on_progress,
        Phase::Syncing,
        total_passes,
        &planned,
        planned.size,
        0,
        None,
        Some("syncing"),
    );
    io.sync()?;

    if outcome == Outcome::Success {
        journal::complete_and_remove(&journal_path, &mut journal)?;
    } else {
        journal.phase = "cancelled".into();
        let _ = journal::checkpoint(&journal_path, &journal);
    }

    let finished = OffsetDateTime::now_utc();
    let elapsed_ms = started_instant.elapsed().as_millis() as u64;
    let report = report::from_plan(
        &planned,
        outcome,
        started,
        finished,
        bytes_written,
        bad,
        Some(elapsed_ms),
    );
    emit(
        &mut on_progress,
        if outcome == Outcome::Success {
            Phase::Done
        } else {
            Phase::Cancelling
        },
        total_passes,
        &planned,
        planned.size,
        0,
        None,
        None,
    );
    Ok(report)
}

fn io_path_for(user: &Path, snap: &TargetSnapshot) -> PathBuf {
    if snap.is_regular_file || snap.is_directory {
        return user.to_path_buf();
    }
    device::raw_path_for(&snap.identity.path)
}

fn chunk_len(direct: bool, align: usize, chunk: usize, offset: u64, size: u64) -> usize {
    let remaining = size - offset;
    let want = remaining.min(chunk as u64) as usize;
    if direct {
        let a = (want / align) * align;
        if a == 0 {
            want
        } else {
            a
        }
    } else {
        want
    }
}

fn write_pass(
    io: &mut RawIo,
    plan: &Plan,
    pass: Pass,
    pass_no: u32,
    total_passes: u32,
    seed: Option<[u8; 32]>,
    start_offset: u64,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut impl FnMut(Progress),
    journal_path: &Path,
    journal: &mut Journal,
    bad: &mut Vec<SectorError>,
) -> Result<u64, Error> {
    let size = plan.size;
    if size == 0 || start_offset >= size {
        return Ok(0);
    }
    let chunk = plan.chunk_size as usize;
    let align = io.alignment.max(512);
    let direct = io.direct;
    let pattern = pass.pattern;

    // Fill thread owns the CSPRNG and produces buffers in offset order so verify
    // can replay the same stream from the same seed.
    let (job_tx, job_rx) = sync_channel::<Option<(u64, usize)>>(2);
    let (buf_tx, buf_rx) = sync_channel::<Result<(u64, open::AlignedBuf, usize), Error>>(2);

    let fill = thread::Builder::new()
        .name("framewipe-fill".into())
        .spawn(move || {
            let mut rng = seed.map(ChaCha20Rng::from_seed);
            while let Ok(Some((off, len))) = job_rx.recv() {
                let mut buf = match alloc_aligned(len.max(align), align) {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = buf_tx.send(Err(e));
                        return;
                    }
                };
                fill_pattern(&mut buf.as_mut()[..len], pattern, rng.as_mut());
                if buf_tx.send(Ok((off, buf, len))).is_err() {
                    return;
                }
            }
        })
        .map_err(|e| Error::Message(format!("failed to start fill thread: {e}")))?;

    let shutdown = |job_tx: &std::sync::mpsc::SyncSender<Option<(u64, usize)>>,
                    fill: std::thread::JoinHandle<()>| {
        let _ = job_tx.send(None);
        let _ = fill.join();
    };

    let mut next_to_queue: u64 = start_offset;
    let mut in_flight: u32 = 0;
    let mut written: u64 = 0;
    let mut meter = Meter::new();
    let mut since_journal: u64 = 0;

    let queue_one = |next_to_queue: &mut u64, in_flight: &mut u32| -> Result<(), Error> {
        if *next_to_queue >= size {
            return Ok(());
        }
        let len = chunk_len(direct, align, chunk, *next_to_queue, size);
        if len == 0 {
            *next_to_queue = size;
            return Ok(());
        }
        job_tx
            .send(Some((*next_to_queue, len)))
            .map_err(|_| Error::Message("fill thread closed".into()))?;
        *next_to_queue += len as u64;
        *in_flight += 1;
        Ok(())
    };

    queue_one(&mut next_to_queue, &mut in_flight)?;
    if next_to_queue < size {
        queue_one(&mut next_to_queue, &mut in_flight)?;
    }

    while in_flight > 0 {
        if cancel.load(Ordering::SeqCst) {
            shutdown(&job_tx, fill);
            return Err(Error::Cancelled);
        }
        let (off, buf, len) = match buf_rx.recv() {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                shutdown(&job_tx, fill);
                return Err(e);
            }
            Err(_) => {
                let _ = fill.join();
                return Err(Error::Message("fill thread ended unexpectedly".into()));
            }
        };
        in_flight -= 1;
        if next_to_queue < size {
            if let Err(e) = queue_one(&mut next_to_queue, &mut in_flight) {
                shutdown(&job_tx, fill);
                return Err(e);
            }
        }
        match write_exact_or_report(io, off, &buf.as_ref()[..len], plan.skip_bad_sectors, bad) {
            Ok(n) => {
                written += n as u64;
                since_journal += n as u64;
                let pos = off + n as u64;
                let (bps, eta) = meter.snapshot(pos, size);
                emit_pass(
                    on_progress,
                    Phase::Writing,
                    pass_no,
                    total_passes,
                    plan,
                    pos,
                    bps,
                    eta,
                    None,
                );
                if since_journal >= plan.chunk_size || pos >= size {
                    journal.offset = pos;
                    journal.phase = "writing".into();
                    if let Err(e) = journal::checkpoint(journal_path, journal) {
                        shutdown(&job_tx, fill);
                        return Err(e);
                    }
                    since_journal = 0;
                }
            }
            Err(e) => {
                shutdown(&job_tx, fill);
                return Err(e);
            }
        }
    }
    let _ = job_tx.send(None);
    match fill.join() {
        Ok(()) => {}
        Err(_) => return Err(Error::Message("fill thread panicked".into())),
    }
    Ok(written)
}

fn write_exact_or_report(
    io: &mut RawIo,
    offset: u64,
    buf: &[u8],
    skip_bad: bool,
    bad: &mut Vec<SectorError>,
) -> Result<usize, Error> {
    match io.write_at(offset, buf) {
        Ok(n) if n == buf.len() => Ok(n),
        Ok(0) => Err(Error::ShortWrite {
            offset,
            expected: buf.len(),
            wrote: 0,
        }),
        Ok(n) => {
            // One retry of the remainder.
            match io.write_at(offset + n as u64, &buf[n..]) {
                Ok(m) if n + m == buf.len() => Ok(n + m),
                Ok(m) => Err(Error::ShortWrite {
                    offset,
                    expected: buf.len(),
                    wrote: n + m,
                }),
                Err(e) => Err(e),
            }
        }
        Err(Error::BadSector {
            offset,
            length,
            detail,
        }) if skip_bad => {
            let len = if length == 0 { buf.len() } else { length };
            bad.push(SectorError {
                offset,
                length: len as u64,
                detail: detail.clone(),
                during: "write".into(),
            });
            Ok(buf.len())
        }
        Err(e) => Err(e),
    }
}

fn verify_pass(
    io: &mut RawIo,
    plan: &Plan,
    pass: Pass,
    pass_no: u32,
    total_passes: u32,
    seed: Option<[u8; 32]>,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut impl FnMut(Progress),
    bad: &mut Vec<SectorError>,
) -> Result<(), Error> {
    let size = plan.size;
    let chunk = plan.chunk_size as usize;
    let align = io.alignment.max(512);
    let mut rng = seed.map(ChaCha20Rng::from_seed);
    let mut expected = alloc_aligned(chunk.max(align), align)?;
    let mut actual = alloc_aligned(chunk.max(align), align)?;
    let mut offset = 0u64;
    let mut meter = Meter::new();
    while offset < size {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        let remaining = (size - offset) as usize;
        let len = remaining.min(chunk);
        fill_pattern(&mut expected.as_mut()[..len], pass.pattern, rng.as_mut());
        match io.read_at(offset, &mut actual.as_mut()[..len]) {
            Ok(n) if n == len => {}
            Ok(n) => {
                return Err(Error::Verify {
                    offset,
                    detail: format!("short read: expected {len} bytes, got {n}"),
                });
            }
            Err(Error::BadSector {
                offset: off,
                length,
                detail,
            }) if plan.skip_bad_sectors => {
                bad.push(SectorError {
                    offset: off,
                    length: length as u64,
                    detail,
                    during: "verify".into(),
                });
                offset += len as u64;
                continue;
            }
            Err(e) => return Err(e),
        }
        if expected.as_ref()[..len] != actual.as_ref()[..len] {
            let pos = expected
                .as_ref()
                .iter()
                .zip(actual.as_ref().iter())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            return Err(Error::Verify {
                offset: offset + pos as u64,
                detail: format!(
                    "byte mismatch at +{pos} (expected 0x{:02x}, read 0x{:02x})",
                    expected.as_ref()[pos],
                    actual.as_ref()[pos]
                ),
            });
        }
        offset += len as u64;
        let (bps, eta) = meter.snapshot(offset, size);
        emit_pass(
            on_progress,
            Phase::Verifying,
            pass_no,
            total_passes,
            plan,
            offset,
            bps,
            eta,
            None,
        );
    }
    Ok(())
}

fn seed_for_pass(pattern: Pattern, pass_no: u32) -> Option<[u8; 32]> {
    if pattern != Pattern::Random {
        return None;
    }
    let mut seed = [0u8; 32];
    if let Err(e) = getrandom::getrandom(&mut seed) {
        // Fall back to a time-based seed only if the OS CSPRNG failed; still not unwrap.
        let t = OffsetDateTime::now_utc()
            .unix_timestamp_nanos()
            .to_le_bytes();
        for (i, b) in t.iter().cycle().take(32).enumerate() {
            seed[i] = *b ^ (pass_no as u8).wrapping_mul(17);
        }
        let _ = e;
    }
    Some(seed)
}

fn emit(
    on_progress: &mut impl FnMut(Progress),
    phase: Phase,
    pass: u32,
    plan: &Plan,
    bytes_done: u64,
    bps: u64,
    countdown: Option<u32>,
    message: Option<&str>,
) {
    on_progress(Progress {
        phase,
        pass,
        passes: plan.passes.len() as u32,
        bytes_done,
        bytes_total: plan.size,
        throughput_bps: bps,
        eta_secs: None,
        countdown_remaining: countdown,
        message: message.map(|s| s.to_string()),
    });
}

fn emit_pass(
    on_progress: &mut impl FnMut(Progress),
    phase: Phase,
    pass: u32,
    passes: u32,
    plan: &Plan,
    bytes_done: u64,
    bps: u64,
    eta: Option<u64>,
    message: Option<&str>,
) {
    on_progress(Progress {
        phase,
        pass,
        passes,
        bytes_done,
        bytes_total: plan.size,
        throughput_bps: bps,
        eta_secs: eta,
        countdown_remaining: None,
        message: message.map(|s| s.to_string()),
    });
}
