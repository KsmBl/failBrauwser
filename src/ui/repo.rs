//! The Repository menu and the windows its commands answer in (M15). The submenu is the folder's
//! own tool's commands as filtered by what the selection carries (`failbrauwser::repo`), and every
//! entry names the window action its command owns: the menu is built from that same list, so an
//! entry never stands without a window behind it. Nothing here reads a repository's own data — the
//! tool's client is asked, and its words are shown as they arrived, because that is the answer a
//! user can act on.

use super::model::Item;
use super::pane::Pane;
use super::window::Window;
use super::util;
use failbrauwser::location::Location;
use failbrauwser::repo::run as client;
use failbrauwser::repo::verbs::{Ask, Command, Request, Selection};
use failbrauwser::repo::{self, Tool};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The title the Clone form gives itself, and the hint under it (M15). A scenario names these to
/// find the form, so they are stated once here.
const CLONE_TITLE: &str = "Clone a repository";
const CLONE_HINT: &str = "The address to read from, the folder it makes, and the tool that clones.";

/// The window a Repository command answers in (M15): the client's words line for line, and a
/// status line naming the calls that ran and how they ended.
pub struct Answer {
    pub window: gtk::Window,
    status: gtk::Label,
    body: gtk::TextView,
    /// The calls that have not answered yet: the answer is in once this is 0.
    pending: RefCell<usize>,
}

/// The Clone form (M15): no marker says which tool should clone into a folder no tool owns, so the
/// address, the folder it makes and the tool that clones are asked together.
pub struct CloneForm {
    address: gtk::Entry,
    folder: gtk::Entry,
    tool: gtk::ComboBoxText,
}

thread_local! {
    static ANSWERS: RefCell<Vec<Rc<Answer>>> = const { RefCell::new(Vec::new()) };
    static CLONE: RefCell<Option<Rc<CloneForm>>> = const { RefCell::new(None) };
}

/// The answer window opened last and still shown (tests).
pub fn current() -> Option<Rc<Answer>> {
    ANSWERS.with(|a| a.borrow().last().cloned())
}

/// The Clone form shown, if one is (tests).
pub fn clone_form() -> Option<Rc<CloneForm>> {
    CLONE.with(|c| c.borrow().clone())
}

/// The commands the submenu offers for this folder (M15): its tool's commands that the selection
/// can carry, or *Clone* alone where no working copy reaches — there a clone is the only thing a
/// repository command can do, and the tool that clones has to be asked for.
fn offered(tool: Option<Tool>, selection: &Selection) -> Vec<Command> {
    match tool {
        None => vec![Command::Clone],
        Some(t) => Command::all().iter().copied().filter(|c| c.applies(t, selection, true)).collect(),
    }
}

/// What the selection carries, as far as the Repository menu cares (M15).
fn selection_of(items: &[Item]) -> Selection {
    Selection { files: items.iter().filter(|i| !i.is_dir_like()).count(), folders: items.iter().filter(|i| i.is_dir_like()).count() }
}

/// The Repository submenu of the folder being viewed (M15). A place that is no folder answers
/// nothing: an archive, a share listed through GVfs and the recycle bin hold no working copy.
pub fn submenu(pane: &Pane, items: &[Item]) -> Option<gio::Menu> {
    let dir = match pane.location() { Location::Dir(p) => p, _ => return None };
    let menu = gio::Menu::new();
    for c in offered(repo::find(&dir).map(|c| c.tool), &selection_of(items)) {
        menu.append(Some(c.label()), Some(&format!("win.{}", c.action())));
    }
    Some(menu)
}

/// One action per command, under the very name the command gives its menu entry (M15) — which is why
/// an entry cannot stand without a window behind it: the menu spells `win.` and this name out of the
/// same source. GTK3 greys an entry whose action is missing, so installing all of them is what keeps
/// the entries live.
pub fn install_actions(w: &Window) {
    for c in Command::all() {
        w.add_action(c.action(), move |w| run(w, *c));
    }
}

