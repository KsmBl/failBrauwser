//! Two versions of a file side by side, as TortoiseGitMerge shows them: the old one on the
//! left, the new one on the right, line numbers in front, removed lines red, added lines
//! yellow, gaps grey; buttons to jump to the previous or next difference.

use super::super::window::Window;
use super::dialog;
use failbrauwser::git::{self, DiffKind, DiffRow, Repo};
use gtk::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

const REMOVED: &str = "#ffc8c8";
const ADDED: &str = "#ffff96";
const GAP: &str = "#e2e2e2";

thread_local! {
    /// The diff windows shown and their rows (tests).
    static SHOWN: RefCell<Vec<(glib::WeakRef<gtk::Window>, Vec<DiffRow>)>> = const { RefCell::new(Vec::new()) };
}

/// The working tree file against HEAD.
pub fn working(w: &Window, repo: &Repo, path: &str) {
    let old = repo.show_file("HEAD", path);
    let new = std::fs::read(repo.root.join(path)).unwrap_or_default();
    open(w, repo, path, &format!("{path} - Base (HEAD)"), &format!("{path} - Working Tree"), old, new);
}

/// The file as its last commit left it against the version before.
pub fn previous(w: &Window, repo: &Repo, path: &str) {
    let Some((latest, before)) = repo.last_two_versions(path) else {
        dialog::error(&w.win, &format!("{path} has no history."));
        return;
    };
    revisions(w, repo, path, path, &before, &latest);
}

/// `path` at `new_rev` against `old_path` at `old_rev`.
pub fn revisions(w: &Window, repo: &Repo, old_path: &str, path: &str, old_rev: &str, new_rev: &str) {
    let short = |r: &str| if r == git::EMPTY_TREE { "(none)".to_string() } else { r[..r.len().min(8)].to_string() };
    let old = if old_rev == git::EMPTY_TREE { Vec::new() } else { repo.show_file(old_rev, old_path) };
    let new = repo.show_file(new_rev, path);
    open(w, repo, path, &format!("{old_path} - Revision {}", short(old_rev)), &format!("{path} - Revision {}", short(new_rev)), old, new);
}

