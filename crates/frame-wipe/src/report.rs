//! Completion report (JSON and plain text). Not a certification.

use crate::planner::Plan;
use crate::units::{format_bytes, format_eta, format_throughput};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Cancelled,
    DryRun,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectorError {
    pub offset: u64,
    pub length: u64,
    pub detail: String,
    pub during: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub tool: String,
    pub version: String,
    pub started_utc: String,
    pub finished_utc: String,
    pub elapsed_ms: Option<u64>,
    pub target_path: String,
    pub display_name: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub size: u64,
    pub method: String,
    pub scope: String,
    pub bytes_written: u64,
    pub verified: bool,
    pub outcome: Outcome,
    pub host: String,
    pub bad_sectors: Vec<SectorError>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str("Frame Wipe completion report\n");
        s.push_str("============================\n");
        s.push_str(&format!("Tool:        {} {}\n", self.tool, self.version));
        s.push_str(&format!("Host:        {}\n", self.host));
        s.push_str(&format!("Started:     {}\n", self.started_utc));
        s.push_str(&format!("Finished:    {}\n", self.finished_utc));
        if let Some(ms) = self.elapsed_ms {
            s.push_str(&format!("Elapsed:     {}\n", format_elapsed_ms(ms)));
            if ms > 0 && self.bytes_written > 0 {
                let bps = self.bytes_written.saturating_mul(1000) / ms.max(1);
                s.push_str(&format!("Throughput:  {}\n", format_throughput(bps)));
            }
        }
        s.push_str(&format!(
            "Target:      {} ({})\n",
            self.display_name, self.target_path
        ));
        s.push_str(&format!(
            "Model:       {}\n",
            self.model.as_deref().unwrap_or("unknown")
        ));
        s.push_str(&format!(
            "Serial:      {}\n",
            self.serial.as_deref().unwrap_or("unknown")
        ));
        s.push_str(&format!(
            "Size:        {} ({} bytes)\n",
            format_bytes(self.size),
            self.size
        ));
        s.push_str(&format!("Scope:       {}\n", self.scope));
        s.push_str(&format!("Method:      {}\n", self.method));
        s.push_str(&format!(
            "Written:     {}\n",
            format_bytes(self.bytes_written)
        ));
        s.push_str(&format!(
            "Verified:    {}\n",
            if self.verified { "yes" } else { "no" }
        ));
        s.push_str(&format!("Outcome:     {:?}\n", self.outcome));
        if !self.bad_sectors.is_empty() {
            s.push_str(&format!("Bad sectors: {}\n", self.bad_sectors.len()));
            for b in &self.bad_sectors {
                s.push_str(&format!(
                    "  - offset {} length {} during {}: {}\n",
                    b.offset, b.length, b.during, b.detail
                ));
            }
        }
        if !self.warnings.is_empty() {
            s.push_str("Warnings:\n");
            for w in &self.warnings {
                s.push_str(&format!("  - {w}\n"));
            }
        }
        for n in &self.notes {
            s.push_str(&format!("Note: {n}\n"));
        }
        s.push_str("\nThis report is a record of what the tool did. It is not a certification\n");
        s.push_str("(for example NIST or DoD). See SAFETY.md and the README for limits,\n");
        s.push_str("especially on SSDs and NVMe devices.\n");
        s
    }
}

pub fn from_plan(
    plan: &Plan,
    outcome: Outcome,
    started: OffsetDateTime,
    finished: OffsetDateTime,
    bytes_written: u64,
    bad_sectors: Vec<SectorError>,
    elapsed_ms: Option<u64>,
) -> Report {
    let verified = plan.passes.iter().any(|p| p.verify) && outcome == Outcome::Success;
    let mut notes = Vec::new();
    if outcome == Outcome::DryRun {
        notes.push("Dry run: no bytes were written.".into());
    }
    if outcome == Outcome::Cancelled {
        notes.push("The run was cancelled. The target may be only partly overwritten. A journal file may still be present.".into());
    }
    Report {
        tool: "framewipe".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        started_utc: started
            .format(&Rfc3339)
            .unwrap_or_else(|_| started.to_string()),
        finished_utc: finished
            .format(&Rfc3339)
            .unwrap_or_else(|_| finished.to_string()),
        elapsed_ms,
        target_path: plan.identity.path.display().to_string(),
        display_name: plan.identity.display_name.clone(),
        model: plan.identity.model.clone(),
        serial: plan.identity.serial.clone(),
        size: plan.size,
        method: plan.method.label(),
        scope: plan.scope.label().to_string(),
        bytes_written,
        verified,
        outcome,
        host: hostname(),
        bad_sectors,
        warnings: plan.warnings.clone(),
        notes,
    }
}

pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        let ok = unsafe { GetComputerNameW(buf.as_mut_ptr(), &mut len) };
        if ok != 0 {
            return String::from_utf16_lossy(&buf[..len as usize]);
        }
    }
    "unknown".into()
}

pub fn write_report_files(
    report: &Report,
    path: &std::path::Path,
) -> Result<(), crate::error::Error> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let json_path = if path.extension().is_none() {
        path.with_extension("json")
    } else {
        path.to_path_buf()
    };
    let text_path = json_path.with_extension("txt");
    let mut jf = std::fs::File::create(&json_path)?;
    jf.write_all(
        report
            .to_json()
            .map_err(|e| crate::error::Error::Message(e.to_string()))?
            .as_bytes(),
    )?;
    jf.sync_all()?;
    let mut tf = std::fs::File::create(&text_path)?;
    tf.write_all(report.to_text().as_bytes())?;
    tf.sync_all()?;
    Ok(())
}

pub fn format_elapsed_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else {
        format_eta(ms / 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::Scope;

    #[test]
    fn scope_label_is_readable() {
        assert_eq!(Scope::WholeDevice.label(), "whole device");
        assert_eq!(Scope::FreeSpace.label(), "free space");
        assert_ne!(Scope::WholeDevice.label(), "wholedevice");
    }

    #[test]
    fn elapsed_uses_a_human_duration() {
        assert_eq!(format_elapsed_ms(0), "0 ms");
        assert_eq!(format_elapsed_ms(250), "250 ms");
        assert_eq!(format_elapsed_ms(2_292_000), "38m 12s");
        let r = Report {
            tool: "framewipe".into(),
            version: "0".into(),
            started_utc: "t".into(),
            finished_utc: "t".into(),
            elapsed_ms: Some(2_292_000),
            target_path: "/dev/sdb".into(),
            display_name: "sdb".into(),
            model: None,
            serial: None,
            size: 100,
            method: "zeros".into(),
            scope: Scope::WholeDevice.label().into(),
            bytes_written: 100,
            verified: true,
            outcome: Outcome::Success,
            host: "test".into(),
            bad_sectors: vec![],
            warnings: vec![],
            notes: vec![],
        };
        let text = r.to_text();
        assert!(text.contains("Elapsed:     38m 12s"), "{text}");
        assert!(text.contains("Scope:       whole device"), "{text}");
        assert!(!text.contains("wholedevice"), "{text}");
        assert!(!text.contains("2292.000 s"), "{text}");
    }
}
