//! Git in the file manager, the way TortoiseGit does it in Explorer: overlay icons on the
//! files and folders of working trees, "Git Sync…", "Git Commit -> "branch"…" and the
//! TortoiseGit submenu in the right-click menu, and TortoiseGit's dialogs for what they do.

pub mod askpass;
pub mod blame;
pub mod commit;
pub mod dialog;
pub mod dialogs;
pub mod diff;
pub mod filelist;
pub mod icons;
pub mod log;
pub mod overlay;
pub mod progress;
pub mod sync;
pub mod testing;

use super::model::Item;
use super::pane::Pane;
use super::window::Window;
use failbrauwser::vcs::git::{self, Repo, Status};
use failbrauwser::location::Location;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::path::PathBuf;
use std::rc::Rc;

/// What a Git menu item works on: the selected files and folders, or the folder shown.
#[derive(Clone, Debug)]
pub struct Target {
    pub repo: Option<Repo>,
    pub paths: Vec<PathBuf>,
    /// Each path's overlay status, where known.
    pub status: Vec<Option<Status>>,
    pub is_dir: Vec<bool>,
    /// The folder shown.
    pub folder: PathBuf,
    /// Nothing is selected: the menu of the folder itself.
    pub background: bool,
}

impl Target {
    /// The paths relative to the top of the working tree ("" for all of it).
    pub fn rel(&self) -> Vec<String> {
        let Some(r) = &self.repo else { return Vec::new() };
        let rel: Vec<String> = self.paths.iter().filter_map(|p| r.relative(p)).collect();
        if rel.iter().any(String::is_empty) { vec![String::new()] } else { rel }
    }

    /// What the dialog titles name: the one path, or the folder.
    pub fn title_path(&self) -> PathBuf {
        match self.paths.as_slice() {
            [one] => one.clone(),
            _ => self.folder.clone(),
        }
    }

    /// The one file selected (not a folder), relative to the top of the working tree.
    pub fn single_file(&self) -> Option<String> {
        match (self.paths.as_slice(), self.is_dir.as_slice()) {
            ([p], [false]) => self.repo.as_ref()?.relative(p),
            _ => None,
        }
    }

    fn any(&self, f: impl Fn(Option<Status>) -> bool) -> bool {
        self.status.iter().any(|s| f(*s))
    }
}

impl Target {
    /// The whole working tree.
    pub fn of_repo(repo: &Repo) -> Target {
        Target { repo: Some(repo.clone()), paths: vec![repo.root.clone()], status: vec![None], is_dir: vec![true], folder: repo.root.clone(), background: true }
    }
}

/// Runs `work` on a worker thread and hands its result to `done` on the UI thread.
pub fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("fb-git".into())
        .spawn(move || {
            let _ = tx.send_blocking(work());
        })
        .expect("spawn git worker");
    glib::spawn_future_local(async move {
        if let Ok(v) = rx.recv().await {
            done(v);
        }
    });
}

/// What the Git items work on in the window's current tab.
pub fn target(w: &Window) -> Option<Target> {
    target_of(&w.current_pane(), &w.current_pane().selected_items())
}

fn target_of(pane: &Rc<Pane>, items: &[Item]) -> Option<Target> {
    let folder = match pane.location() {
        Location::Dir(d) => d,
        Location::Search(s) => s.root.clone(),
        _ => return None,
    };
    if failbrauwser::ops::gvfs::is_gvfs_path(&folder) {
        return None;
    }
    let mut t = Target { repo: None, paths: Vec::new(), status: Vec::new(), is_dir: Vec::new(), folder: folder.clone(), background: items.is_empty() };
    if items.is_empty() {
        t.paths.push(folder.clone());
        t.status.push(None);
        t.is_dir.push(true);
    }
    for i in items {
        let (Item::Fs(e) | Item::Found(e, _)) = i else { return None };
        t.paths.push(e.path.clone());
        t.status.push(pane.git_status(&i.os_name().to_string_lossy()).and_then(icons::status_from_code));
        t.is_dir.push(e.is_dir_like());
    }
    t.repo = git::discover(&t.paths[0]);
    // Items of different working trees: none of them.
    if let Some(r) = &t.repo {
        if t.paths.iter().any(|p| git::discover(p).as_ref() != Some(r)) {
            t.repo = None;
        }
    }
    Some(t)
}

/// An item of the menu with its Git icon.
fn item(label: &str, action: &str, icon: &str) -> gio::MenuItem {
    let it = gio::MenuItem::new(Some(label), Some(action));
    if let Some(i) = icons::gicon(icon) {
        it.set_icon(&i);
    }
    it
}

fn item_with(label: &str, action: &str, value: &str, icon: &str) -> gio::MenuItem {
    let it = gio::MenuItem::new(Some(label), None);
    it.set_action_and_target_value(Some(action), Some(&value.to_variant()));
    if let Some(i) = icons::gicon(icon) {
        it.set_icon(&i);
    }
    it
}

