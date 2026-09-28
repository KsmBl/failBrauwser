//! Working inside archives from the UI: opening entries with applications (edits are saved
//! back into the archive), copying and moving in and out, deleting, renaming, creating,
//! extracting and compressing. Every archive operation runs as a job.

use super::clipboard::ClipSource;
use super::fileops;
use super::jobs::Outcome;
use super::model::Item;
use super::pane::Pane;
use super::util;
use super::window::Window;
use failbrauwser::archive::vfs::Vfs;
use failbrauwser::archive::{ArchiveError, CREATE_FORMATS, is_browsable_name};
use failbrauwser::location::{ArchiveLoc, Location};
use failbrauwser::ops::copy::{self, Mode};
use failbrauwser::ops::job::{Answer, JobCtx, Question};
use failbrauwser::ops::{delete, names};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

fn io(e: ArchiveError) -> std::io::Error {
    std::io::Error::other(e.message)
}

fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

/// The archive folder a tab shows, if it shows one.
fn archive_of(loc: &Location) -> Option<&ArchiveLoc> {
    match loc {
        Location::Archive(a) => Some(a),
        _ => None,
    }
}

/// Refreshes every tab that shows a folder of this archive (after edits the file monitor
/// catches the outer file; nested archives need a nudge too).
fn refresh_archive_tabs(w: &Window, a: &ArchiveLoc) {
    for win in w.app.windows() {
        for p in win.panes() {
            if let Location::Archive(x) = p.location() {
                if x.file == a.file {
                    p.schedule_refresh();
                }
            }
        }
    }
}

// ── Opening entries ─────────────────────────────────────────────────

thread_local! {
    /// Files extracted for an application; their monitors save edits back.
    static OPENED: RefCell<Vec<gio::FileMonitor>> = const { RefCell::new(Vec::new()) };
}

pub fn open_entry(w: &Window, item: &Item) {
    let (Item::Archive(node), Location::Archive(loc)) = (item, w.current_location()) else { return };
    if node.is_dir {
        return;
    }
    let entry = node.path.clone();
    let watched_entry = entry.clone();
    let loc2 = loc.clone();
    let weak = Rc::downgrade(&w.me());
    let title = format!("Opening “{}”", node.name);
    w.app.jobs.start(
        w.win.upcast_ref(),
        title,
        move |_| {
            let v = Vfs::global();
            let dir = v.scratch_dir("open").map_err(io)?;
            v.extract(&loc2, std::slice::from_ref(&entry), &dir).map_err(io)
        },
        move |outcome| {
            let (Some(w), Outcome::Done(paths)) = (weak.upgrade(), outcome) else { return };
            let Some(path) = paths.first().cloned() else { return };
            util::open_with_default(w.win.upcast_ref(), &path);
            watch_opened(&w, loc, watched_entry, path);
        },
    );
}

/// Saves an extracted file back into its archive whenever an application writes it.
fn watch_opened(w: &Window, loc: ArchiveLoc, entry: String, path: PathBuf) {
    if entry.is_empty() {
        return;
    }
    let Ok(monitor) = gio::File::for_path(&path).monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { return };
    let pending = Rc::new(std::cell::Cell::new(false));
    let weak = Rc::downgrade(&w.me());
    monitor.connect_changed(move |_, _, _, ev| {
        use gio::FileMonitorEvent as E;
        if !matches!(ev, E::ChangesDoneHint | E::Created | E::MovedIn | E::Renamed | E::Changed) || pending.replace(true) {
            return;
        }
        let (weak, loc, entry, path, pending) = (weak.clone(), loc.clone(), entry.clone(), path.clone(), pending.clone());
        // Editors write in bursts; save once it settled.
        glib::timeout_add_local_once(Duration::from_millis(800), move || {
            pending.set(false);
            let Some(w) = weak.upgrade() else { return };
            if !path.is_file() {
                return;
            }
            let (loc2, entry2, path2) = (loc.clone(), entry.clone(), path.clone());
            let w2 = Rc::downgrade(&w);
            let name = leaf(&entry).to_string();
            w.app.jobs.start(
                w.win.upcast_ref(),
                format!("Saving “{name}” into the archive"),
                move |_| Vfs::global().add(&loc2, &[(path2, entry2)]).map(|_| Vec::new()).map_err(io),
                move |outcome| {
                    if let Some(w) = w2.upgrade() {
                        if matches!(outcome, Outcome::Done(_)) {
                            w.set_status(&format!("Saved changes to “{name}” into the archive."));
                        }
                        refresh_archive_tabs(&w, &loc);
                    }
                },
            );
        });
    });
    OPENED.with(|o| o.borrow_mut().push(monitor));
}