impl CloneForm {
    /// Fills one of the form's fields (tests): its address, its folder, or the tool it is to use.
    pub fn set(&self, field: &str, value: &str) -> Result<(), String> {
        match field {
            "address" => {
                self.address.set_text(value);
                Ok(())
            }
            "folder" => {
                self.folder.set_text(value);
                Ok(())
            }
            "tool" => {
                let Some(at) = clone_tools().iter().position(|t| t.label() == value) else { return Err(format!("no tool named {value} can clone")) };
                self.tool.set_active(Some(at as u32));
                Ok(())
            }
            other => Err(format!("the Clone form has no field named {other}")),
        }
    }

    /// The folder the form will make (tests).
    pub fn folder(&self) -> String {
        self.folder.text().to_string()
    }

    /// The tool the form has chosen (tests).
    pub fn tool(&self) -> Option<String> {
        self.tool.active_text().map(|t| t.to_string())
    }
}

impl Answer {
    /// A window for one command's answer, naming the calls it is about to run (M15).
    fn shown(w: &Window, title: &str, status: &str) -> Rc<Answer> {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title(title);
        window.set_transient_for(Some(&w.win));
        window.set_default_size(680, 360);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_border_width(12);
        let status = gtk::Label::new(Some(status));
        status.set_xalign(0.0);
        status.set_line_wrap(true);
        status.set_widget_name("repo-status");
        let body = gtk::TextView::new();
        body.set_editable(false);
        body.set_cursor_visible(false);
        body.set_monospace(true);
        body.set_wrap_mode(gtk::WrapMode::WordChar);
        let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroll.set_min_content_height(240);
        scroll.set_shadow_type(gtk::ShadowType::In);
        scroll.add(&body);
        let close = gtk::Button::with_mnemonic("_Close");
        let closed = window.clone();
        close.connect_clicked(move |_| closed.close());
        root.pack_start(&status, false, false, 0);
        root.pack_start(&scroll, true, true, 0);
        root.pack_start(&close, false, false, 0);
        window.add(&root);
        // An answer window closed by its user leaves the list, so a scenario never reads a stale one.
        window.connect_destroy(move |w| ANSWERS.with(|all| all.borrow_mut().retain(|a| a.window != *w)));
        window.show_all();
        let a = Rc::new(Answer { window, status, body, pending: RefCell::new(0) });
        ANSWERS.with(|all| all.borrow_mut().push(a.clone()));
        a
    }

    /// The words the window shows (tests).
    pub fn text(&self) -> String {
        let b = self.body.buffer().expect("a text view has a buffer");
        let (start, end) = b.bounds();
        b.text(&start, &end, false).unwrap_or_default().to_string()
    }

    /// The line naming the calls and how they ended (tests).
    pub fn status(&self) -> String {
        self.status.text().to_string()
    }

    /// A call that has not answered yet (tests).
    pub fn is_busy(&self) -> bool {
        *self.pending.borrow() > 0
    }

    /// The client's words, and the calls that produced them ending how they ended (M15).
    fn fill(&self, out: &client::Answer, shown: &str) {
        self.body.buffer().expect("a text view has a buffer").set_text(&out.text);
        let end = match (out.ok, out.code) {
            (true, _) => "done".to_string(),
            (false, Some(code)) => format!("refused, exit code {code}"),
            (false, None) => "refused".to_string(),
        };
        self.status.set_text(&format!("{shown} — {end}"));
    }
}

/// A window answering in this program's own words, where no client call was to be made (M15).
fn say(w: &Window, title: &str, how: &str, words: &str) {
    let a = Answer::shown(w, title, how);
    a.body.buffer().expect("a text view has a buffer").set_text(words);
}

