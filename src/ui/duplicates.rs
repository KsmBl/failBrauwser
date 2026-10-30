//! Tools > Find Duplicates: files with the same bytes, and pictures that show the same thing
//! (at another resolution, saved again), reviewed group by group side by side, with size and
//! resolution under each, choosing which to keep; the others go to the trash (Ctrl+Z brings
//! them back).

use super::jobs::Outcome;
use super::window::Window;
use failbrauwser::fs::duplicates::{self, Found};
use failbrauwser::fs::format::{human_size, human_time};
use failbrauwser::ops::undo::{Step, Undoable};
use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// One file of a group.
#[derive(Debug, Clone)]
pub struct Item {
    pub file: Found,
    /// Width and height, for pictures.
    pub resolution: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct Group {
    /// Pictures that look the same (else: the same bytes).
    pub similar: bool,
    pub items: Vec<Item>,
}

pub struct Duplicates {
    pub window: gtk::Window,
    folder: gtk::FileChooserButton,
    pictures: gtk::CheckButton,
    search: gtk::Button,
    spinner: gtk::Spinner,
    status: gtk::Label,
    heading: gtk::Label,
    cards: gtk::Box,
    prev: gtk::Button,
    skip: gtk::Button,
    delete: gtk::Button,
    groups: RefCell<Vec<Group>>,
    current: Cell<usize>,
    /// Per item of the group shown: kept?
    keep: RefCell<Vec<gtk::CheckButton>>,
    busy: Cell<bool>,
    cancel: RefCell<Arc<AtomicBool>>,
    main: Weak<Window>,
    weak: RefCell<Weak<Duplicates>>,
}

thread_local! {
    static OPEN: RefCell<Vec<Rc<Duplicates>>> = const { RefCell::new(Vec::new()) };
}

/// The duplicates window shown (tests).
pub fn current() -> Option<Rc<Duplicates>> {
    OPEN.with(|o| o.borrow().iter().rev().find(|c| c.window.is_visible()).cloned())
}

pub fn show(w: &Window, folder: Option<PathBuf>) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Find Duplicates");
    window.set_transient_for(Some(&w.win));
    window.set_default_size(900, 620);
    let chooser = gtk::FileChooserButton::new("Folder to search", gtk::FileChooserAction::SelectFolder);
    if let Some(f) = &folder {
        chooser.set_filename(f);
    }
    chooser.set_hexpand(true);
    let pictures = gtk::CheckButton::with_mnemonic("Compare _pictures by what they show");
    pictures.set_active(true);
    pictures.set_tooltip_text(Some("Also finds the same picture at another resolution or saved again, not by its metadata but by its content"));
    let search = gtk::Button::with_mnemonic("_Search");
    let spinner = gtk::Spinner::new();
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    top.pack_start(&gtk::Label::new(Some("Folder:")), false, false, 0);
    top.pack_start(&chooser, true, true, 0);
    top.pack_start(&pictures, false, false, 0);
    top.pack_start(&spinner, false, false, 0);
    top.pack_start(&search, false, false, 0);
    let status = gtk::Label::new(Some("Searches the folder and all folders in it."));
    status.set_xalign(0.0);
    let heading = gtk::Label::new(None);
    heading.set_xalign(0.0);
    let attrs = gtk::pango::AttrList::new();
    attrs.insert(gtk::pango::AttrFloat::new_scale(1.15));
    attrs.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold));
    heading.set_attributes(Some(&attrs));
    let cards = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    cards.set_homogeneous(true);
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.add(&cards);
    let prev = gtk::Button::with_mnemonic("◀ _Previous");
    let skip = gtk::Button::with_mnemonic("S_kip ▶");
    let delete = gtk::Button::with_mnemonic("_Delete Unticked");
    delete.style_context().add_class("destructive-action");
    delete.set_tooltip_text(Some("Moves the files not ticked to the trash and goes to the next group"));
    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    nav.pack_start(&prev, false, false, 0);
    nav.pack_end(&delete, false, false, 0);
    nav.pack_end(&skip, false, false, 0);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_border_width(12);
    root.pack_start(&top, false, false, 0);
    root.pack_start(&status, false, false, 0);
    root.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 0);
    root.pack_start(&heading, false, false, 0);
    root.pack_start(&scroll, true, true, 0);
    root.pack_start(&nav, false, false, 0);
    window.add(&root);

    let d = Rc::new(Duplicates {
        window: window.clone(),
        folder: chooser,
        pictures,
        search,
        spinner,
        status,
        heading,
        cards,
        prev,
        skip,
        delete,
        groups: RefCell::new(Vec::new()),
        current: Cell::new(0),
        keep: RefCell::new(Vec::new()),
        busy: Cell::new(false),
        cancel: RefCell::new(Arc::new(AtomicBool::new(false))),
        main: Rc::downgrade(&w.me()),
        weak: RefCell::new(Weak::new()),
    });
    *d.weak.borrow_mut() = Rc::downgrade(&d);
    let weak = Rc::downgrade(&d);
    d.search.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.run();
        }
    });
    let weak = Rc::downgrade(&d);
    d.prev.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.go(d.current.get().saturating_sub(1));
        }
    });
    let weak = Rc::downgrade(&d);
    d.skip.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.go(d.current.get() + 1);
        }
    });
    let weak = Rc::downgrade(&d);
    d.delete.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.apply();
        }
    });
    let weak = Rc::downgrade(&d);
    window.connect_destroy(move |_| {
        if let Some(d) = weak.upgrade() {
            d.cancel.borrow().store(true, Ordering::Relaxed);
            OPEN.with(|o| o.borrow_mut().retain(|x| !Rc::ptr_eq(x, &d)));
        }
    });
    OPEN.with(|o| o.borrow_mut().push(d.clone()));
    window.show_all();
    d.go(0);
    if folder.is_some() {
        d.run();
    }
}