// ── Transfers ───────────────────────────────────────────────────────

/// Resolves name clashes with what the archive folder already holds. Returns the entry
/// names to write (`None` = skip).
fn plan_names(ctx: &JobCtx, a: &ArchiveLoc, sources: &[(PathBuf, String)]) -> std::io::Result<Vec<Option<String>>> {
    let archive = Vfs::global().archive(a).map_err(io)?;
    let mut out = Vec::new();
    for (src, name) in sources {
        let target = a.entry_path(name);
        let Some(existing) = archive.tree.get(&target) else {
            out.push(Some(target));
            continue;
        };
        let meta = std::fs::metadata(src).ok();
        let q = Question::Conflict {
            src: src.clone(),
            dst: Location::Archive(a.with_inner(&target)).display().into(),
            src_size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            dst_size: existing.size,
            src_mtime: meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0),
            dst_mtime: existing.mtime.unwrap_or(0),
            dst_is_dir: existing.is_dir,
        };
        match ctx.ask(q) {
            Answer::Replace => out.push(Some(target)),
            Answer::KeepBoth => {
                let free = names::free_name_in(&|n| archive.tree.get(&a.entry_path(&n.to_string_lossy())).is_some(), OsStr::new(name), None);
                out.push(Some(a.entry_path(&free.to_string_lossy())));
            }
            Answer::Cancel => return Err(failbrauwser::ops::fastcopy::cancelled()),
            _ => out.push(None),
        }
    }
    Ok(out)
}

/// Copies or moves anything involving an archive (the plain folder-to-folder case is
/// handled by the regular copy engine).
pub fn transfer(w: &Window, sources: Vec<ClipSource>, dest: Location, mode: Mode, after: Box<dyn FnOnce()>) {
    let verb = if mode == Mode::Move { "Moving" } else { "Copying" };
    let n = sources.len();
    let title = format!("{verb} {n} item{} to “{}”", if n == 1 { "" } else { "s" }, dest.title());
    let opts = copy::Options { smooth_writes: w.app.settings.borrow().smooth_writes, workers: None };
    let weak = Rc::downgrade(&w.me());
    let dest2 = dest.clone();
    let touched: Vec<ArchiveLoc> = sources
        .iter()
        .filter_map(|s| match s {
            ClipSource::Archive(a, _) => Some(a.clone()),
            _ => None,
        })
        .chain(archive_of(&dest).cloned())
        .collect();
    w.app.jobs.start(
        w.win.upcast_ref(),
        title,
        move |ctx| transfer_work(ctx, sources, &dest2, mode, &opts),
        move |outcome| {
            after();
            let Some(w) = weak.upgrade() else { return };
            for a in &touched {
                refresh_archive_tabs(&w, a);
            }
            if let Outcome::Done(created) = outcome {
                fileops::select_created(&w, &dest, created);
            }
        },
    );
}

