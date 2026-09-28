//! Copying and moving files and folder trees.
//!
//! A transfer first plans the whole tree (so progress has a total), creates all folders,
//! then copies files with several workers at once — many small files are bound by
//! per-file latency, not bandwidth, so parallel requests help a lot on SSDs and network
//! shares. Folders get their final permissions and times at the end, because writing
//! into a folder changes its time.

use super::device::{DeviceClass, classify};
use super::fastcopy::{self, copy_contents, is_cancelled_error};
use super::job::{Answer, JobCtx, Phase, Question};
use super::names::free_name;
use std::ffi::{CString, OsStr};
use std::fs::{self, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Move,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Pace writes to slow (removable, rotational) targets.
    pub smooth_writes: bool,
    /// Override the automatic number of workers.
    pub workers: Option<usize>,
}

#[derive(Debug)]
struct FileTask {
    src: PathBuf,
    dst: PathBuf,
    size: u64,
    mode: u32,
    atime: (i64, i64),
    mtime: (i64, i64),
    ino: u64,
}

#[derive(Debug)]
struct DirTask {
    src: PathBuf,
    dst: PathBuf,
    mode: u32,
    atime: (i64, i64),
    mtime: (i64, i64),
    /// We created it (so we own its metadata); false when merging into an existing folder.
    created: bool,
}

#[derive(Default)]
struct Plan {
    dirs: Vec<DirTask>,
    files: Vec<FileTask>,
    /// (source link, its target, destination)
    links: Vec<(PathBuf, PathBuf, PathBuf)>,
    /// Source folders to remove after a cross-device move, deepest first.
    move_leftovers: Vec<PathBuf>,
}

fn umask() -> u32 {
    static UMASK: OnceLock<u32> = OnceLock::new();
    *UMASK.get_or_init(|| {
        fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("Umask:").map(|v| v.trim().to_string())))
            .and_then(|v| u32::from_str_radix(&v, 8).ok())
            .unwrap_or(0o022)
    })
}

fn cpath(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

fn times(m: &fs::Metadata) -> ((i64, i64), (i64, i64)) {
    ((m.atime(), m.atime_nsec()), (m.mtime(), m.mtime_nsec()))
}

fn set_times_path(p: &Path, atime: (i64, i64), mtime: (i64, i64), follow: bool) {
    let Ok(c) = cpath(p) else { return };
    let ts = [
        libc::timespec { tv_sec: atime.0, tv_nsec: atime.1 },
        libc::timespec { tv_sec: mtime.0, tv_nsec: mtime.1 },
    ];
    let flags = if follow { 0 } else { libc::AT_SYMLINK_NOFOLLOW };
    unsafe {
        libc::utimensat(libc::AT_FDCWD, c.as_ptr(), ts.as_ptr(), flags);
    }
}

/// rename(2) that refuses to replace an existing target.
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let (f, t) = (cpath(from)?, cpath(to)?);
    let r = unsafe { libc::renameat2(libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE) };
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

fn remove_any(p: &Path) -> io::Result<()> {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(p),
        Ok(_) => fs::remove_file(p),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Runs `op` until it succeeds, the user skips it (`Ok(None)`) or cancels (`Err`).
fn with_retry<T>(ctx: &JobCtx, path: &Path, mut op: impl FnMut() -> io::Result<T>) -> io::Result<Option<T>> {
    loop {
        match op() {
            Ok(v) => return Ok(Some(v)),
            Err(e) if is_cancelled_error(&e) => return Err(e),
            Err(e) => match ctx.ask(Question::Error { path: path.to_path_buf(), message: e.to_string() }) {
                Answer::Retry => continue,
                Answer::Cancel => return Err(fastcopy::cancelled()),
                _ => {
                    ctx.record_error(format!("{}: {}", path.display(), e));
                    return Ok(None);
                }
            },
        }
    }
}

/// What to do about an existing destination.
enum Resolve {
    /// Write to this path (the original, or a free name).
    Into(PathBuf),
    /// Replace the existing file at this path once the new one is complete.
    Replace(PathBuf),
    /// Merge into the existing folder.
    Merge,
    Skip,
}

fn resolve_conflict(ctx: &JobCtx, src: &Path, src_meta: &fs::Metadata, dst: &Path) -> io::Result<Resolve> {
    let dst_meta = match fs::symlink_metadata(dst) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Resolve::Into(dst.to_path_buf())),
        Err(e) => return Err(e),
    };
    if src_meta.is_dir() && dst_meta.is_dir() {
        return Ok(Resolve::Merge);
    }
    let q = Question::Conflict {
        src: src.to_path_buf(),
        dst: dst.to_path_buf(),
        src_size: src_meta.len(),
        dst_size: dst_meta.len(),
        src_mtime: src_meta.mtime(),
        dst_mtime: dst_meta.mtime(),
        dst_is_dir: dst_meta.is_dir(),
    };
    match ctx.ask(q) {
        Answer::Replace if src_meta.is_file() && dst_meta.is_file() => Ok(Resolve::Replace(dst.to_path_buf())),
        Answer::Replace => {
            remove_any(dst)?;
            Ok(Resolve::Into(dst.to_path_buf()))
        }
        Answer::KeepBoth => {
            let parent = dst.parent().unwrap_or(Path::new("/"));
            let name = dst.file_name().unwrap_or_default();
            Ok(Resolve::Into(parent.join(free_name(parent, name, None))))
        }
        Answer::Cancel => Err(fastcopy::cancelled()),
        _ => Ok(Resolve::Skip),
    }
}

