//! Copying the bytes of one file as fast as the kernel allows.
//!
//! In order of preference: a reflink (`FICLONE`, instant on btrfs/xfs/bcachefs), in-kernel
//! `copy_file_range` (no user-space copy, server-side on NFS 4.2/SMB), and plain read/write.
//!
//! With `throttle` set (slow targets such as USB sticks), data is pushed to the device in
//! fixed windows: the previous window is waited for and dropped from the page cache before
//! the next one starts. That keeps only a few megabytes dirty, so the progress bar shows the
//! real device speed, other programs do not stall behind a huge writeback, and unmounting
//! afterwards is instant instead of freezing for minutes.

use super::job::JobCtx;
use std::io;
use std::os::fd::{AsRawFd, RawFd};

/// Writeback window for throttled copies.
const WINDOW: u64 = 8 << 20;
/// Chunk size for unthrottled `copy_file_range` (keeps cancel and progress responsive).
const CHUNK: u64 = 32 << 20;
/// Size of the fallback read/write buffer.
pub const BUF_SIZE: usize = 1 << 20;
/// Files at least this large get `fallocate` (less fragmentation, early ENOSPC).
const PREALLOC_MIN: u64 = 1 << 20;

pub fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled")
}

pub fn is_cancelled_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Interrupted && e.to_string() == "cancelled"
}

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

/// Tracks what has been written and paces writeback for throttled copies.
struct Pacer {
    fd: RawFd,
    src: RawFd,
    enabled: bool,
    /// Start of the window currently being filled.
    window_start: u64,
    /// The window handed to writeback, not yet waited for.
    in_flight: Option<(u64, u64)>,
}

impl Pacer {
    fn advance(&mut self, pos: u64) {
        if !self.enabled || pos - self.window_start < WINDOW {
            return;
        }
        let len = pos - self.window_start;
        unsafe {
            // Start writing this window out now…
            libc::sync_file_range(self.fd, self.window_start as i64, len as i64, libc::SYNC_FILE_RANGE_WRITE);
            // …and wait for the one before, then forget its pages.
            if let Some((s, l)) = self.in_flight.take() {
                libc::sync_file_range(
                    self.fd,
                    s as i64,
                    l as i64,
                    libc::SYNC_FILE_RANGE_WAIT_BEFORE | libc::SYNC_FILE_RANGE_WRITE | libc::SYNC_FILE_RANGE_WAIT_AFTER,
                );
                libc::posix_fadvise(self.fd, s as i64, l as i64, libc::POSIX_FADV_DONTNEED);
                libc::posix_fadvise(self.src, s as i64, l as i64, libc::POSIX_FADV_DONTNEED);
            }
        }
        self.in_flight = Some((self.window_start, len));
        self.window_start = pos;
    }

    fn finish(&mut self, pos: u64) {
        if !self.enabled {
            return;
        }
        unsafe {
            let flags = libc::SYNC_FILE_RANGE_WAIT_BEFORE | libc::SYNC_FILE_RANGE_WRITE | libc::SYNC_FILE_RANGE_WAIT_AFTER;
            if let Some((s, l)) = self.in_flight.take() {
                libc::sync_file_range(self.fd, s as i64, l as i64, flags);
                libc::posix_fadvise(self.fd, s as i64, l as i64, libc::POSIX_FADV_DONTNEED);
            }
            let len = pos.saturating_sub(self.window_start);
            if len > 0 {
                libc::sync_file_range(self.fd, self.window_start as i64, len as i64, flags);
                libc::posix_fadvise(self.fd, self.window_start as i64, len as i64, libc::POSIX_FADV_DONTNEED);
            }
        }
    }
}

/// Which mechanism ended up copying a file (for tests and statistics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Reflink,
    CopyFileRange,
    ReadWrite,
    Empty,
}

