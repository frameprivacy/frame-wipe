use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Planning,
    Countdown,
    Rechecking,
    Writing,
    Verifying,
    Syncing,
    Sanitizing,
    Cancelling,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub phase: Phase,
    pub pass: u32,
    pub passes: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub throughput_bps: u64,
    pub eta_secs: Option<u64>,
    pub countdown_remaining: Option<u32>,
    pub message: Option<String>,
}

impl Progress {
    pub fn fraction(&self) -> f64 {
        if self.bytes_total == 0 {
            0.0
        } else {
            self.bytes_done as f64 / self.bytes_total as f64
        }
    }
}

/// Tracks throughput over a short window. Not serialized.
pub struct Meter {
    start: Instant,
    last: Instant,
    last_bytes: u64,
}

impl Meter {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last: now,
            last_bytes: 0,
        }
    }

    pub fn snapshot(&mut self, bytes_done: u64, bytes_total: u64) -> (u64, Option<u64>) {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last);
        let bps = if dt >= Duration::from_millis(200) {
            let db = bytes_done.saturating_sub(self.last_bytes);
            self.last = now;
            self.last_bytes = bytes_done;
            if dt.as_secs_f64() > 0.0 {
                (db as f64 / dt.as_secs_f64()) as u64
            } else {
                0
            }
        } else {
            let total_dt = now.saturating_duration_since(self.start).as_secs_f64();
            if total_dt > 0.0 {
                (bytes_done as f64 / total_dt) as u64
            } else {
                0
            }
        };
        let eta = if bps > 0 && bytes_done < bytes_total {
            Some((bytes_total - bytes_done) / bps)
        } else {
            None
        };
        (bps, eta)
    }
}

impl Default for Meter {
    fn default() -> Self {
        Self::new()
    }
}