fn open(w: &Window, repo: &Repo, path: &str, left: &str, right: &str, old: Vec<u8>, new: Vec<u8>) {
    let win = gtk::Window::new(gtk::WindowType::Toplevel);
    win.set_title(&format!("{} - TortoiseGitMerge", repo.root.join(path).display()));
    win.set_transient_for(Some(&w.win));
    win.set_destroy_with_parent(true);
    win.set_default_size(1000, 640);
    if let Some(pb) = super::icons::pixbuf("diff", 32) {
        win.set_icon(Some(&pb));
    }
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    // Toolbar: previous / next difference.
    let bar = gtk::Toolbar::new();
    let prev = gtk::ToolButton::new(Some(&gtk::Image::from_icon_name(Some("go-up"), gtk::IconSize::SmallToolbar)), Some("Previous Difference"));
    let next = gtk::ToolButton::new(Some(&gtk::Image::from_icon_name(Some("go-down"), gtk::IconSize::SmallToolbar)), Some("Next Difference"));
    WidgetExt::set_tooltip_text(&prev, Some("Previous Difference"));
    WidgetExt::set_tooltip_text(&next, Some("Next Difference"));
    prev.set_is_important(true);
    next.set_is_important(true);
    bar.insert(&prev, -1);
    bar.insert(&next, -1);
    bar.set_style(gtk::ToolbarStyle::BothHoriz);
    outer.pack_start(&bar, false, false, 0);
    let binary = old.contains(&0) || new.contains(&0);
    let rows = if binary { Vec::new() } else { git::side_by_side(&String::from_utf8_lossy(&old), &String::from_utf8_lossy(&new)) };
    // Pane titles.
    let heads = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    heads.set_homogeneous(true);
    for t in [left, right] {
        let l = gtk::Label::new(None);
        l.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(t)));
        l.set_xalign(0.0);
        l.set_margin_start(6);
        l.set_margin_top(3);
        l.set_margin_bottom(3);
        l.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        heads.pack_start(&l, true, true, 0);
    }
    outer.pack_start(&heads, false, false, 0);
    let store = gtk::ListStore::new(&[glib::Type::STRING, glib::Type::STRING, glib::Type::STRING, glib::Type::STRING, glib::Type::STRING, glib::Type::STRING]);
    for r in &rows {
        let it = store.append();
        let (lbg, rbg) = match r.kind {
            DiffKind::Same => (None, None),
            DiffKind::Removed => (Some(REMOVED), Some(GAP)),
            DiffKind::Added => (Some(GAP), Some(ADDED)),
            DiffKind::Changed => (Some(REMOVED), Some(ADDED)),
        };
        let num = |s: &Option<(usize, String)>| s.as_ref().map(|(n, _)| n.to_string()).unwrap_or_default();
        let text = |s: &Option<(usize, String)>| s.as_ref().map(|(_, t)| t.replace('\t', "    ")).unwrap_or_default();
        store.set(&it, &[(0, &num(&r.left)), (1, &text(&r.left)), (2, &lbg), (3, &num(&r.right)), (4, &text(&r.right)), (5, &rbg)]);
    }
    let view = gtk::TreeView::with_model(&store);
    view.set_widget_name("diff");
    view.set_headers_visible(false);
    // The colors stay visible: jumping scrolls, it selects nothing.
    view.selection().set_mode(gtk::SelectionMode::None);
    view.set_enable_search(false);
    for (num, text, bg) in [(0, 1, 2), (3, 4, 5)] {
        let nc = gtk::TreeViewColumn::new();
        let nr = gtk::CellRendererText::new();
        nr.set_xalign(1.0);
        nr.set_property("family", "monospace");
        nr.set_property("foreground", "#808080");
        TreeViewColumnExt::pack_start(&nc, &nr, true);
        TreeViewColumnExt::add_attribute(&nc, &nr, "text", num);
        view.append_column(&nc);
        let tc = gtk::TreeViewColumn::new();
        tc.set_expand(true);
        tc.set_resizable(true);
        let tr = gtk::CellRendererText::new();
        tr.set_property("family", "monospace");
        TreeViewColumnExt::pack_start(&tc, &tr, true);
        TreeViewColumnExt::add_attribute(&tc, &tr, "text", text);
        TreeViewColumnExt::add_attribute(&tc, &tr, "cell-background", bg);
        // Dark text on the colored lines, whatever the theme.
        let tr2 = tr.clone();
        CellLayoutExt::set_cell_data_func(
            &tc,
            &tr,
            Some(Box::new(move |_, _, m, it| {
                let colored = m.value(it, bg).get::<Option<String>>().ok().flatten().is_some();
                if colored {
                    tr2.set_property("foreground", "#000000");
                } else {
                    tr2.set_property("foreground-set", false);
                }
            })),
        );
        view.append_column(&tc);
    }
    let sw = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    sw.add(&view);
    if binary {
        let l = gtk::Label::new(Some("Binary files differ: they cannot be shown side by side."));
        l.set_vexpand(true);
        outer.pack_start(&l, true, true, 0);
    } else {
        outer.pack_start(&sw, true, true, 0);
    }
    // Where each block of differences starts.
    let starts: Vec<usize> = rows.iter().enumerate().filter(|(i, r)| r.kind != DiffKind::Same && (*i == 0 || rows[i - 1].kind == DiffKind::Same)).map(|(i, _)| i).collect();
    let status = gtk::Statusbar::new();
    let lines = |side: &dyn Fn(&DiffRow) -> bool| rows.iter().filter(|r| side(r)).count();
    let summary = format!(
        "Left View: {} lines    Right View: {} lines    {} difference(s)",
        lines(&|r| r.left.is_some()),
        lines(&|r| r.right.is_some()),
        starts.len()
    );
    status.push(0, &summary);
    outer.pack_start(&status, false, false, 0);
    win.add(&outer);
    let starts = Rc::new(starts);
    let current = Rc::new(std::cell::Cell::new(None::<usize>));
    for (btn, dir) in [(&prev, -1i32), (&next, 1)] {
        let (view, starts, current) = (view.clone(), starts.clone(), current.clone());
        btn.connect_clicked(move |_| {
            if starts.is_empty() {
                return;
            }
            let i = match (current.get(), dir) {
                (None, _) => 0,
                (Some(i), 1) => (i + 1).min(starts.len() - 1),
                (Some(i), _) => i.saturating_sub(1),
            };
            current.set(Some(i));
            let p = gtk::TreePath::from_indicesv(&[starts[i] as i32]);
            view.scroll_to_cell(Some(&p), gtk::TreeViewColumn::NONE, true, 0.3, 0.0);
        });
    }
    SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|(w, _)| w.upgrade().is_some());
        s.push((win.downgrade(), rows));
    });
    win.show_all();
    // The first difference in view, as TortoiseGitMerge opens.
    if !starts.is_empty() {
        next.emit_clicked();
    }
}

/// The rows of the diff window whose title contains `title`, as "= a", "- a", "+ b", "~ a|b".
pub fn describe(title: &str) -> Option<Vec<String>> {
    SHOWN.with(|s| {
        s.borrow().iter().find_map(|(w, rows)| {
            let w = w.upgrade()?;
            (w.is_visible() && w.title().is_some_and(|t| t.contains(title))).then(|| {
                rows.iter()
                    .map(|r| {
                        let t = |s: &Option<(usize, String)>| s.as_ref().map(|x| x.1.clone()).unwrap_or_default();
                        match r.kind {
                            DiffKind::Same => format!("= {}", t(&r.left)),
                            DiffKind::Removed => format!("- {}", t(&r.left)),
                            DiffKind::Added => format!("+ {}", t(&r.right)),
                            DiffKind::Changed => format!("~ {}|{}", t(&r.left), t(&r.right)),
                        }
                    })
                    .collect()
            })
        })
    })
}