/// Copies `size` bytes from `src` to the freshly created, empty `dst`.
/// `buf` is the caller's reusable buffer for the read/write fallback.
pub fn copy_contents(
    ctx: &JobCtx,
    src: &std::fs::File,
    dst: &std::fs::File,
    size: u64,
    throttle: bool,
    buf: &mut Vec<u8>,
    counted: &mut u64,
) -> io::Result<Method> {
    let (sfd, dfd) = (src.as_raw_fd(), dst.as_raw_fd());
    if size == 0 {
        return Ok(Method::Empty);
    }
    if unsafe { libc::ioctl(dfd, libc::FICLONE, sfd) } == 0 {
        ctx.add_bytes(size);
        *counted += size;
        return Ok(Method::Reflink);
    }
    if size >= PREALLOC_MIN {
        unsafe {
            libc::posix_fadvise(sfd, 0, 0, libc::POSIX_FADV_SEQUENTIAL);
            // Best effort: not every filesystem can preallocate.
            libc::fallocate(dfd, 0, 0, size as i64);
        }
    }
    let mut pacer = Pacer { fd: dfd, src: sfd, enabled: throttle, window_start: 0, in_flight: None };
    let mut pos: u64 = 0;
    let mut method = Method::CopyFileRange;
    let step = if throttle { WINDOW } else { CHUNK };
    loop {
        ctx.wait_while_paused();
        if ctx.is_cancelled() {
            return Err(cancelled());
        }
        let want = step.min(size.saturating_sub(pos).max(1)) as usize;
        let n = if method == Method::CopyFileRange {
            let r = unsafe { libc::copy_file_range(sfd, std::ptr::null_mut(), dfd, std::ptr::null_mut(), want, 0) };
            if r < 0 {
                let e = io::Error::last_os_error();
                match e.raw_os_error() {
                    Some(libc::EINTR) => continue,
                    // Not possible between these files: fall back, continuing at the same offsets.
                    Some(libc::EXDEV | libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP | libc::EBADF | libc::EPERM | libc::ETXTBSY) => {
                        method = Method::ReadWrite;
                        continue;
                    }
                    _ => return Err(e),
                }
            }
            r as usize
        } else {
            if buf.len() < BUF_SIZE {
                buf.resize(BUF_SIZE, 0);
            }
            let want = want.min(buf.len());
            let r = unsafe { libc::read(sfd, buf.as_mut_ptr().cast(), want) };
            if r < 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(e);
            }
            let r = r as usize;
            let mut off = 0;
            while off < r {
                let w = unsafe { libc::write(dfd, buf[off..r].as_ptr().cast(), r - off) };
                if w < 0 {
                    let e = io::Error::last_os_error();
                    if e.raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    return Err(e);
                }
                off += w as usize;
            }
            r
        };
        if n == 0 {
            break; // End of file (it may have shrunk while we copied).
        }
        pos += n as u64;
        ctx.add_bytes(n as u64);
        *counted += n as u64;
        pacer.advance(pos);
    }
    // Preallocation may have left the file longer than what was actually copied.
    if pos != size {
        cvt(unsafe { libc::ftruncate(dfd, pos as i64) })?;
    }
    pacer.finish(pos);
    Ok(method)
}

/// Flushes everything cached for the filesystem holding `dir` (after copies to slow drives).
pub fn syncfs(dir: &std::path::Path) -> io::Result<()> {
    let f = std::fs::File::open(dir)?;
    cvt(unsafe { libc::syncfs(f.as_raw_fd()) }).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn roundtrip(size: usize, throttle: bool) -> Method {
        let dir = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..size).map(|i| (i * 7 % 251) as u8).collect();
        let sp = dir.path().join("s");
        std::fs::File::create(&sp).unwrap().write_all(&data).unwrap();
        let src = std::fs::File::open(&sp).unwrap();
        let dst = std::fs::File::create(dir.path().join("d")).unwrap();
        let ctx = JobCtx::new();
        let mut buf = Vec::new();
        let m = copy_contents(&ctx, &src, &dst, size as u64, throttle, &mut buf, &mut 0).unwrap();
        drop(dst);
        let mut out = Vec::new();
        std::fs::File::open(dir.path().join("d")).unwrap().read_to_end(&mut out).unwrap();
        assert_eq!(out, data);
        assert_eq!(ctx.snapshot().bytes_done, size as u64);
        m
    }

    #[test]
    fn copies_small_medium_and_large_files() {
        assert_eq!(roundtrip(0, false), Method::Empty);
        roundtrip(1, false);
        roundtrip(100_000, false);
        roundtrip(20 << 20, true);
        roundtrip((3 << 20) + 17, false);
    }

    #[test]
    fn source_shorter_than_expected_is_truncated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("s"), vec![1u8; 10]).unwrap();
        let src = std::fs::File::open(dir.path().join("s")).unwrap();
        let dst = std::fs::File::create(dir.path().join("d")).unwrap();
        let ctx = JobCtx::new();
        copy_contents(&ctx, &src, &dst, 5 << 20, false, &mut Vec::new(), &mut 0).unwrap();
        assert_eq!(std::fs::metadata(dir.path().join("d")).unwrap().len(), 10);
    }

    #[test]
    fn cancel_stops_copy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("s"), vec![1u8; 4 << 20]).unwrap();
        let src = std::fs::File::open(dir.path().join("s")).unwrap();
        let dst = std::fs::File::create(dir.path().join("d")).unwrap();
        let ctx = JobCtx::new();
        ctx.cancel();
        let e = copy_contents(&ctx, &src, &dst, 4 << 20, false, &mut Vec::new(), &mut 0).unwrap_err();
        assert!(is_cancelled_error(&e));
    }
}
