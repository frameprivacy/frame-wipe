//! Error types. I/O paths return these instead of panicking.

use std::io;
use std::path::PathBuf;
use thiserror::Error;

/// A safety guard that blocked an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Guard {
    SystemDisk,
    AppDisk,
    Mounted,
    InUse,
    RaidMember,
    LvmMember,
    MissingIdentity,
    IdentityMismatch,
    Symlink,
    ConfirmationMissing,
    ConfirmationMismatch,
    YesWithoutNamedTarget,
    CountdownCancelled,
    RestrictedTestTarget,
    UnmountRequired,
    FollowedLink,
    FirmwareNotSupported,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),

    #[error("refused by the {guard:?} safety guard: {detail}")]
    Refused { guard: Guard, detail: String },

    #[error("device identity changed before write (expected {expected}, found {found})")]
    IdentityMismatch { expected: String, found: String },

    #[error("short write at offset {offset}: expected {expected} bytes, wrote {wrote}")]
    ShortWrite {
        offset: u64,
        expected: usize,
        wrote: usize,
    },

    #[error("bad sector or media error at offset {offset}, length {length}: {detail}")]
    BadSector {
        offset: u64,
        length: usize,
        detail: String,
    },

    #[error("operation cancelled")]
    Cancelled,

    #[error("countdown cancelled before any write")]
    CountdownCancelled,

    #[error("I/O error at offset {offset}: {detail}")]
    IoAt { offset: u64, detail: String },

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("path is a symlink and will not be followed: {0}")]
    Symlink(PathBuf),

    #[error("target is not a regular file or permitted virtual device: {0}")]
    InvalidTarget(PathBuf),

    #[error("failed to open {path}: {detail}")]
    Open { path: PathBuf, detail: String },

    #[error("failed to allocate an aligned {size}-byte buffer: {detail}")]
    Alloc { size: usize, detail: String },

    #[error("verify failed at offset {offset}: {detail}")]
    Verify { offset: u64, detail: String },

    #[error("journal error: {0}")]
    Journal(String),

    #[error("firmware sanitize is not supported on this target: {0}")]
    FirmwareNotSupported(String),

    #[error("elevation is required to open {0}")]
    NeedsElevation(PathBuf),

    #[error("parse error: {0}")]
    Parse(String),
}

impl Error {
    pub fn refused(guard: Guard, detail: impl Into<String>) -> Self {
        Self::Refused {
            guard,
            detail: detail.into(),
        }
    }

    pub fn from_io_at(offset: u64, err: &io::Error) -> Self {
        if is_media_error(err) {
            Self::BadSector {
                offset,
                length: 0,
                detail: err.to_string(),
            }
        } else {
            Self::IoAt {
                offset,
                detail: err.to_string(),
            }
        }
    }

    pub fn guard(&self) -> Option<Guard> {
        match self {
            Self::Refused { guard, .. } => Some(*guard),
            Self::IdentityMismatch { .. } => Some(Guard::IdentityMismatch),
            Self::Symlink(_) => Some(Guard::Symlink),
            Self::CountdownCancelled => Some(Guard::CountdownCancelled),
            Self::Cancelled => None,
            Self::FirmwareNotSupported(_) => Some(Guard::FirmwareNotSupported),
            _ => None,
        }
    }
}

/// Media / bus errors that we report as bad sectors rather than generic I/O.
pub fn is_media_error(err: &io::Error) -> bool {
    match err.raw_os_error() {
        // Windows: ERROR_CRC, ERROR_SECTOR_NOT_FOUND, ERROR_WRITE_FAULT,
        // ERROR_READ_FAULT, ERROR_IO_DEVICE, ERROR_IO_DEVICE (1117), ERROR_DISK_OPERATION_FAILED
        Some(errno) if cfg!(windows) => matches!(errno, 23 | 27 | 29 | 30 | 1117 | 1118),
        // Unix: EIO (5), ENXIO (6) only when we already have an offset context,
        // EREMOTEIO (121 on Linux).
        Some(errno) => matches!(errno, 5 | 121),
        None => false,
    }
}
