//! The location bar: a button for every step of the path, and a text field for typing a
//! location. Clicking right of the last button (or Ctrl+L) switches to the text field;
//! Enter, Escape or clicking elsewhere switches back to the buttons.
//!
//! Going up keeps the deeper buttons, so the way back down is one click too.

use failbrauwser::location::Location;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub struct PathBar {
    pub root: gtk::Stack,
    pub entry: gtk::Entry,
    crumbs: gtk::Box,
    scroller: gtk::ScrolledWindow,
    /// Current location, and the deepest location of the current path shown.
    current: RefCell<Option<Location>>,
    tail: RefCell<Option<Location>>,
    navigate: RefCell<Option<Box<dyn Fn(Location)>>>,
    weak: RefCell<Weak<PathBar>>,
}

/// One button: its label, icon and target.
#[derive(Debug, Clone, PartialEq)]
pub struct Crumb {
    pub label: String,
    pub icon: Option<&'static str>,
    pub location: Location,
}

/// The steps from the top (the file system, or home for paths below it) down to `loc`.
pub fn crumbs_for(loc: &Location) -> Vec<Crumb> {
    if *loc == Location::Drives {
        return vec![Crumb { label: "Drives".into(), icon: Some("drive-harddisk-symbolic"), location: Location::Drives }];
    }
    let home = Location::home();
    let mut chain = vec![loc.clone()];
    let mut cur = loc.clone();
    while cur != home {
        match cur.parent() {
            Some(p) => {
                chain.push(p.clone());
                cur = p;
            }
            None => break,
        }
    }
    chain.reverse();
    chain
        .into_iter()
        .map(|l| {
            let (label, icon) = match &l {
                Location::Dir(p) if *p == glib::home_dir() => ("Home".to_string(), Some("user-home-symbolic")),
                Location::Dir(p) if p.parent().is_none() => (String::new(), Some("drive-harddisk-symbolic")),
                Location::Archive(a) if a.inner.is_empty() => (l.title(), Some("package-x-generic-symbolic")),
                _ => (l.title(), None),
            };
            Crumb { label, icon, location: l }
        })
        .collect()
}

/// True when `a` is `b` or lies below it.
fn is_at_or_below(a: &Location, b: &Location) -> bool {
    let mut cur = Some(a.clone());
    while let Some(c) = cur {
        if &c == b {
            return true;
        }
        cur = c.parent();
    }
    false
}