fn section(items: Vec<gio::MenuItem>) -> Option<gio::Menu> {
    if items.is_empty() {
        return None;
    }
    let s = gio::Menu::new();
    for i in items {
        s.append_item(&i);
    }
    Some(s)
}

/// Menu texts take '_' as mnemonic mark: names keep theirs.
fn escape(s: &str) -> String {
    s.replace('_', "__")
}

/// The Git part of the right-click menu, as TortoiseGit has it in Explorer: in a working
/// tree "Git Sync…", "Git Commit -> "branch"…" and the TortoiseGit submenu; elsewhere
/// "Git Clone…" and "Git Create repository here…" for folders.
pub fn menu_section(w: &Window, pane: &Rc<Pane>, items: &[Item]) -> Option<gio::Menu> {
    if !w.app.settings.borrow().vcs_integration || !git::available() {
        return None;
    }
    let t = target_of(pane, items)?;
    let top = gio::Menu::new();
    let sub = gio::Menu::new();
    let all_dirs = t.is_dir.iter().all(|d| *d);
    match &t.repo {
        None => {
            if all_dirs && t.paths.len() == 1 {
                top.append_item(&item("Git _Clone...", "win.vcs-clone", "clone"));
                top.append_item(&item("Git Create _repository here...", "win.vcs-init", "createrepo"));
            }
        }
        Some(repo) => {
            top.append_item(&item("Git _Sync...", "win.vcs-sync", "sync"));
            top.append_item(&item(&format!("Git _Commit -> \"{}\"...", escape(&repo.branch_label())), "win.vcs-commit", "commit"));
            for s in repo_submenu(repo, &t) {
                sub.append_section(None, &s);
            }
        }
    }
    let about = section(vec![item("_Settings", "win.vcs-settings", "settings"), item("_About", "win.vcs-about", "about")]).expect("items");
    sub.append_section(None, &about);
    let tg = gio::MenuItem::new_submenu(Some("TortoiseGit"), &sub);
    if let Some(i) = icons::gicon("tgit") {
        tg.set_icon(&i);
    }
    top.append_item(&tg);
    Some(top)
}

/// The sections of the TortoiseGit submenu inside a working tree.
fn repo_submenu(repo: &Repo, t: &Target) -> Vec<gio::Menu> {
    let mut out = Vec::new();
    let one_file = t.single_file();
    let changed = t.background || t.is_dir.iter().any(|d| *d) || t.any(|s| s.is_some_and(Status::is_change));
    let versioned = t.any(|s| matches!(s, Some(s) if s != Status::Unversioned && s != Status::Ignored));
    let unversioned = t.any(|s| s == Some(Status::Unversioned));
    let conflicted = t.any(|s| s == Some(Status::Conflicted)) || ((t.background || t.is_dir.iter().any(|d| *d)) && repo.git_dir.join("MERGE_HEAD").exists());
    let stash = repo.common_dir().join("logs/refs/stash").exists();
    out.extend(section(vec![item("_Pull...", "win.vcs-pull", "pull"), item("_Fetch...", "win.vcs-fetch", "fetch"), item("Pu_sh...", "win.vcs-push", "push")]));
    let mut diff = Vec::new();
    if one_file.is_some() && t.any(|s| s.is_some_and(Status::is_change)) {
        diff.push(item("_Diff", "win.vcs-diff", "diff"));
    }
    if one_file.is_some() && versioned && !t.any(|s| s == Some(Status::Added)) {
        diff.push(item("Diff with previous _version", "win.vcs-diff-prev", "diffprev"));
    }
    out.extend(section(diff));
    let mut log = vec![item("Show _log", "win.vcs-log", "log"), item("Show _Reflog", "win.vcs-reflog", "reflog")];
    if one_file.is_some() && versioned && !t.any(|s| s == Some(Status::Added)) {
        log.push(item("_Blame", "win.vcs-blame", "blame"));
    }
    log.push(item("Check for _modifications", "win.vcs-checkmods", "checkmods"));
    log.push(item("Stash Sa_ve", "win.vcs-stash-save", "stashsave"));
    if stash {
        log.push(item("Stash P_op", "win.vcs-stash-pop", "stashpop"));
    }
    out.extend(section(log));
    let mut fix = Vec::new();
    if conflicted {
        fix.push(item("Resol_ve...", "win.vcs-resolve", "resolve"));
    }
    if changed {
        fix.push(item("_Revert...", "win.vcs-revert", "revert"));
    }
    if t.background || t.is_dir.iter().all(|d| *d) {
        fix.push(item("Clean _up...", "win.vcs-cleanup", "cleanup"));
    }
    out.extend(section(fix));
    out.extend(section(vec![
        item("S_witch/Checkout...", "win.vcs-switch", "switch"),
        item("_Merge...", "win.vcs-merge", "merge"),
        item("Create _Branch...", "win.vcs-branch", "branch"),
        item("Create _Tag...", "win.vcs-tag", "tag"),
        item("_Export...", "win.vcs-export", "export"),
    ]));
    let mut files = Vec::new();
    if t.background || unversioned || t.is_dir.iter().any(|d| *d) {
        files.push(item("_Add...", "win.vcs-add", "add"));
    }
    if !t.background && versioned {
        files.push(item("_Delete", "win.vcs-delete", "delete"));
        files.push(item("Delete (_keep local)", "win.vcs-delete-keep", "deletekeep"));
    }
    if !t.background && versioned && t.paths.len() == 1 {
        files.push(item("Re_name...", "win.vcs-rename", "rename"));
    }
    if !t.background && t.any(|s| s == Some(Status::Unversioned)) {
        let ig = gio::Menu::new();
        if let [p] = t.paths.as_slice() {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for pat in git::ignore_patterns(&name) {
                ig.append_item(&item_with(&escape(&pat), "win.vcs-ignore", &pat, "ignore"));
            }
        } else {
            ig.append_item(&item_with(&format!("{} items by _name", t.paths.len()), "win.vcs-ignore", "", "ignore"));
        }
        let sm = gio::MenuItem::new_submenu(Some("Add to _ignore list"), &ig);
        if let Some(i) = icons::gicon("ignore") {
            sm.set_icon(&i);
        }
        files.push(sm);
    }
    out.extend(section(files));
    out.extend(section(vec![item("Create _Patch Serial...", "win.vcs-patch-create", "patchcreate"), item("Appl_y Patch Serial...", "win.vcs-patch-apply", "patchapply")]));
    out
}

