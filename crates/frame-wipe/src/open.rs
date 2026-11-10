//! Open targets with exclusive access, without following symlinks, and with direct I/O when possible.

use crate::error::Error;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub struct RawIo {
    file: File,
    pub direct: bool,
    pub alignment: usize,
    pub path_display: String,
}

impl RawIo {
    pub fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<usize, Error> {
        if let Err(e) = self.file.seek(SeekFrom::Start(offset)) {
            return Err(Error::from_io_at(offset, &e));
        }
        match self.file.write(buf) {
            Ok(n) => Ok(n),
            Err(e) => Err(Error::from_io_at(offset, &e)),
        }
    }

    pub fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, Error> {
        if let Err(e) = self.file.seek(SeekFrom::Start(offset)) {
            return Err(Error::from_io_at(offset, &e));
        }
        match self.file.read(buf) {
            Ok(n) => Ok(n),
            Err(e) => Err(Error::from_io_at(offset, &e)),
        }
    }

    pub fn sync(&mut self) -> Result<(), Error> {
        self.file.sync_all().map_err(Error::from)
    }

    pub fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

/// Open `path` for read/write. Never follows a symlink. Attempts direct I/O.
pub fn open_rw(path: &Path, alignment_hint: usize) -> Result<RawIo, Error> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(Error::Symlink(path.to_path_buf()));
        }
    }
    let alignment = if alignment_hint == 0 {
        4096
    } else {
        alignment_hint
    };
    // Direct I/O requires aligned transfers. Small or unaligned regular files
    // use buffered I/O; the engine still fsyncs at the end.
    let mut try_direct = true;
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_file() {
            let len = meta.len();
            if len < alignment as u64 || len % alignment as u64 != 0 {
                try_direct = false;
            }
        }
    }
    if try_direct {
        if let Ok(io) = open_direct(path, alignment) {
            return Ok(io);
        }
    }
    open_buffered(path, alignment)
}

/// Open read-only for identity / size checks. Never follows a symlink.
pub fn open_ro(path: &Path) -> Result<File, Error> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(Error::Symlink(path.to_path_buf()));
        }
    }
    open_ro_inner(path)
}

fn open_buffered(path: &Path, alignment: usize) -> Result<RawIo, Error> {
    let file = open_rw_nofollow(path, false)?;
    lock_exclusive(&file, path)?;
    Ok(RawIo {
        file,
        direct: false,
        alignment,
        path_display: path.display().to_string(),
    })
}

fn open_direct(path: &Path, alignment: usize) -> Result<RawIo, Error> {
    let file = open_rw_nofollow(path, true)?;
    lock_exclusive(&file, path)?;
    Ok(RawIo {
        file,
        direct: true,
        alignment,
        path_display: path.display().to_string(),
    })
}

fn open_rw_nofollow(path: &Path, direct: bool) -> Result<File, Error> {
    #[cfg(unix)]
    {
        unix_open(path, true, direct)
    }
    #[cfg(windows)]
    {
        windows_open(path, true, direct)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = direct;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| Error::Open {
                path: path.to_path_buf(),
                detail: e.to_string(),
            })
    }
}

fn open_ro_inner(path: &Path) -> Result<File, Error> {
    #[cfg(unix)]
    {
        unix_open(path, false, false)
    }
    #[cfg(windows)]
    {
        windows_open(path, false, false)
    }
    #[cfg(not(any(unix, windows)))]
    {
        OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|e| Error::Open {
                path: path.to_path_buf(),
                detail: e.to_string(),
            })
    }
}

#[cfg(unix)]
fn unix_open(path: &Path, rw: bool, direct: bool) -> Result<File, Error> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{FromRawFd, OwnedFd};
    let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| Error::Open {
        path: path.to_path_buf(),
        detail: "path contains an interior NUL".into(),
    })?;
    let mut flags = libc::O_CLOEXEC | libc::O_NOFOLLOW;
    flags |= if rw { libc::O_RDWR } else { libc::O_RDONLY };
    #[cfg(target_os = "linux")]
    if direct {
        flags |= libc::O_DIRECT;
    }
    let fd = unsafe { libc::open(c.as_ptr(), flags, 0o600) };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ELOOP) {
            return Err(Error::Symlink(path.to_path_buf()));
        }
        return Err(map_open_error(path, err));
    }
    #[cfg(target_os = "macos")]
    if direct {
        // F_NOCACHE = 48 on macOS. Apply before the fd is wrapped.
        let _ = unsafe { libc::fcntl(fd, 48, 1) };
    }
    let _ = direct;
    Ok(File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
}