/// Copies or moves `sources` into `dest_dir`. Returns the top-level destinations written.
pub fn transfer(ctx: &JobCtx, sources: &[PathBuf], dest_dir: &Path, mode: Mode, opts: &Options) -> io::Result<Vec<PathBuf>> {
    ctx.set_phase(Phase::Preparing);
    let dest_meta = fs::metadata(dest_dir)?;
    let dest_class = classify(dest_dir);
    let mut plan = Plan::default();
    let mut tops = Vec::new();

    for src in sources {
        if ctx.is_cancelled() {
            return Err(fastcopy::cancelled());
        }
        let Some(name) = src.file_name() else { continue };
        let Some(src_meta) = with_retry(ctx, src, || fs::symlink_metadata(src))? else { continue };
        if src_meta.is_dir() && dest_dir.starts_with(src) {
            ctx.record_error(format!("Cannot put “{}” into itself.", src.display()));
            continue;
        }
        let mut dst = dest_dir.join(name);
        if dst == *src {
            if mode == Mode::Move {
                continue; // Moving onto itself: nothing to do.
            }
            dst = dest_dir.join(free_name(dest_dir, name, Some("copy")));
        }

        // Moves within one filesystem are a rename, whatever the size.
        if mode == Mode::Move && src_meta.dev() == dest_meta.dev() {
            ctx.set_current(name.to_string_lossy());
            move_by_rename(ctx, src, &src_meta, &dst)?;
            tops.push(dst);
            continue;
        }

        let dst = match resolve_conflict(ctx, src, &src_meta, &dst)? {
            // The file task meets the existing file again and replaces it then.
            Resolve::Into(p) | Resolve::Replace(p) => p,
            Resolve::Merge => dst,
            Resolve::Skip => continue,
        };
        ctx.set_current(format!("Preparing {}", name.to_string_lossy()));
        plan_tree(ctx, src, &src_meta, &dst, mode, &mut plan)?;
        tops.push(dst);
    }

    let total: u64 = plan.files.iter().map(|f| f.size).sum();
    ctx.bytes_total.fetch_add(total, Ordering::Relaxed);
    ctx.files_total.fetch_add(plan.files.len() as u64 + plan.links.len() as u64, Ordering::Relaxed);
    ctx.set_phase(Phase::Working);

    // Folders first, in order, so every file has its parent.
    for d in &mut plan.dirs {
        if ctx.is_cancelled() {
            return Err(fastcopy::cancelled());
        }
        if !d.created {
            continue;
        }
        let res = with_retry(ctx, &d.dst, || match fs::create_dir(&d.dst) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && d.dst.is_dir() => Ok(false),
            Err(e) => Err(e),
            Ok(()) => Ok(true),
        })?;
        d.created = res.unwrap_or(false);
    }

    let throttle = opts.smooth_writes && dest_class.is_slow();
    let src_class = sources.first().and_then(|s| s.parent()).map(classify).unwrap_or(DeviceClass::Fast);
    let workers = opts.workers.unwrap_or_else(|| dest_class.workers().min(src_class.workers())).max(1);
    if src_class == DeviceClass::Rotational {
        // Reading in inode order keeps a spinning source from seeking back and forth.
        plan.files.sort_by_key(|f| f.ino);
    } else {
        // Spread the workers over different folders: creating files in one folder is
        // serialized by the filesystem, different folders proceed in parallel.
        interleave_by_folder(&mut plan.files);
    }
    copy_files(ctx, &plan.files, mode, throttle, workers)?;

    for (src, target, dst) in &plan.links {
        if ctx.is_cancelled() {
            return Err(fastcopy::cancelled());
        }
        let made = with_retry(ctx, dst, || std::os::unix::fs::symlink(target, dst))?;
        if made.is_some() && mode == Mode::Move {
            let _ = fs::remove_file(src);
        }
        ctx.files_done.fetch_add(1, Ordering::Relaxed);
    }

    // Folder metadata last (deepest first); then clean up after cross-device moves.
    for d in plan.dirs.iter().rev().filter(|d| d.created) {
        let _ = fs::set_permissions(&d.dst, fs::Permissions::from_mode(d.mode & 0o7777));
        set_times_path(&d.dst, d.atime, d.mtime, true);
    }
    if mode == Mode::Move {
        for dir in &plan.move_leftovers {
            // Fails harmlessly where something was skipped and stayed behind.
            let _ = fs::remove_dir(dir);
        }
    }

    if dest_class.is_slow() {
        // Report "done" only when the data is on the device: unplugging is safe right after.
        ctx.set_phase(Phase::Flushing);
        ctx.set_current("Writing cached data to the drive…");
        let _ = fastcopy::syncfs(dest_dir);
    }
    ctx.set_phase(Phase::Done);
    Ok(tops)
}

