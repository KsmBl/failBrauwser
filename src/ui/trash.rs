//! The trash view's operations: restore, delete permanently, empty. The trash itself is the
//! freedesktop.org one, shared with Thunar and other file managers.

use super::jobs::Outcome;
use super::model::Item;
use super::util;
use super::window::Window;
use failbrauwser::location::Location;
use failbrauwser::ops::fastcopy;
use failbrauwser::ops::job::{Answer, Question};
use failbrauwser::ops::names::free_name;
use failbrauwser::trash::{self, TrashItem};
use gtk::prelude::*;
use std::os::unix::fs::MetadataExt;
use std::rc::Rc;

fn selected(w: &Window) -> Vec<TrashItem> {
    w.current_pane()
        .selected_items()
        .into_iter()
        .filter_map(|i| match i {
            Item::Trash(t, _) => Some(t),
            _ => None,
        })
        .collect()
}

pub fn install_actions(w: &Window) {
    w.add_action("restore", restore_selection);
    w.add_action("empty-trash", empty);
}

pub fn update_sensitivity(w: &Window, set: impl Fn(&str, bool)) {
    let in_trash = w.current_location() == Location::Trash;
    set("restore", in_trash && !selected(w).is_empty());
    set("empty-trash", !trash::is_empty(&trash::trash_dirs()));
}

pub fn restore_selection(w: &Window) {
    let items = selected(w);
    if items.is_empty() {
        return;
    }
    let n = items.len();
    let weak = Rc::downgrade(&w.me());
    w.app.jobs.start(
        w.win.upcast_ref(),
        format!("Restoring {n} item{}", if n == 1 { "" } else { "s" }),
        move |ctx| {
            let mut done = Vec::new();
            for item in &items {
                if ctx.is_cancelled() {
                    return Err(fastcopy::cancelled());
                }
                ctx.set_current(item.original_name());
                match trash::restore(item, None) {
                    Ok(p) => done.push(p),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        let there = std::fs::symlink_metadata(&item.original).ok();
                        let here = std::fs::symlink_metadata(&item.path).ok();
                        let q = Question::Conflict {
                            src: item.path.clone(),
                            dst: item.original.clone(),
                            src_size: here.as_ref().map(|m| m.len()).unwrap_or(0),
                            dst_size: there.as_ref().map(|m| m.len()).unwrap_or(0),
                            src_mtime: here.as_ref().map(|m| m.mtime()).unwrap_or(0),
                            dst_mtime: there.as_ref().map(|m| m.mtime()).unwrap_or(0),
                            dst_is_dir: there.is_some_and(|m| m.is_dir()),
                        };
                        let parent = item.original.parent().unwrap_or(std::path::Path::new("/")).to_path_buf();
                        let name = item.original.file_name().unwrap_or_default().to_os_string();
                        match ctx.ask(q) {
                            Answer::KeepBoth => done.push(trash::restore(item, Some(&parent.join(free_name(&parent, &name, None))))?),
                            Answer::Replace => {
                                // What is in the way goes to the trash instead of being lost.
                                if let Err(e) = gtk::gio::File::for_path(&item.original).trash(gtk::gio::Cancellable::NONE) {
                                    ctx.record_error(format!("{}: {e}", item.original.display()));
                                    continue;
                                }
                                done.push(trash::restore(item, None)?);
                            }
                            Answer::Cancel => return Err(fastcopy::cancelled()),
                            _ => {}
                        }
                    }
                    Err(e) => ctx.record_error(format!("{}: {e}", item.original_name())),
                }
            }
            Ok(done)
        },
        move |outcome| {
            if let (Some(w), Outcome::Done(paths)) = (weak.upgrade(), outcome) {
                let n = paths.len();
                w.set_status(&format!("Restored {n} item{}.", if n == 1 { "" } else { "s" }));
            }
        },
    );
}

pub fn delete_selection(w: &Window) {
    let items = selected(w);
    if items.is_empty() {
        return;
    }
    let n = items.len();
    w.app.jobs.start(
        w.win.upcast_ref(),
        format!("Deleting {n} item{} for good", if n == 1 { "" } else { "s" }),
        move |ctx| {
            for item in &items {
                ctx.set_current(item.original_name());
                if let Err(e) = trash::delete(item) {
                    ctx.record_error(format!("{}: {e}", item.original_name()));
                }
            }
            Ok(Vec::new())
        },
        |_| {},
    );
}

pub fn empty(w: &Window) {
    let weak = Rc::downgrade(&w.me());
    util::confirm(&w.win, "Empty the trash?", "All items in the trash will be deleted permanently.", "_Empty Trash", move || {
        let Some(w) = weak.upgrade() else { return };
        w.app.jobs.start(w.win.upcast_ref(), "Emptying the trash", |_| trash::empty(&trash::trash_dirs()).map(|_| Vec::new()), |_| {});
    });
}

/// "“report.pdf” (1.2 MB), deleted from /home/me/Documents on 2026-09-29 10:15".
pub fn status_text(t: &TrashItem, size: u64, dir: bool) -> String {
    let from = t.original.parent().map(|p| p.display().to_string()).unwrap_or_default();
    let when = failbrauwser::fs::format::human_time(t.deleted_unix(), gtk::glib::real_time() / 1_000_000);
    let size = if dir { String::new() } else { format!(" ({})", failbrauwser::fs::format::human_size(size)) };
    format!("“{}”{size}, deleted from {from} {when}", t.original_name())
}