impl PathBar {
    pub fn new() -> Rc<PathBar> {
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_input_purpose(gtk::InputPurpose::Url);

        let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        crumbs.style_context().add_class("linked");
        let scroller = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        // Scrollable without a scrollbar: long paths keep their end in view.
        scroller.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        scroller.set_propagate_natural_width(true);
        scroller.set_shadow_type(gtk::ShadowType::None);
        scroller.add(&crumbs);
        // The empty space after the last button opens the text field.
        let filler = gtk::EventBox::new();
        filler.set_hexpand(true);
        filler.set_tooltip_text(Some("Click to type a location"));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.pack_start(&scroller, false, true, 0);
        row.pack_start(&filler, true, true, 0);

        let root = gtk::Stack::new();
        root.set_hexpand(true);
        root.add_named(&row, "crumbs");
        root.add_named(&entry, "entry");
        root.set_visible_child_name("crumbs");
        root.show_all();

        let bar = Rc::new(PathBar {
            root,
            entry,
            crumbs,
            scroller,
            current: RefCell::new(None),
            tail: RefCell::new(None),
            navigate: RefCell::new(None),
            weak: RefCell::new(Weak::new()),
        });
        *bar.weak.borrow_mut() = Rc::downgrade(&bar);

        let weak = Rc::downgrade(&bar);
        filler.connect_button_press_event(move |_, ev| {
            if ev.button() == 1 {
                if let Some(b) = weak.upgrade() {
                    b.edit();
                }
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&bar);
        bar.entry.connect_focus_out_event(move |_, _| {
            if let Some(b) = weak.upgrade() {
                b.show_crumbs();
            }
            glib::Propagation::Proceed
        });
        // Keep the end of a long path in view when the bar is resized.
        let sc = bar.scroller.clone();
        bar.crumbs.connect_size_allocate(move |_, _| {
            let adj = sc.hadjustment();
            adj.set_value(adj.upper());
        });
        bar
    }

    pub fn connect_navigate(&self, f: impl Fn(Location) + 'static) {
        *self.navigate.borrow_mut() = Some(Box::new(f));
    }

    pub fn is_editing(&self) -> bool {
        self.root.visible_child_name().as_deref() == Some("entry")
    }

    /// Switches to the text field with the whole location selected.
    pub fn edit(&self) {
        if let Some(loc) = self.current.borrow().as_ref() {
            self.entry.set_text(&loc.display());
        }
        self.root.set_visible_child_name("entry");
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    pub fn show_crumbs(&self) {
        self.root.set_visible_child_name("crumbs");
    }

    /// The labels of the buttons shown (tests).
    pub fn labels(&self) -> Vec<String> {
        self.crumbs
            .children()
            .iter()
            .filter_map(|c| c.downcast_ref::<gtk::Button>().map(|b| b.widget_name().to_string()))
            .collect()
    }

    /// Clicks the button with this label (tests).
    pub fn click(&self, label: &str) -> bool {
        for c in self.crumbs.children() {
            if let Some(b) = c.downcast_ref::<gtk::Button>() {
                if b.widget_name() == label {
                    b.clicked();
                    return true;
                }
            }
        }
        false
    }

    pub fn set_location(&self, loc: &Location) {
        if !self.entry.has_focus() {
            self.entry.set_text(&loc.display());
            self.show_crumbs();
        }
        *self.current.borrow_mut() = Some(loc.clone());
        // Keep the deeper steps while moving up or down along the same path.
        let keep = self.tail.borrow().as_ref().is_some_and(|t| is_at_or_below(t, loc));
        if !keep {
            *self.tail.borrow_mut() = Some(loc.clone());
        }
        let tail = self.tail.borrow().clone().unwrap_or_else(|| loc.clone());
        self.rebuild(&crumbs_for(&tail), loc);
    }

    fn rebuild(&self, crumbs: &[Crumb], current: &Location) {
        for c in self.crumbs.children() {
            self.crumbs.remove(&c);
        }
        for crumb in crumbs {
            let b = gtk::Button::new();
            b.set_focus_on_click(false);
            // Names identify buttons in tests; the root button has only an icon.
            b.set_widget_name(if crumb.label.is_empty() { "/" } else { &crumb.label });
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            if let Some(icon) = crumb.icon {
                content.pack_start(&gtk::Image::from_gicon(&gio::ThemedIcon::new(icon), gtk::IconSize::Menu), false, false, 0);
            }
            if !crumb.label.is_empty() {
                let label = gtk::Label::new(None);
                let text = glib::markup_escape_text(&crumb.label);
                // The current step in bold, like other path bars.
                if crumb.location == *current {
                    label.set_markup(&format!("<b>{text}</b>"));
                } else {
                    label.set_text(&crumb.label);
                }
                label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                label.set_max_width_chars(24);
                content.pack_start(&label, false, false, 0);
            }
            b.add(&content);
            b.set_tooltip_text(Some(&crumb.location.display()));
            let target = crumb.location.clone();
            let weak = self.weak.borrow().clone();
            b.connect_clicked(move |_| {
                if let Some(bar) = weak.upgrade() {
                    if let Some(f) = bar.navigate.borrow().as_ref() {
                        f(target.clone());
                    }
                }
            });
            self.crumbs.pack_start(&b, false, false, 0);
        }
        self.crumbs.show_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use failbrauwser::location::ArchiveLoc;
    use std::path::PathBuf;

    #[test]
    fn paths_below_home_start_at_home() {
        let loc = Location::Dir(glib::home_dir().join("Music/Albums"));
        let c = crumbs_for(&loc);
        let labels: Vec<_> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["Home", "Music", "Albums"]);
        assert_eq!(c[0].location, Location::home());
    }

    #[test]
    fn other_paths_start_at_the_root() {
        let c = crumbs_for(&Location::Dir(PathBuf::from("/usr/share")));
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].icon, Some("drive-harddisk-symbolic"));
        assert_eq!(c[2].label, "share");
    }

    #[test]
    fn archives_continue_the_path() {
        let a = ArchiveLoc::root(PathBuf::from("/data/x.zip")).enter_nested("in/y.tar").with_inner("docs");
        let c = crumbs_for(&Location::Archive(a));
        let labels: Vec<_> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["", "data", "x.zip", "in", "y.tar", "docs"]);
        assert_eq!(c[2].icon, Some("package-x-generic-symbolic"));
        assert_eq!(crumbs_for(&Location::Drives)[0].label, "Drives");
    }

    #[test]
    fn ancestry() {
        let deep = Location::Dir(PathBuf::from("/a/b/c"));
        assert!(is_at_or_below(&deep, &Location::Dir(PathBuf::from("/a"))));
        assert!(!is_at_or_below(&Location::Dir(PathBuf::from("/a")), &deep));
    }
}