fn transfer_work(ctx: &JobCtx, sources: Vec<ClipSource>, dest: &Location, mode: Mode, opts: &copy::Options) -> std::io::Result<Vec<PathBuf>> {
    let v = Vfs::global();
    let mut created = Vec::new();
    // Moves inside one archive are renames.
    if let (Location::Archive(d), Mode::Move) = (dest, mode) {
        let same: Vec<String> = sources
            .iter()
            .filter_map(|s| match s {
                ClipSource::Archive(a, e) if a.archive_key() == d.archive_key() => Some(e.clone()),
                _ => None,
            })
            .collect();
        if same.len() == sources.len() {
            for e in same {
                if parent_of(&e) == d.inner || d.inner == e || d.inner.starts_with(&format!("{e}/")) {
                    continue;
                }
                let to = d.entry_path(leaf(&e));
                ctx.set_current(leaf(&e).to_string());
                v.rename(d, &e, &to).map_err(io)?;
                created.push(PathBuf::from(leaf(&e)));
            }
            return Ok(created);
        }
    }

    // Everything coming out of an archive is extracted to a staging folder first: next to
    // the destination when that is a folder (so the final step is a rename), else in the cache.
    let staging = match dest {
        Location::Dir(d) => {
            let s = tempfile::Builder::new().prefix(".fb-extract-").tempdir_in(d)?;
            s.keep()
        }
        _ => v.scratch_dir("stage").map_err(io)?,
    };
    let result = (|| {
        let mut local: Vec<PathBuf> = Vec::new();
        let mut from_archives: Vec<(ArchiveLoc, String)> = Vec::new();
        for s in &sources {
            match s {
                ClipSource::Local(p) => local.push(p.clone()),
                ClipSource::Archive(a, e) => {
                    ctx.set_current(format!("Extracting {}", leaf(e)));
                    let got = v.extract(a, std::slice::from_ref(e), &staging).map_err(io)?;
                    local.extend(got);
                    from_archives.push((a.clone(), e.clone()));
                }
            }
        }
        match dest {
            Location::Dir(d) => {
                // Extracted items move out of staging (same filesystem: renames); local
                // sources copy or move as usual. Both ask about conflicts.
                let (staged, plain): (Vec<PathBuf>, Vec<PathBuf>) = local.into_iter().partition(|p| p.starts_with(&staging));
                if !staged.is_empty() {
                    created.extend(copy::transfer(ctx, &staged, d, Mode::Move, opts)?);
                }
                if !plain.is_empty() {
                    created.extend(copy::transfer(ctx, &plain, d, mode, opts)?);
                }
            }
            Location::Archive(d) => {
                let items: Vec<(PathBuf, String)> = local.iter().map(|p| (p.clone(), p.file_name().unwrap_or_default().to_string_lossy().into_owned())).collect();
                let targets = plan_names(ctx, d, &items)?;
                let add: Vec<(PathBuf, String)> = items.iter().zip(&targets).filter_map(|((p, _), t)| t.clone().map(|t| (p.clone(), t))).collect();
                if !add.is_empty() {
                    ctx.set_current(format!("Adding {} item{} to the archive", add.len(), if add.len() == 1 { "" } else { "s" }));
                    v.add(d, &add).map_err(io)?;
                }
                created.extend(add.iter().map(|(_, t)| PathBuf::from(leaf(t))));
                if mode == Mode::Move {
                    let moved_local: Vec<PathBuf> = local.iter().zip(&targets).filter(|(p, t)| t.is_some() && !p.starts_with(&staging)).map(|(p, _)| p.clone()).collect();
                    delete::delete_permanently(ctx, &moved_local)?;
                }
            }
            Location::Drives => {}
        }
        if mode == Mode::Move {
            for (a, e) in &from_archives {
                ctx.set_current(format!("Removing {} from the archive", leaf(e)));
                v.remove(a, std::slice::from_ref(e)).map_err(io)?;
            }
        }
        Ok(created)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

// ── Delete, rename, create ──────────────────────────────────────────

pub fn delete_selection(w: &Window, pane: &Rc<Pane>) {
    let Location::Archive(a) = pane.location() else { return };
    let entries: Vec<String> = pane.selected_items().iter().filter_map(|i| match i {
        Item::Archive(n) => Some(n.path.clone()),
        _ => None,
    }).collect();
    if entries.is_empty() {
        return;
    }
    let weak = Rc::downgrade(&w.me());
    let a2 = a.clone();
    w.app.jobs.start(
        w.win.upcast_ref(),
        format!("Deleting {} item{} from the archive", entries.len(), if entries.len() == 1 { "" } else { "s" }),
        move |_| Vfs::global().remove(&a2, &entries).map(|_| Vec::new()).map_err(io),
        move |_| {
            if let Some(w) = weak.upgrade() {
                refresh_archive_tabs(&w, &a);
            }
        },
    );
}

pub fn rename(w: &Window, pane: &Rc<Pane>, item: &Item, new: &str) {
    let (Location::Archive(a), Item::Archive(n)) = (pane.location(), item) else { return };
    let from = n.path.clone();
    let to = a.entry_path(new);
    if pane.has_name(new) {
        util::show_error(&w.win, "Cannot rename", &format!("“{new}” already exists."));
        return;
    }
    let weak = Rc::downgrade(&w.me());
    let name = new.to_string();
    let a2 = a.clone();
    w.app.jobs.start(
        w.win.upcast_ref(),
        format!("Renaming “{}”", n.name),
        move |_| Vfs::global().rename(&a2, &from, &to).map(|_| Vec::new()).map_err(io),
        move |outcome| {
            if let Some(w) = weak.upgrade() {
                refresh_archive_tabs(&w, &a);
                if matches!(outcome, Outcome::Done(_)) {
                    w.current_pane().select_when_present(vec![name.clone().into()]);
                }
            }
        },
    );
}

pub fn create(w: &Window, loc: &Location, name: &str, folder: bool) {
    let Location::Archive(a) = loc.clone() else { return };
    let path = a.entry_path(name);
    let weak = Rc::downgrade(&w.me());
    let name2 = name.to_string();
    let a2 = a.clone();
    w.app.jobs.start(
        w.win.upcast_ref(),
        format!("Creating “{name}” in the archive"),
        move |_| {
            let v = Vfs::global();
            if folder {
                v.mkdir(&a2, &path).map_err(io)?;
            } else {
                let dir = v.scratch_dir("new").map_err(io)?;
                let file = dir.join("empty");
                std::fs::write(&file, b"")?;
                let r = v.add(&a2, &[(file, path)]).map_err(io);
                let _ = std::fs::remove_dir_all(&dir);
                r?;
            }
            Ok(Vec::new())
        },
        move |outcome| {
            if let Some(w) = weak.upgrade() {
                refresh_archive_tabs(&w, &a);
                if matches!(outcome, Outcome::Done(_)) {
                    w.current_pane().select_when_present(vec![name2.clone().into()]);
                }
            }
        },
    );
}

/// Total sizes of the folders shown in an archive (known from the listing, no scanning).
pub fn fill_sizes(pane: &Rc<Pane>) {
    let Location::Archive(a) = pane.location() else { return };
    let Some(archive) = Vfs::global().cached(&a) else { return };
    let sizes: Vec<(std::ffi::OsString, u64)> = pane
        .dir_names()
        .into_iter()
        .map(|name| {
            let total = archive.tree.total_size(&a.entry_path(&name.to_string_lossy()));
            (name, total)
        })
        .collect();
    pane.set_sizes(&sizes);
}

// ── Extract, compress, open as archive ──────────────────────────────

/// Archive files among the selection (local files only).
pub fn selected_archives(pane: &Pane) -> Vec<PathBuf> {
    pane.selected_items().iter().filter_map(|i| i.path().filter(|_| !i.is_dir_like() && is_browsable_name(&i.display_name())).cloned()).collect()
}

pub fn extract_selected(w: &Window, dest: Option<PathBuf>) {
    let pane = w.current_pane();
    let archives = selected_archives(&pane);
    let Some(dest) = dest.or_else(|| pane.location().local_path().map(Path::to_path_buf)) else { return };
    if archives.is_empty() {
        return;
    }
    let weak = Rc::downgrade(&w.me());
    let dest_loc = Location::Dir(dest.clone());
    w.app.jobs.start(
        w.win.upcast_ref(),
        format!("Extracting {} archive{}", archives.len(), if archives.len() == 1 { "" } else { "s" }),
        move |ctx| {
            let mut made = Vec::new();
            for a in &archives {
                ctx.set_current(a.file_name().unwrap_or_default().to_string_lossy());
                made.push(Vfs::global().extract_all(&ArchiveLoc::root(a.clone()), &dest).map_err(io)?);
            }
            Ok(made)
        },
        move |outcome| {
            if let (Some(w), Outcome::Done(made)) = (weak.upgrade(), outcome) {
                fileops::select_created(&w, &dest_loc, made);
            }
        },
    );
}

pub fn extract_to(w: &Window) {
    let chooser = gtk::FileChooserNative::new(Some("Extract To"), Some(&w.win), gtk::FileChooserAction::SelectFolder, Some("_Extract"), Some("_Cancel"));
    if let Some(dir) = w.current_location().nearest_dir() {
        let _ = chooser.set_current_folder(dir);
    }
    let weak = Rc::downgrade(&w.me());
    chooser.connect_response(move |c, resp| {
        if resp == gtk::ResponseType::Accept {
            if let (Some(w), Some(dir)) = (weak.upgrade(), c.filename()) {
                extract_selected(&w, Some(dir));
            }
        }
    });
    chooser.show();
}

pub fn compress(w: &Window) {
    let pane = w.current_pane();
    let Location::Dir(dir) = pane.location() else { return };
    let items: Vec<PathBuf> = pane.selected_items().iter().filter_map(|i| i.path().cloned()).collect();
    if items.is_empty() {
        return;
    }
    let base = if items.len() == 1 {
        items[0].file_name().unwrap_or_default().to_string_lossy().into_owned()
    } else {
        dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into())
    };
    let d = gtk::Dialog::with_buttons(Some("Compress"), Some(&w.win), gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT, &[("_Cancel", gtk::ResponseType::Cancel), ("C_reate", gtk::ResponseType::Ok)]);
    d.set_default_response(gtk::ResponseType::Ok);
    let area = d.content_area();
    area.set_spacing(6);
    area.set_border_width(12);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(8);
    grid.set_row_spacing(6);
    let name_label = gtk::Label::with_mnemonic("Archive _name:");
    name_label.set_xalign(1.0);
    let entry = gtk::Entry::new();
    entry.set_text(&base);
    entry.set_activates_default(true);
    entry.set_width_chars(32);
    name_label.set_mnemonic_widget(Some(&entry));
    let fmt_label = gtk::Label::with_mnemonic("_Format:");
    fmt_label.set_xalign(1.0);
    let combo = gtk::ComboBoxText::new();
    for (label, ext) in CREATE_FORMATS {
        combo.append(Some(ext), &format!("{label} ({ext})"));
    }
    combo.set_active(Some(0));
    fmt_label.set_mnemonic_widget(Some(&combo));
    grid.attach(&name_label, 0, 0, 1, 1);
    grid.attach(&entry, 1, 0, 1, 1);
    grid.attach(&fmt_label, 0, 1, 1, 1);
    grid.attach(&combo, 1, 1, 1, 1);
    area.add(&grid);
    d.show_all();
    let weak = Rc::downgrade(&w.me());
    let reply = super::selftest::scripted_text();
    let run = move |name: String, ext: String| {
        let Some(w) = weak.upgrade() else { return };
        if !util::valid_file_name(&name) {
            util::show_error(&w.win, "Invalid name", "A name cannot be empty or contain “/”.");
            return;
        }
        let file = format!("{name}{ext}");
        let target = dir.join(&file);
        if target.exists() {
            util::show_error(&w.win, "Cannot create archive", &format!("“{file}” already exists."));
            return;
        }
        let items: Vec<(PathBuf, String)> = items.iter().map(|p| (p.clone(), p.file_name().unwrap_or_default().to_string_lossy().into_owned())).collect();
        let loc = Location::Dir(dir.clone());
        let w2 = Rc::downgrade(&w);
        w.app.jobs.start(
            w.win.upcast_ref(),
            format!("Creating “{file}”"),
            move |_| Vfs::global().helper().add(&target, &items, None).map(|_| vec![target.clone()]).map_err(io),
            move |outcome| {
                if let (Some(w), Outcome::Done(made)) = (w2.upgrade(), outcome) {
                    fileops::select_created(&w, &loc, made);
                }
            },
        );
    };
    if let Some(r) = reply {
        // Self-test: "name.ext" picks the format by its extension.
        let ext = CREATE_FORMATS.iter().map(|(_, e)| *e).find(|e| r.ends_with(e)).unwrap_or(".zip");
        run(r.trim_end_matches(ext).to_string(), ext.to_string());
        d.close();
        return;
    }
    d.connect_response(move |d, resp| {
        if resp == gtk::ResponseType::Ok {
            run(entry.text().to_string(), combo.active_id().map(|s| s.to_string()).unwrap_or_else(|| ".zip".into()));
        }
        d.close();
    });
}

/// Opens any file as a folder if the archive library understands it.
pub fn open_as_archive(w: &Window) {
    let pane = w.current_pane();
    let Some(item) = pane.selected_items().into_iter().next() else { return };
    let loc = pane.location();
    let target = match (&loc, &item) {
        (_, Item::Fs(e)) if !e.is_dir_like() => Location::Archive(ArchiveLoc::root(e.path.clone())),
        (Location::Archive(a), Item::Archive(n)) if !n.is_dir => Location::Archive(a.enter_nested(&n.path)),
        _ => return,
    };
    let probe_path = item.path().cloned();
    let me = Rc::downgrade(&w.me());
    glib::spawn_future_local(async move {
        // Plain files are probed first so a non-archive gives a clear message.
        if let Some(p) = probe_path {
            let res = gio::spawn_blocking(move || Vfs::global().helper().probe(&p)).await;
            if !matches!(res, Ok(Ok((true, _, _)))) {
                if let Some(w) = me.upgrade() {
                    util::show_error(&w.win, "Not an archive", "This file cannot be opened as a folder.");
                }
                return;
            }
        }
        if let Some(w) = me.upgrade() {
            w.navigate(target);
        }
    });
}

pub fn ask_password(w: &Window, pane: &Rc<Pane>, loc: &Location) {
    let Location::Archive(a) = loc else { return };
    let file = a.file.clone();
    let p = Rc::downgrade(pane);
    util::ask_secret(&w.win, "Encrypted Archive", &format!("Password for “{}”:", loc.title()), move |pw| {
        Vfs::global().set_password(&file, &pw);
        if let Some(p) = p.upgrade() {
            p.reload();
        }
    });
}
