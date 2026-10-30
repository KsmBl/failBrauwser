//! A failBrauwser window that picks files for another application, asked for through the
//! desktop portal: the usual side panel, path bar and views, and a bar at the bottom with the
//! name (when saving), the file types and the button that picks.

use super::app::AppCtx;
use super::model::Item;
use super::window::Window;
use failbrauwser::chooser::{Filter, Mode, Request};
use failbrauwser::location::Location;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

/// What the chooser hands back: the files and the filter chosen; None: cancelled.
pub type Done = Box<dyn FnOnce(Option<(Vec<PathBuf>, Option<Filter>)>)>;

pub struct Chooser {
    request: Request,
    window: Weak<Window>,
    name: Option<gtk::Entry>,
    filter: Option<gtk::ComboBoxText>,
    accept: gtk::Button,
    done: RefCell<Option<Done>>,
}

thread_local! {
    static OPEN: RefCell<Vec<Rc<Chooser>>> = const { RefCell::new(Vec::new()) };
}

/// The chooser shown last (tests).
pub fn current() -> Option<(Rc<Chooser>, Rc<Window>)> {
    OPEN.with(|o| o.borrow().iter().rev().find_map(|c| c.window.upgrade().map(|w| (c.clone(), w))))
}

/// Opens a chooser window for `request`; `done` gets the answer once.
pub fn open(app: &Rc<AppCtx>, request: Request, done: Done) -> Rc<Chooser> {
    let start = request.folder.clone().filter(|f| f.is_dir()).unwrap_or_else(glib::home_dir);
    let w = Window::new(app, vec![Location::Dir(start)]);
    app.adopt_window(w.clone());
    w.menubar.hide();
    w.win.set_default_size(980, 640);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.set_border_width(8);
    let name = matches!(request.mode, Mode::Save { .. }).then(|| {
        let e = gtk::Entry::new();
        e.set_widget_name("chooser-name");
        e.set_hexpand(true);
        e.set_activates_default(true);
        if let Mode::Save { name } = &request.mode {
            e.set_text(name);
        }
        let l = gtk::Label::with_mnemonic("_Name:");
        l.set_mnemonic_widget(Some(&e));
        bar.pack_start(&l, false, false, 0);
        bar.pack_start(&e, true, true, 0);
        e
    });
    if name.is_none() {
        // What is picked, said in words.
        let hint = gtk::Label::new(Some(match &request.mode {
            Mode::Open { directory: true, .. } => "Open the folder to pick, or select it.",
            Mode::Open { multiple: true, .. } => "Select one or more files (Ctrl+click for more).",
            Mode::SaveFiles { names } if names.len() == 1 => "Open the folder to save the file in.",
            Mode::SaveFiles { .. } => "Open the folder to save the files in.",
            _ => "Select a file, or double-click it.",
        }));
        hint.set_xalign(0.0);
        hint.style_context().add_class("dim-label");
        bar.pack_start(&hint, true, true, 0);
    }
    let filter = (!request.filters.is_empty()).then(|| {
        let c = gtk::ComboBoxText::new();
        c.set_widget_name("chooser-filter");
        for (i, f) in request.filters.iter().enumerate() {
            c.append(Some(&i.to_string()), &f.name);
        }
        c.set_active(request.current_filter.map(|i| i as u32));
        bar.pack_start(&c, false, false, 0);
        c
    });
    let cancel = gtk::Button::with_mnemonic("_Cancel");
    let label = request.accept_label.clone().unwrap_or_else(|| match &request.mode {
        Mode::Open { directory: true, .. } => "_Select".into(),
        Mode::Open { .. } => "_Open".into(),
        _ => "_Save".into(),
    });
    let accept = gtk::Button::with_mnemonic(&label);
    accept.style_context().add_class("suggested-action");
    accept.set_can_default(true);
    bar.pack_end(&accept, false, false, 0);
    bar.pack_end(&cancel, false, false, 0);
    w.bottom.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 0);
    w.bottom.pack_start(&bar, false, false, 0);
    w.bottom.show_all();
    w.win.set_default(Some(&accept));
    let c = Rc::new(Chooser { request, window: Rc::downgrade(&w), name, filter, accept, done: RefCell::new(Some(done)) });
    *w.chooser.borrow_mut() = Some(c.clone());
    w.win.set_title(&c.request.title);
    OPEN.with(|o| o.borrow_mut().push(c.clone()));
    c.apply_filter();

    let weak = Rc::downgrade(&c);
    cancel.connect_clicked(move |_| {
        if let Some(c) = weak.upgrade() {
            c.finish(None);
        }
    });
    let weak = Rc::downgrade(&c);
    c.accept.connect_clicked(move |_| {
        if let Some(c) = weak.upgrade() {
            c.accept_now();
        }
    });
    if let Some(f) = &c.filter {
        let weak = Rc::downgrade(&c);
        f.connect_changed(move |_| {
            if let Some(c) = weak.upgrade() {
                c.apply_filter();
            }
        });
    }
    if let Some(e) = &c.name {
        let weak = Rc::downgrade(&c);
        e.connect_changed(move |_| {
            if let Some(c) = weak.upgrade() {
                c.update();
            }
        });
        e.grab_focus();
        // The name without its extension is selected, ready to be typed over (after the
        // focus, which selects all).
        let name = e.text().to_string();
        let stem = name.rfind('.').filter(|i| *i > 0).map(|i| name[..i].chars().count() as i32).unwrap_or(-1);
        e.select_region(0, stem);
    }
    // The selection and the folder decide whether the button may be pressed.
    let weak = Rc::downgrade(&c);
    w.connect_location_changed(move |_| {
        if let Some(c) = weak.upgrade() {
            c.apply_filter();
        }
    });
    let weak = Rc::downgrade(&c);
    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        let Some(c) = weak.upgrade() else { return glib::ControlFlow::Break };
        if c.done.borrow().is_none() {
            return glib::ControlFlow::Break;
        }
        c.update();
        glib::ControlFlow::Continue
    });
    // Closing the window cancels.
    let weak = Rc::downgrade(&c);
    w.win.connect_destroy(move |_| {
        if let Some(c) = weak.upgrade() {
            c.finish(None);
        }
    });
    w.win.present();
    c
}