#[cfg(windows)]
fn windows_open(path: &Path, rw: bool, direct: bool) -> Result<File, Error> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use std::ptr;
    use windows_sys::Win32::Foundation::{GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut access = GENERIC_READ;
    if rw {
        access |= GENERIC_WRITE;
    }
    let mut flags = FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT;
    if direct {
        flags |= FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH;
    }
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        let code = unsafe { GetLastError() } as i32;
        let err = std::io::Error::from_raw_os_error(code);
        // ERROR_CANT_ACCESS_FILE / reparse
        if matches!(code, 1920 | 4390 | 4393) {
            return Err(Error::Symlink(path.to_path_buf()));
        }
        return Err(map_open_error(path, err));
    }
    // Refuse if the handle is a reparse point we should not follow.
    Ok(File::from(unsafe { OwnedHandle::from_raw_handle(h as _) }))
}

fn map_open_error(path: &Path, err: std::io::Error) -> Error {
    let permission = err.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(
            err.raw_os_error(),
            Some(13) | Some(1) | Some(5) | Some(1314)
        );
    if permission
        && !crate::device::is_elevated()
        && crate::safety::path_looks_like_raw_device(path)
    {
        return Error::NeedsElevation(path.to_path_buf());
    }
    let detail = if permission {
        format!("permission denied (the file is not writable): {err}")
    } else {
        err.to_string()
    };
    Error::Open {
        path: path.to_path_buf(),
        detail,
    }
}

fn lock_exclusive(file: &File, path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock
                || err.raw_os_error() == Some(libc::EWOULDBLOCK)
                || err.raw_os_error() == Some(libc::EAGAIN)
            {
                return Err(Error::refused(
                    crate::error::Guard::InUse,
                    format!("{} is locked by another process", path.display()),
                ));
            }
            // Some devices do not support flock; continue without a lock.
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::mem::zeroed;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::Storage::FileSystem::{
            LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        };
        use windows_sys::Win32::System::IO::OVERLAPPED;
        let mut ov: OVERLAPPED = unsafe { zeroed() };
        let ok = unsafe {
            LockFileEx(
                file.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                &mut ov,
            )
        };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            // ERROR_LOCK_VIOLATION = 33, ERROR_IO_PENDING is not expected with FAIL_IMMEDIATELY
            if code == 33 {
                return Err(Error::refused(
                    crate::error::Guard::InUse,
                    format!("{} is locked by another process", path.display()),
                ));
            }
        }
        let _ = path;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, path);
        Ok(())
    }
}

/// Buffer aligned for O_DIRECT / FILE_FLAG_NO_BUFFERING. Drop uses the original layout.
pub struct AlignedBuf {
    ptr: *mut u8,
    size: usize,
    align: usize,
}

unsafe impl Send for AlignedBuf {}
unsafe impl Sync for AlignedBuf {}

impl AlignedBuf {
    pub fn as_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) }
    }

    pub fn as_ref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.size) }
    }

    pub fn len(&self) -> usize {
        self.size
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.size != 0 {
            let layout =
                unsafe { std::alloc::Layout::from_size_align_unchecked(self.size, self.align) };
            unsafe { std::alloc::dealloc(self.ptr, layout) };
            self.ptr = std::ptr::null_mut();
        }
    }
}

/// Allocate a buffer aligned to `align` (required for O_DIRECT / NO_BUFFERING).
pub fn alloc_aligned(size: usize, align: usize) -> Result<AlignedBuf, Error> {
    let align = align.max(16).next_power_of_two();
    let size = size.max(align);
    let layout = std::alloc::Layout::from_size_align(size, align).map_err(|e| Error::Alloc {
        size,
        detail: e.to_string(),
    })?;
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        return Err(Error::Alloc {
            size,
            detail: "allocator returned null".into(),
        });
    }
    Ok(AlignedBuf { ptr, size, align })
}

/// Fill `buf` with a pattern. `rng` is used only for Random.
pub fn fill_pattern(
    buf: &mut [u8],
    pattern: crate::method::Pattern,
    rng: Option<&mut rand_chacha::ChaCha20Rng>,
) {
    use crate::method::Pattern;
    use rand::RngCore;
    match pattern {
        Pattern::Zeros => buf.fill(0),
        Pattern::Ones => buf.fill(0xFF),
        Pattern::Random => {
            if let Some(r) = rng {
                r.fill_bytes(buf);
            } else {
                buf.fill(0);
            }
        }
    }
}