/// Moves within one filesystem: rename, merging into existing folders entry by entry.
fn move_by_rename(ctx: &JobCtx, src: &Path, src_meta: &fs::Metadata, dst: &Path) -> io::Result<()> {
    match rename_noreplace(src, dst) {
        Ok(()) => {
            ctx.files_done.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        Err(e) if e.raw_os_error() == Some(libc::EEXIST) || e.raw_os_error() == Some(libc::ENOTEMPTY) => {}
        Err(e) if e.raw_os_error() == Some(libc::EINVAL) => {
            // Old kernels or filesystems without RENAME_NOREPLACE: check, then rename.
            if fs::symlink_metadata(dst).is_err() {
                return with_retry(ctx, src, || fs::rename(src, dst)).map(|_| ());
            }
        }
        Err(e) => {
            return with_retry(ctx, src, || Err::<(), _>(io::Error::new(e.kind(), e.to_string()))).map(|_| ());
        }
    }
    match resolve_conflict(ctx, src, src_meta, dst)? {
        Resolve::Skip => Ok(()),
        // rename(2) replaces a file atomically.
        Resolve::Into(p) | Resolve::Replace(p) => with_retry(ctx, src, || fs::rename(src, &p)).map(|_| ()),
        Resolve::Merge => {
            let entries: Vec<_> = fs::read_dir(src)?.filter_map(Result::ok).collect();
            for e in entries {
                if ctx.is_cancelled() {
                    return Err(fastcopy::cancelled());
                }
                let child = e.path();
                let Ok(m) = fs::symlink_metadata(&child) else { continue };
                move_by_rename(ctx, &child, &m, &dst.join(e.file_name()))?;
            }
            let _ = fs::remove_dir(src);
            Ok(())
        }
    }
}

fn plan_tree(ctx: &JobCtx, src: &Path, meta: &fs::Metadata, dst: &Path, mode: Mode, plan: &mut Plan) -> io::Result<()> {
    if ctx.is_cancelled() {
        return Err(fastcopy::cancelled());
    }
    let ft = meta.file_type();
    let (atime, mtime) = times(meta);
    if ft.is_symlink() {
        let Some(target) = with_retry(ctx, src, || fs::read_link(src))? else { return Ok(()) };
        plan.links.push((src.to_path_buf(), target, dst.to_path_buf()));
    } else if ft.is_dir() {
        let exists = dst.is_dir();
        plan.dirs.push(DirTask { src: src.to_path_buf(), dst: dst.to_path_buf(), mode: meta.mode(), atime, mtime, created: !exists });
        let Some(entries) = with_retry(ctx, src, || fs::read_dir(src)?.collect::<io::Result<Vec<_>>>())? else { return Ok(()) };
        for e in entries {
            let child = e.path();
            let Some(m) = with_retry(ctx, &child, || fs::symlink_metadata(&child))? else { continue };
            plan_tree(ctx, &child, &m, &dst.join(e.file_name()), mode, plan)?;
        }
        if mode == Mode::Move {
            plan.move_leftovers.push(src.to_path_buf());
        }
    } else if ft.is_file() {
        plan.files.push(FileTask { src: src.to_path_buf(), dst: dst.to_path_buf(), size: meta.len(), mode: meta.mode(), atime, mtime, ino: meta.ino() });
    } else {
        ctx.record_error(format!("{}: special files are not copied", src.display()));
    }
    Ok(())
}

/// Reorders files round-robin over their folders: first file of every folder, then the
/// second of every folder, and so on.
fn interleave_by_folder(files: &mut Vec<FileTask>) {
    let mut slot: std::collections::HashMap<PathBuf, u32> = std::collections::HashMap::new();
    let mut keyed: Vec<(u32, usize, FileTask)> = Vec::with_capacity(files.len());
    let mut folder_ids: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
    for f in files.drain(..) {
        let parent = f.dst.parent().map(Path::to_path_buf).unwrap_or_default();
        let n = folder_ids.len();
        let folder = *folder_ids.entry(parent.clone()).or_insert(n);
        let s = slot.entry(parent).or_insert(0);
        keyed.push((*s, folder, f));
        *s += 1;
    }
    keyed.sort_by_key(|(s, folder, _)| (*s, *folder));
    files.extend(keyed.into_iter().map(|(_, _, f)| f));
}

fn copy_files(ctx: &JobCtx, files: &[FileTask], mode: Mode, throttle: bool, workers: usize) -> io::Result<()> {
    let next = AtomicUsize::new(0);
    let failure: Mutex<Option<io::Error>> = Mutex::new(None);
    let workers = workers.min(files.len()).max(1);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                let mut buf = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(task) = files.get(i) else { break };
                    if ctx.is_cancelled() {
                        break;
                    }
                    ctx.set_current(task.src.file_name().unwrap_or_default().to_string_lossy());
                    match copy_one(ctx, task, mode, throttle, &mut buf) {
                        Ok(()) => {
                            ctx.files_done.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            let mut f = failure.lock().unwrap();
                            if f.is_none() {
                                *f = Some(e);
                            }
                            ctx.cancel();
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = failure.into_inner().unwrap() {
        return Err(e);
    }
    if ctx.is_cancelled() {
        return Err(fastcopy::cancelled());
    }
    Ok(())
}

/// Copies one file. Errors the user chose to skip return `Ok`.
fn copy_one(ctx: &JobCtx, t: &FileTask, mode: Mode, throttle: bool, buf: &mut Vec<u8>) -> io::Result<()> {
    let src_meta = match fs::symlink_metadata(&t.src) {
        Ok(m) => m,
        Err(e) => {
            ctx.record_error(format!("{}: {}", t.src.display(), e));
            return Ok(());
        }
    };
    let (dst, replace) = match resolve_conflict(ctx, &t.src, &src_meta, &t.dst)? {
        Resolve::Into(p) => (p, None),
        Resolve::Replace(p) => {
            // Write next to it under a temporary name; rename over it when complete.
            let parent = p.parent().unwrap_or(Path::new("/")).to_path_buf();
            let mut tmp = std::ffi::OsString::from(".");
            tmp.push(p.file_name().unwrap_or_default());
            tmp.push(".fbpart");
            let tmp = if fs::symlink_metadata(parent.join(&tmp)).is_ok() { parent.join(free_name(&parent, &tmp, None)) } else { parent.join(tmp) };
            (tmp, Some(p))
        }
        Resolve::Merge | Resolve::Skip => {
            ctx.add_bytes(t.size);
            return Ok(());
        }
    };
    let done = with_retry(ctx, &t.src, || {
        let mut counted = 0;
        let r = write_file(ctx, t, &dst, throttle, buf, &mut counted);
        if r.is_err() {
            // A retry starts over: take back what the failed attempt counted.
            ctx.bytes_done.fetch_sub(counted, Ordering::Relaxed);
        }
        r
    })?;
    if done.is_none() {
        ctx.add_bytes(t.size);
        return Ok(());
    }
    if let Some(target) = replace {
        if let Err(e) = fs::rename(&dst, &target) {
            let _ = fs::remove_file(&dst);
            ctx.record_error(format!("{}: {}", target.display(), e));
            return Ok(());
        }
    }
    if mode == Mode::Move {
        let _ = with_retry(ctx, &t.src, || fs::remove_file(&t.src))?;
    }
    Ok(())
}

fn write_file(ctx: &JobCtx, t: &FileTask, dst: &Path, throttle: bool, buf: &mut Vec<u8>, counted: &mut u64) -> io::Result<()> {
    let src = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&t.src)?;
    let perm = t.mode & 0o7777;
    let out = OpenOptions::new().write(true).create_new(true).mode(perm & 0o777).open(dst)?;
    let result = (|| {
        copy_contents(ctx, &src, &out, t.size, throttle, buf, counted)?;
        // Only when the umask changed what we asked for (saves a syscall per file).
        if perm != (perm & 0o777 & !umask()) {
            out.set_permissions(fs::Permissions::from_mode(perm))?;
        }
        let ts = [
            libc::timespec { tv_sec: t.atime.0, tv_nsec: t.atime.1 },
            libc::timespec { tv_sec: t.mtime.0, tv_nsec: t.mtime.1 },
        ];
        unsafe {
            libc::futimens(out.as_raw_fd(), ts.as_ptr());
        }
        Ok(())
    })();
    drop(out);
    if result.is_err() {
        let _ = fs::remove_file(dst);
    }
    result
}

/// The name of `p` for messages.
pub fn display_name(p: &Path) -> String {
    p.file_name().unwrap_or(OsStr::new("/")).to_string_lossy().into_owned()
}
