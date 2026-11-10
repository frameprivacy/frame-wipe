//! Typed confirmation helpers and a cancellable countdown.

use crate::error::Error;
use crate::identity::confirmation_matches;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Return Ok if `typed` matches the required device name.
pub fn check_typed_name(display_name: &str, typed: &str) -> Result<(), Error> {
    if confirmation_matches(display_name, typed) {
        Ok(())
    } else {
        Err(Error::refused(
            crate::error::Guard::ConfirmationMismatch,
            format!("confirmation '{typed}' does not match device name '{display_name}'"),
        ))
    }
}

/// Interactive prompt used by the CLI when stdin is a terminal.
/// Reads one line and compares it to the device name. Never panics on I/O.
pub fn prompt_device_name(
    display_name: &str,
    stdout: &mut dyn Write,
    stdin: &mut dyn io::BufRead,
) -> Result<(), Error> {
    writeln!(
        stdout,
        "Type the device name '{display_name}' to continue (or press Enter to abort):"
    )
    .map_err(|e| Error::Message(format!("failed to write confirmation prompt: {e}")))?;
    stdout
        .flush()
        .map_err(|e| Error::Message(format!("failed to flush confirmation prompt: {e}")))?;
    let mut line = String::new();
    stdin
        .read_line(&mut line)
        .map_err(|e| Error::Message(format!("failed to read confirmation: {e}")))?;
    let typed = line.trim();
    if typed.is_empty() {
        return Err(Error::refused(
            crate::error::Guard::ConfirmationMissing,
            "empty confirmation",
        ));
    }
    check_typed_name(display_name, typed)
}

/// Sleep up to `secs` seconds, aborting early if `cancel` is set.
/// `on_tick` is called with the remaining seconds at the start of each second.
/// A `secs` of 0 returns immediately.
pub fn countdown(
    secs: u32,
    cancel: &Arc<AtomicBool>,
    mut on_tick: impl FnMut(u32) -> Result<(), Error>,
) -> Result<(), Error> {
    if secs == 0 {
        return Ok(());
    }
    for remaining in (1..=secs).rev() {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::CountdownCancelled);
        }
        on_tick(remaining)?;
        // Sleep in 50 ms slices so Cancel reacts quickly.
        let mut slept = 0u32;
        while slept < 1000 {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::CountdownCancelled);
            }
            thread::sleep(Duration::from_millis(50));
            slept += 50;
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(Error::CountdownCancelled);
    }
    Ok(())
}