impl Duplicates {
    fn me(&self) -> Rc<Duplicates> {
        self.weak.borrow().upgrade().expect("duplicates window alive")
    }

    pub fn is_busy(&self) -> bool {
        self.busy.get()
    }

    /// Searches the folder on worker threads, telling how far it got.
    pub fn run(&self) {
        let Some(root) = self.folder.filename() else {
            self.status.set_text("Choose a folder.");
            return;
        };
        self.cancel.borrow().store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.borrow_mut() = cancel.clone();
        let pictures = self.pictures.is_active();
        let (stage, done, total) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        self.busy.set(true);
        self.spinner.start();
        self.search.set_sensitive(false);
        self.groups.borrow_mut().clear();
        self.go(0);
        // What the worker is at, every few tenths of a second.
        let me = self.me();
        let (s2, d2, t2) = (stage.clone(), done.clone(), total.clone());
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            if !me.busy.get() {
                return glib::ControlFlow::Break;
            }
            let (s, d, t) = (s2.load(Ordering::Relaxed), d2.load(Ordering::Relaxed), t2.load(Ordering::Relaxed));
            me.status.set_text(&match s {
                0 => "Looking through the folders…".to_string(),
                1 => format!("Comparing files of the same size… {d} of {t}"),
                _ => format!("Looking at pictures… {d} of {t}"),
            });
            glib::ControlFlow::Continue
        });
        let me = self.me();
        glib::spawn_future_local(async move {
            let res = gio::spawn_blocking(move || find(&root, pictures, &cancel, &stage, &done, &total)).await;
            me.busy.set(false);
            me.spinner.stop();
            me.search.set_sensitive(true);
            let Ok(Some(groups)) = res else { return };
            let wasted: u64 = groups.iter().map(|g| g.items.iter().map(|i| i.file.size).sum::<u64>() - g.items.iter().map(|i| i.file.size).max().unwrap_or(0)).sum();
            let (exact, similar) = (groups.iter().filter(|g| !g.similar).count(), groups.iter().filter(|g| g.similar).count());
            me.status.set_text(&if groups.is_empty() {
                "No duplicates found.".to_string()
            } else {
                let n = |k: usize, one: &str, many: &str| if k == 1 { format!("1 {one}") } else { format!("{k} {many}") };
                format!(
                    "{}, {} — up to {} to free.",
                    n(exact, "group of identical files", "groups of identical files"),
                    n(similar, "group of pictures that look the same", "groups of pictures that look the same"),
                    human_size(wasted)
                )
            });
            *me.groups.borrow_mut() = groups;
            me.go(0);
        });
    }

    /// Shows group `i` (past the last: says all were looked at).
    fn go(&self, i: usize) {
        for c in self.cards.children() {
            self.cards.remove(&c);
        }
        self.keep.borrow_mut().clear();
        let groups = self.groups.borrow();
        self.current.set(i.min(groups.len()));
        let Some(g) = groups.get(i) else {
            self.heading.set_text(if groups.is_empty() { "" } else { "All groups looked at." });
            for b in [&self.prev, &self.skip, &self.delete] {
                b.set_sensitive(!groups.is_empty() && b == &self.prev);
            }
            return;
        };
        let what = if g.similar { "Pictures that look the same" } else { "Identical files" };
        self.heading.set_text(&format!("{what} — group {} of {}", i + 1, groups.len()));
        // Kept by default: the biggest picture, else the oldest file.
        let best = if g.similar {
            (0..g.items.len()).max_by_key(|&k| (g.items[k].resolution.map_or(0, |(w, h)| w as u64 * h as u64), g.items[k].file.size)).unwrap_or(0)
        } else {
            (0..g.items.len()).min_by_key(|&k| (g.items[k].file.mtime, g.items[k].file.path.clone())).unwrap_or(0)
        };
        let now = glib::real_time() / 1_000_000;
        for (k, item) in g.items.iter().enumerate() {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
            card.set_valign(gtk::Align::Start);
            let image = gtk::Image::new();
            image.set_size_request(320, 240);
            if item.resolution.is_some() {
                load_preview(&image, item.file.path.clone());
            } else {
                image.set_from_icon_name(Some("text-x-generic"), gtk::IconSize::Dialog);
            }
            let name = gtk::Label::new(Some(&item.file.path.file_name().unwrap_or_default().to_string_lossy()));
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name.set_attributes(Some(&{
                let a = gtk::pango::AttrList::new();
                a.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold));
                a
            }));
            let folder = gtk::Label::new(Some(&item.file.path.parent().map(|p| p.display().to_string()).unwrap_or_default()));
            folder.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            folder.style_context().add_class("dim-label");
            let size = gtk::Label::new(Some(&match item.resolution {
                Some((w, h)) => format!("{w} × {h} pixels — {}", human_size(item.file.size)),
                None => human_size(item.file.size),
            }));
            let date = gtk::Label::new(Some(&format!("Modified {}", human_time(item.file.mtime, now))));
            date.style_context().add_class("dim-label");
            let keep = gtk::CheckButton::with_label("Keep");
            keep.set_halign(gtk::Align::Center);
            keep.set_active(k == best);
            let me = self.me();
            keep.connect_toggled(move |_| me.update_delete());
            for wdg in [image.upcast_ref::<gtk::Widget>(), name.upcast_ref(), folder.upcast_ref(), size.upcast_ref(), date.upcast_ref(), keep.upcast_ref()] {
                card.pack_start(wdg, false, false, 0);
            }
            self.cards.pack_start(&card, true, true, 0);
            self.keep.borrow_mut().push(keep);
        }
        self.cards.show_all();
        self.prev.set_sensitive(i > 0);
        self.skip.set_sensitive(true);
        drop(groups);
        self.update_delete();
    }

    /// Deleting needs at least one kept and one not.
    fn update_delete(&self) {
        let keep = self.keep.borrow();
        let kept = keep.iter().filter(|k| k.is_active()).count();
        self.delete.set_sensitive(kept > 0 && kept < keep.len());
    }

    /// Moves the unticked files of the group shown to the trash, then shows the next group.
    fn apply(&self) {
        let i = self.current.get();
        let Some(g) = self.groups.borrow().get(i).cloned() else { return };
        let gone: Vec<PathBuf> = g.items.iter().zip(self.keep.borrow().iter()).filter(|(_, k)| !k.is_active()).map(|(it, _)| it.file.path.clone()).collect();
        if gone.is_empty() {
            return;
        }
        let Some(w) = self.main.upgrade() else { return };
        let me = self.me();
        let paths = gone.clone();
        w.app.jobs.start(
            w.win.upcast_ref(),
            format!("Moving {} duplicate{} to the trash", gone.len(), if gone.len() == 1 { "" } else { "s" }),
            move |ctx| failbrauwser::ops::delete::trash(ctx, &paths).map(|_| Vec::new()),
            move |outcome| {
                let gone: Vec<PathBuf> = gone.iter().filter(|p| p.symlink_metadata().is_err()).cloned().collect();
                if let Some(w) = me.main.upgrade() {
                    if !gone.is_empty() {
                        w.app.remember(Undoable { what: format!("Move {} duplicate{} to the trash", gone.len(), if gone.len() == 1 { "" } else { "s" }), step: Step::Trashed(gone.clone()) });
                    }
                }
                // The group keeps what is left; with one file left it is no group any more.
                {
                    let mut groups = me.groups.borrow_mut();
                    if let Some(g) = groups.get_mut(i) {
                        g.items.retain(|it| !gone.contains(&it.file.path));
                        if g.items.len() < 2 {
                            groups.remove(i);
                        }
                    }
                }
                if matches!(outcome, Outcome::Done(_)) {
                    let next = if me.groups.borrow().get(i).is_some_and(|g| g.items.len() >= 2) { i + 1 } else { i };
                    me.go(next);
                } else {
                    me.go(i);
                }
            },
        );
    }

    /// The names in the group shown, the kept ones marked with `*` (tests).
    pub fn shown(&self) -> Vec<String> {
        let groups = self.groups.borrow();
        let Some(g) = groups.get(self.current.get()) else { return Vec::new() };
        g.items
            .iter()
            .zip(self.keep.borrow().iter())
            .map(|(it, k)| format!("{}{}", if k.is_active() { "*" } else { "" }, it.file.path.file_name().unwrap_or_default().to_string_lossy()))
            .collect()
    }

    pub fn group_count(&self) -> usize {
        self.groups.borrow().len()
    }

    /// Keeps only `name` in the group shown (tests).
    pub fn keep_only(&self, name: &str) -> Result<(), String> {
        let groups = self.groups.borrow();
        let g = groups.get(self.current.get()).ok_or("no group shown")?;
        let i = g.items.iter().position(|it| it.file.path.file_name().is_some_and(|n| n == name)).ok_or("no such file in the group")?;
        let keep = self.keep.borrow().clone();
        drop(groups);
        for (k, b) in keep.iter().enumerate() {
            b.set_active(k == i);
        }
        Ok(())
    }

    pub fn click(&self, what: &str) {
        match what {
            "delete" => self.apply(),
            "skip" => self.go(self.current.get() + 1),
            _ => self.go(self.current.get().saturating_sub(1)),
        }
    }
}