/// Runs one Repository command (M15). The folder's own tool answers it, the words that tool would
/// otherwise read from an editor of its own are asked for first, and a word that never arrives
/// starts no client at all.
fn run(w: &Window, c: Command) {
    let pane = w.current_pane();
    let dir = match pane.location() { Location::Dir(p) => p, _ => return };
    if c == Command::Clone {
        ask_clone(w, &dir);
        return;
    }
    let Some(copy) = repo::find(&dir) else {
        say(w, c.label(), "no working copy", &format!("{} is not inside a working copy of any tool this program knows, so it has no client of its own to ask.", dir.display()));
        return;
    };
    let items = pane.selected_items();
    let picked: Vec<&Path> = items.iter().filter_map(|i| i.path().map(|p| p.as_path())).collect();
    // The names as the client sees them: from the folder it runs in, since a central tool reads the
    // names it is given relative to where it runs.
    let names: Vec<&Path> = picked.iter().map(|p| p.strip_prefix(&dir).unwrap_or(p)).collect();
    let tool = copy.tool;
    match c.asks(tool) {
        None => answer(w, c, tool, &dir, &names, &Request::default()),
        Some(ask) => {
            let (title, hint, ok) = question(c, tool);
            let initial = match ask { Ask::NewName => items.first().map(|i| i.display_name()).unwrap_or_default(), _ => String::new() };
            let weak = Rc::downgrade(&w.me());
            let dir = dir.clone();
            let names: Vec<PathBuf> = names.iter().map(|p| p.to_path_buf()).collect();
            util::ask_text(&w.win, title, hint, &initial, ok, matches!(ask, Ask::NewName), move |word| {
                let Some(w) = weak.upgrade() else { return };
                let what = request_with(ask, &word);
                let names: Vec<&Path> = names.iter().map(|p| p.as_path()).collect();
                answer(&w, c, tool, &dir, &names, &what);
            });
        }
    }
}

/// The words a command was given, in the shape its arguments are spelled from (M15).
fn request_with(ask: Ask, word: &str) -> Request<'_> {
    match ask {
        Ask::Message => Request { message: Some(word), ..Default::default() },
        Ask::Reference => Request { reference: Some(word), ..Default::default() },
        Ask::Address => Request { url: Some(word), ..Default::default() },
        Ask::NewName => Request { new_name: Some(word), ..Default::default() },
    }
}

/// What a command asks before anything runs (M15): the title, the words that say what is wanted,
/// and the button that lets the client have its way. Only the commands whose client would open an
/// editor or a prompt of its own ask — see `Command::asks`.
fn question(c: Command, tool: Tool) -> (&'static str, &'static str, &'static str) {
    match (c, tool) {
        (Command::Commit, Tool::VisualSourceSafe) => ("Check in", "The comment Visual Source Safe records with this version.", "_Check in"),
        (Command::Commit, _) => ("Commit", "The message the tool records with this change.", "_Commit"),
        (Command::Rename, _) => ("Rename in the repository", "The name the tool moves this to.", "_Rename"),
        (Command::SwitchCheckout, Tool::Subversion) => ("Switch", "The place in the repository this folder is to follow.", "_Switch"),
        (Command::SwitchCheckout, Tool::VisualSourceSafe) => ("Get a version", "The version to get, in the label Visual Source Safe shows.", "_Get"),
        (Command::SwitchCheckout, _) => ("Switch/Checkout", "The branch, tag or revision to work on.", "_Switch"),
        (Command::DiffWithPrevious, _) => ("Diff with previous version", "The revision to compare with the one before it.", "_Diff"),
        _ => (c.label(), "The words this command needs before its client can run.", "_Run"),
    }
}