/// Runs `f` with the working tree of the menu's target, or tells why there is none.
fn with_repo(w: &Window, f: impl FnOnce(&Window, Target, Repo)) {
    let Some(t) = target(w) else { return };
    match t.repo.clone() {
        Some(r) => f(w, t, r),
        None => dialog::error(&w.win, "This is not a Git working tree."),
    }
}

pub fn install_actions(w: &Rc<Window>) {
    overlay::window_created(w);
    let repo_actions: &[(&str, fn(&Window, Target, Repo))] = &[
        ("vcs-sync", sync::show),
        ("vcs-commit", |w, t, r| commit::show(w, &r, &t.title_path(), t.rel())),
        ("vcs-pull", dialogs::pull),
        ("vcs-fetch", dialogs::fetch),
        ("vcs-push", dialogs::push),
        ("vcs-diff", |w, t, r| {
            if let Some(f) = t.single_file() {
                diff::working(w, &r, &f);
            }
        }),
        ("vcs-diff-prev", |w, t, r| {
            if let Some(f) = t.single_file() {
                diff::previous(w, &r, &f);
            }
        }),
        ("vcs-log", |w, t, r| log::show(w, &r, &t.title_path(), t.rel())),
        ("vcs-reflog", dialogs::reflog),
        ("vcs-blame", |w, t, r| {
            if let Some(f) = t.single_file() {
                blame::show(w, &r, &f);
            }
        }),
        ("vcs-checkmods", dialogs::check_modifications),
        ("vcs-stash-save", dialogs::stash_save),
        ("vcs-stash-pop", dialogs::stash_pop),
        ("vcs-resolve", dialogs::resolve),
        ("vcs-revert", dialogs::revert),
        ("vcs-cleanup", dialogs::cleanup),
        ("vcs-switch", dialogs::switch),
        ("vcs-merge", dialogs::merge),
        ("vcs-branch", dialogs::branch),
        ("vcs-tag", dialogs::tag),
        ("vcs-export", dialogs::export),
        ("vcs-add", dialogs::add),
        ("vcs-delete", |w, t, r| dialogs::delete(w, t, r, false)),
        ("vcs-delete-keep", |w, t, r| dialogs::delete(w, t, r, true)),
        ("vcs-rename", dialogs::rename),
        ("vcs-patch-create", dialogs::patch_create),
        ("vcs-patch-apply", dialogs::patch_apply),
    ];
    for (name, f) in repo_actions {
        let f = *f;
        w.add_action(name, move |w| with_repo(w, f));
    }
    w.add_action("vcs-clone", |w| {
        if let Some(t) = target(w) {
            dialogs::clone(w, &t.title_path());
        }
    });
    w.add_action("vcs-init", |w| {
        if let Some(t) = target(w) {
            dialogs::init(w, &t.title_path());
        }
    });
    w.add_action("vcs-settings", |w| {
        let t = target(w);
        dialogs::settings(w, t.and_then(|t| t.repo));
    });
    w.add_action("vcs-about", dialogs::about);
    let a = gio::SimpleAction::new("vcs-ignore", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(w);
    a.connect_activate(move |_, v| {
        let (Some(w), Some(pattern)) = (weak.upgrade(), v.and_then(|v| v.get::<String>())) else { return };
        with_repo(&w, |w, t, r| dialogs::ignore(w, t, r, pattern));
    });
    w.win.add_action(&a);
}