/// Loads a picture into `image`, fitted to its box, on a worker thread.
fn load_preview(image: &gtk::Image, path: PathBuf) {
    image.set_from_icon_name(Some("image-loading"), gtk::IconSize::Dialog);
    let weak = image.downgrade();
    glib::spawn_future_local(async move {
        let bytes = gio::spawn_blocking(move || {
            let pb = Pixbuf::from_file_at_scale(&path, 320, 240, true).ok()?;
            let pb = pb.apply_embedded_orientation().unwrap_or(pb);
            Some((pb.read_pixel_bytes(), pb.has_alpha(), pb.width(), pb.height(), pb.rowstride()))
        })
        .await
        .ok()
        .flatten();
        let Some(image) = weak.upgrade() else { return };
        match bytes {
            Some((b, alpha, w, h, stride)) => image.set_from_pixbuf(Some(&Pixbuf::from_bytes(&b, gtk::gdk_pixbuf::Colorspace::Rgb, alpha, 8, w, h, stride))),
            None => image.set_from_icon_name(Some("image-missing"), gtk::IconSize::Dialog),
        }
    });
}

/// The search itself (worker thread): identical files first, then pictures that look the
/// same (pictures identical to the byte are already among the first).
fn find(root: &std::path::Path, pictures: bool, cancel: &AtomicBool, stage: &AtomicUsize, done: &AtomicUsize, total: &AtomicUsize) -> Option<Vec<Group>> {
    let files = duplicates::walk(root, cancel);
    stage.store(1, Ordering::Relaxed);
    total.store(files.len(), Ordering::Relaxed);
    let exact = duplicates::exact_groups(&files, cancel, done);
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let resolution = |p: &std::path::Path| Pixbuf::file_info(p).map(|(_, w, h)| (w as u32, h as u32));
    let mut groups: Vec<Group> = exact
        .into_iter()
        .map(|g| Group {
            similar: false,
            items: g.into_iter().map(|f| Item { resolution: duplicates::is_picture_name(&f.path.to_string_lossy()).then(|| resolution(&f.path)).flatten(), file: f }).collect(),
        })
        .collect();
    if pictures {
        // One of each set of identical pictures is enough to compare.
        let twins: std::collections::HashSet<PathBuf> = groups.iter().flat_map(|g| g.items.iter().skip(1).map(|i| i.file.path.clone())).collect();
        let pics: Vec<&Found> = files.iter().filter(|f| duplicates::is_picture_name(&f.path.to_string_lossy()) && !twins.contains(&f.path)).collect();
        stage.store(2, Ordering::Relaxed);
        done.store(0, Ordering::Relaxed);
        total.store(pics.len(), Ordering::Relaxed);
        let paths: Vec<PathBuf> = pics.iter().map(|f| f.path.clone()).collect();
        let prints = duplicates::fingerprints(&paths, cancel, done);
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let (idx, fps): (Vec<usize>, Vec<duplicates::Fingerprint>) = prints.into_iter().enumerate().filter_map(|(i, f)| f.map(|f| (i, f))).unzip();
        for g in duplicates::similar_groups(&fps) {
            let mut items: Vec<Item> = g.iter().map(|&k| Item { file: pics[idx[k]].clone(), resolution: Some((fps[k].width, fps[k].height)) }).collect();
            items.sort_by(|a, b| a.file.path.cmp(&b.file.path));
            // All the same bytes: already an identical group.
            if items.iter().all(|it| it.file.size == items[0].file.size) && groups.iter().any(|eg| eg.items.iter().any(|x| x.file.path == items[0].file.path)) {
                continue;
            }
            groups.push(Group { similar: true, items });
        }
    }
    Some(groups)
}