/// The calls a command answers, as the status line names them (M15): each one its client's own
/// name and arguments. The format strings this program asks the clients for carry control
/// characters, which would only break the line, so the name keeps the words.
fn spelled(tool: Tool, runs: &[Vec<String>]) -> String {
    let one = |argv: &[String]| argv.iter().map(|a| a.chars().filter(|c| !c.is_control()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" ");
    runs.iter().map(|argv| format!("{} {}", tool.cli(), one(argv))).collect::<Vec<_>>().join("; ")
}

/// Runs the calls this command answers, in a window of this program's own (M15).
fn answer(w: &Window, c: Command, tool: Tool, dir: &Path, names: &[&Path], what: &Request) {
    let runs = c.runs(tool, what, names);
    if runs.is_empty() {
        say(w, c.label(), "nothing ran", &format!("{} needs words it was not given, so no call of {} ran.", c.label(), tool.label()));
        return;
    }
    let shown = spelled(tool, &runs);
    let a = Answer::shown(w, &format!("{} — {}", c.label(), tool.label()), &format!("Running {shown}…"));
    let weak = Rc::downgrade(&a);
    *a.pending.borrow_mut() += 1;
    let seen = Rc::downgrade(&w.me());
    let dir = dir.to_path_buf();
    let names: Vec<PathBuf> = names.iter().map(|p| p.to_path_buf()).collect();
    glib::spawn_future_local(async move {
        let out = gio::spawn_blocking(move || finish(c, tool, &dir, &names, &runs)).await.unwrap_or_else(|_| client::Answer { ok: false, text: "the client could not be started here.".into(), code: None });
        let ok = out.ok;
        if let Some(a) = weak.upgrade() {
            a.fill(&out, &shown);
            *a.pending.borrow_mut() -= 1;
        }
        // A command that can change what the folder holds is followed by the folder read again.
        if ok && changes_the_folder(c) && let Some(w) = seen.upgrade() {
            w.current_pane().reload();
        }
    });
}

/// The work behind the window: the tool's own calls in order, and what its records need besides
/// them (M15).
fn finish(c: Command, tool: Tool, dir: &Path, names: &[PathBuf], runs: &[Vec<String>]) -> client::Answer {
    // CVS's own remove takes the local copy with it, so the copy this command was asked to keep is
    // put back once the client has had its way (M15).
    let kept: Vec<(PathBuf, Option<Vec<u8>>)> = if c == Command::DeleteKeepLocal && matches!(tool, Tool::Cvs) {
        names.iter().map(|n| (dir.join(n), std::fs::read(dir.join(n)).ok())).collect()
    } else {
        Vec::new()
    };
    let mut out = client::call_in_order(tool, dir, runs);
    for (p, bytes) in kept {
        if let Some(bytes) = bytes && std::fs::write(&p, &bytes).is_ok() {
            out.text = line(out.text, &format!("the local copy of {} was put back.", p.display()));
        }
    }
    if c == Command::DeleteAndIgnore && out.ok {
        out.text = line(out.text, &ignore(tool, dir, names));
    }
    out
}

/// One more line of a window's words, without gluing it to the line before it.
fn line(mut text: String, more: &str) -> String {
    if more.is_empty() {
        return text;
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(more);
    text
}

/// The tool's own record of names to leave alone (M15): one line in the file it reads, or the
/// folder's property where the tool keeps a property instead of a file. A name the record already
/// holds is not written a second time.
fn ignore(tool: Tool, dir: &Path, names: &[PathBuf]) -> String {
    let Some(file) = Command::ignore_file(tool) else {
        return names
            .iter()
            .map(|n| {
                let argv = Command::ignore_command(tool, ".", &n.display().to_string());
                if argv.is_empty() {
                    return format!("{} keeps no ignore record this name could go into.", tool.label());
                }
                client::call(tool, dir, &argv).text
            })
            .collect::<Vec<_>>()
            .join("\n");
    };
    let path = dir.join(file);
    let known = std::fs::read_to_string(&path).unwrap_or_default();
    let mut text = String::new();
    if !known.is_empty() && !known.ends_with('\n') {
        text.push('\n');
    }
    let mut words = Vec::new();
    for n in names {
        let name = n.display().to_string();
        if known.lines().chain(text.lines()).any(|l| l.trim() == name) {
            words.push(format!("{name} was already in {file}."));
            continue;
        }
        text.push_str(&name);
        text.push('\n');
        words.push(format!("{name} is in {file}."));
    }
    if text.is_empty() {
        return words.join("\n");
    }
    match appended(&path, &text) {
        Ok(()) => words.join("\n"),
        Err(e) => format!("{file} could not be written: {e}"),
    }
}

/// Appends what the ignore record is missing, keeping one name to a line.
fn appended(path: &Path, text: &str) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(text.as_bytes())
}

/// The commands whose answer can change what the folder holds, so its listing is read again once
/// the client has answered (M15).
fn changes_the_folder(c: Command) -> bool {
    matches!(c, Command::Commit | Command::Pull | Command::Push | Command::Sync | Command::Stash | Command::SwitchCheckout | Command::Revert | Command::Rename | Command::Delete | Command::DeleteKeepLocal | Command::DeleteAndIgnore | Command::Clone)
}

/// The tools that can clone into a folder no tool owns (M15): one list for the form's rows and for
/// what a scenario may name.
fn clone_tools() -> Vec<Tool> {
    Tool::all().iter().copied().filter(|t| Command::Clone.supports(*t)).collect()
}

/// The Clone form (M15). No marker answers which tool is to clone here, and the folder the clone
/// makes is the address's own name until a name of its own is typed into the field.
fn ask_clone(w: &Window, dir: &Path) {
    let d = gtk::MessageDialog::new(Some(&w.win), gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT, gtk::MessageType::Question, gtk::ButtonsType::None, CLONE_TITLE);
    d.set_secondary_text(Some(CLONE_HINT));
    d.add_button("_Cancel", gtk::ResponseType::Cancel);
    d.add_button("_Clone", gtk::ResponseType::Ok);
    d.set_default_response(gtk::ResponseType::Ok);
    let address = gtk::Entry::new();
    let folder = gtk::Entry::new();
    let tool = gtk::ComboBoxText::new();
    for t in clone_tools() {
        tool.append(Some(t.cli()), t.label());
    }
    tool.set_active(Some(0));
    for e in [&address, &folder] {
        e.set_width_chars(46);
    }
    folder.set_placeholder_text(Some("the name the address ends with"));
    if let Ok(area) = d.message_area().downcast::<gtk::Box>() {
        for e in [&address, &folder] {
            area.pack_start(e, false, false, 0);
        }
        area.pack_start(&tool, false, false, 0);
    }
    let suggested = Rc::new(RefCell::new(String::new()));
    {
        let (f, suggested) = (folder.clone(), suggested.clone());
        address.connect_changed(move |a| {
            let next = suggested_folder(&a.text());
            let shown = f.text().to_string();
            if next.is_empty() || (!shown.is_empty() && shown != *suggested.borrow()) {
                return;
            }
            *suggested.borrow_mut() = next.clone();
            f.set_text(&next);
        });
    }
    CLONE.with(|c| *c.borrow_mut() = Some(Rc::new(CloneForm { address: address.clone(), folder: folder.clone(), tool: tool.clone() })));
    address.grab_focus();
    let weak = Rc::downgrade(&w.me());
    let dir = dir.to_path_buf();
    let answered = Cell::new(false);
    d.connect_response(move |d, resp| {
        CLONE.with(|c| *c.borrow_mut() = None);
        if answered.replace(true) {
            return;
        }
        let (url, leaf) = (address.text().to_string(), folder.text().to_string());
        let picked = tool.active().and_then(|i| clone_tools().get(i as usize).copied());
        // Gone before the answer window shows: closing a message dialog only queues a delete event,
        // which GTK drops while another window takes over.
        unsafe { d.destroy() };
        if resp != gtk::ResponseType::Ok || url.trim().is_empty() {
            return;
        }
        let (Some(tool), Some(w)) = (picked, weak.upgrade()) else { return };
        let target = if leaf.trim().is_empty() { dir.clone() } else { dir.join(leaf.trim()) };
        let url = url.trim().to_string();
        // The clone runs where the new folder is to sit; the client names the folder itself.
        let what = Request { url: Some(&url), ..Default::default() };
        answer(&w, Command::Clone, tool, &dir, &[target.as_path()], &what);
    });
    d.show_all();
}

/// The folder an address names at its end: `…/repo.git`, `…/repo/` and `host:repo` all say `repo`
/// (M15). An address that ends in a server alone suggests nothing, since no folder is named in it.
fn suggested_folder(address: &str) -> String {
    let tail = address.trim_end_matches(['/', '\\']);
    let name = tail.rsplit(['/', '\\']).next().unwrap_or_default();
    match name.rsplit(':').next() {
        Some(n) if !n.is_empty() && n != tail => n.trim_end_matches(".git").to_string(),
        _ => String::new(),
    }
}