impl Chooser {
    fn window(&self) -> Option<Rc<Window>> {
        self.window.upgrade()
    }

    fn current_filter(&self) -> Option<Filter> {
        let i: usize = self.filter.as_ref()?.active_id()?.parse().ok()?;
        self.request.filters.get(i).cloned()
    }

    /// The type filter on every tab; a folder chooser shows folders only.
    fn apply_filter(&self) {
        let Some(w) = self.window() else { return };
        let f = match self.request.mode {
            Mode::Open { directory: true, .. } => Some(Filter { name: "Folders".into(), globs: vec![], types: vec![] }),
            _ => self.current_filter(),
        };
        for p in w.panes() {
            p.set_type_filter(f.clone());
        }
        self.update();
    }

    /// The folder shown, if it is one on disk (not an archive, the trash, …).
    fn folder(&self) -> Option<PathBuf> {
        match self.window()?.current_location() {
            Location::Dir(d) => Some(d),
            _ => None,
        }
    }

    fn selected(&self) -> Vec<Item> {
        self.window().map(|w| w.current_pane().selected_items()).unwrap_or_default()
    }

    /// What pressing the button would pick now.
    fn picks(&self) -> Vec<PathBuf> {
        let sel = self.selected();
        match &self.request.mode {
            Mode::Open { directory: false, multiple } => {
                let files: Vec<PathBuf> = sel.iter().filter(|i| !i.is_dir_like()).filter_map(|i| i.path().cloned()).collect();
                if !multiple && files.len() > 1 { Vec::new() } else { files }
            }
            Mode::Open { directory: true, .. } => match sel.as_slice() {
                [one] if one.is_dir_like() => one.path().cloned().into_iter().collect(),
                _ => self.folder().into_iter().collect(),
            },
            Mode::Save { .. } => {
                let name = self.name.as_ref().map(|e| e.text().trim().to_string()).unwrap_or_default();
                match self.folder() {
                    Some(d) if super::util::valid_file_name(&name) => vec![d.join(name)],
                    _ => Vec::new(),
                }
            }
            Mode::SaveFiles { names } => self.folder().map(|d| names.iter().map(|n| d.join(n)).collect()).unwrap_or_default(),
        }
    }

    fn update(&self) {
        self.accept.set_sensitive(!self.picks().is_empty());
        // Saving: a file selected gives its name.
        if let (Mode::Save { .. }, Some(e), [Item::Fs(f)]) = (&self.request.mode, &self.name, self.selected().as_slice()) {
            if !f.is_dir_like() && !e.has_focus() && e.text().as_str() != f.name.to_string_lossy() {
                e.set_text(&f.name.to_string_lossy());
            }
        }
    }

    /// A file opened (double click, Enter): picked, where that makes sense.
    pub fn activated(&self, items: &[Item]) {
        match &self.request.mode {
            Mode::Open { directory: false, .. } => self.accept_now(),
            Mode::Save { .. } => {
                if let (Some(e), [one]) = (&self.name, items) {
                    e.set_text(&one.display_name());
                    self.accept_now();
                }
            }
            _ => {}
        }
    }

    /// Picks what is chosen; saving over files that are there asks first.
    pub fn accept_now(&self) {
        let picks = self.picks();
        if picks.is_empty() {
            return;
        }
        let saving = matches!(self.request.mode, Mode::Save { .. } | Mode::SaveFiles { .. });
        let taken: Vec<&PathBuf> = picks.iter().filter(|p| saving && p.symlink_metadata().is_ok()).collect();
        let Some(w) = self.window() else { return };
        if taken.is_empty() {
            self.finish(Some(picks));
            return;
        }
        let what = match taken.as_slice() {
            [one] => format!("“{}”", one.file_name().unwrap_or_default().to_string_lossy()),
            many => format!("{} files", many.len()),
        };
        let me = OPEN.with(|o| o.borrow().iter().find(|c| std::ptr::eq(c.as_ref(), self)).cloned());
        super::util::confirm(&w.win, &format!("Replace {what}?"), "Saving replaces what is there.", "_Replace", move || {
            if let Some(c) = &me {
                c.finish(Some(picks.clone()));
            }
        });
    }

    /// Answers once and closes the window.
    fn finish(&self, picks: Option<Vec<PathBuf>>) {
        let Some(done) = self.done.borrow_mut().take() else { return };
        let filter = self.current_filter();
        done(picks.map(|p| (p, filter)));
        OPEN.with(|o| o.borrow_mut().retain(|c| !std::ptr::eq(c.as_ref(), self)));
        if let Some(w) = self.window() {
            w.win.close();
        }
    }

    /// Closed from outside (the application gave up waiting).
    pub fn cancel(&self) {
        self.finish(None);
    }

    /// The title the application gave the dialog.
    pub fn title(&self) -> String {
        self.request.title.clone()
    }

    /// The name field (tests).
    pub fn set_name(&self, text: &str) {
        if let Some(e) = &self.name {
            e.set_text(text);
        }
    }

    pub fn set_filter(&self, i: u32) {
        if let Some(f) = &self.filter {
            f.set_active(Some(i));
        }
    }
}
