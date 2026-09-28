//! Scripted UI self-test. With `FB_SELFTEST=<script>` the first window runs the script's
//! steps through the real actions, dialogs and jobs, then exits with 0 on success.
//! Used by the UI tests (in a headless compositor) and to take the README screenshots.
//!
//! Script lines: `open <path>`, `tab <path>`, `select <name>…`, `action <name> [arg]`,
//! `answer <replace|skip|keepboth|cancel|delete>`, `text <reply for the next text prompt>`,
//! `wait-idle`, `wait-exists <path>`, `wait-missing <path>`, `expect-rows <n>`,
//! `expect-selected <name>`, `expect-location <text>`, `screenshot <file>`, `sleep <ms>`,
//! `size <w> <h>`, `expect-tree-root <path>`, `wait-tree-selected <path>`, `tree-click <path>`,
//! `wait-cell <column> <row> = <text>`, `quit`. Blank lines and `#` comments are ignored.

use super::app::AppCtx;
use super::window::Window;
use failbrauwser::location::Location;
use failbrauwser::ops::job::Answer;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static ANSWER: Cell<Option<Answer>> = const { Cell::new(None) };
    static TEXT: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
}

pub fn active() -> bool {
    ACTIVE.with(Cell::get)
}

/// The scripted answer for job questions, if the script set one.
pub fn scripted_answer() -> Option<Answer> {
    ANSWER.with(Cell::get)
}

/// The scripted reply for the next text prompt.
pub fn scripted_text() -> Option<String> {
    TEXT.with(|t| t.borrow_mut().pop_front())
}

pub fn script_path() -> Option<String> {
    std::env::var("FB_SELFTEST").ok().filter(|s| !s.is_empty())
}

struct Runner {
    steps: VecDeque<(usize, String)>,
    window: Rc<Window>,
    app: Rc<AppCtx>,
    waiting: Option<(Instant, Box<dyn Fn(&Runner) -> Result<bool, String>>)>,
    /// The step being run or waited for, for failure messages.
    current: (usize, String),
}

fn fail(line: usize, msg: &str) -> ! {
    eprintln!("SELFTEST FAIL (line {line}): {msg}");
    std::process::exit(1);
}

pub fn start(app: &Rc<AppCtx>, window: &Rc<Window>) {
    let Some(path) = script_path() else { return };
    ACTIVE.with(|a| a.set(true));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("SELFTEST FAIL: cannot read {path}: {e}");
        std::process::exit(2);
    });
    let steps = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let runner = Rc::new(RefCell::new(Runner { steps, window: window.clone(), app: app.clone(), waiting: None, current: (0, String::new()) }));
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let mut r = runner.borrow_mut();
        if let Some((since, check)) = r.waiting.take() {
            match check(&r) {
                Ok(true) => {}
                Ok(false) if since.elapsed() < Duration::from_secs(20) => {
                    r.waiting = Some((since, check));
                    return glib::ControlFlow::Continue;
                }
                Ok(false) => fail(r.current.0, &format!("{}: timed out", r.current.1)),
                Err(e) => fail(r.current.0, &format!("{}: {e}", r.current.1)),
            }
        }
        let Some((line, step)) = r.steps.pop_front() else {
            println!("SELFTEST OK");
            std::process::exit(0);
        };
        r.current = (line, step.clone());
        if let Err(e) = run_step(&mut r, &step) {
            fail(line, &format!("{step}: {e}"));
        }
        glib::ControlFlow::Continue
    });
}

fn idle(r: &Runner) -> bool {
    !r.app.jobs.is_busy() && r.window.panes().iter().all(|p| !p.is_loading() && !p.refresh_pending())
}

fn run_step(r: &mut Runner, step: &str) -> Result<(), String> {
    let (cmd, arg) = step.split_once(' ').map(|(c, a)| (c, a.trim())).unwrap_or((step, ""));
    let w = r.window.clone();
    match cmd {
        "open" => {
            let loc = Location::parse(arg, failbrauwser::archive::is_browsable_name).ok_or("no such location")?;
            w.navigate(loc);
            wait(r, |r| Ok(idle(r)));
        }
        "tab" => {
            let loc = Location::parse(arg, failbrauwser::archive::is_browsable_name).ok_or("no such location")?;
            w.add_tab(loc, true);
            wait(r, |r| Ok(idle(r)));
        }
        "select" => {
            let names: Vec<OsString> = arg.split(" | ").map(OsString::from).collect();
            w.current_pane().select_names(&names);
            let got = w.current_pane().selected_items().len();
            if got != names.len() {
                return Err(format!("selected {got} of {}", names.len()));
            }
        }
        "action" => {
            let (name, param) = arg.split_once(' ').map(|(n, p)| (n, Some(p))).unwrap_or((arg, None));
            let variant = param.map(|p| p.to_variant());
            let group: Option<gio::ActionGroup> = if let Some(n) = name.strip_prefix("win.") {
                w.win.lookup_action(n).map(|_| w.win.clone().upcast())
            } else if let Some(n) = name.strip_prefix("app.") {
                r.app.app.lookup_action(n).map(|_| r.app.app.clone().upcast())
            } else {
                None
            };
            let group = group.ok_or("unknown action")?;
            let short = name.split_once('.').map(|x| x.1).unwrap_or(name);
            if !group.is_action_enabled(short) {
                return Err("action is disabled".into());
            }
            group.activate_action(short, variant.as_ref());
        }
        "answer" => {
            let a = match arg {
                "replace" => Answer::Replace,
                "skip" => Answer::Skip,
                "keepboth" => Answer::KeepBoth,
                "delete" => Answer::DeleteInstead,
                "retry" => Answer::Retry,
                _ => Answer::Cancel,
            };
            ANSWER.with(|x| x.set(Some(a)));
        }
        "text" => TEXT.with(|t| t.borrow_mut().push_back(arg.to_string())),
        "wait-idle" => wait(r, |r| Ok(idle(r))),
        "wait-exists" => {
            let p = arg.to_string();
            wait(r, move |r| Ok(Path::new(&p).exists() && idle(r)));
        }
        "wait-missing" => {
            let p = arg.to_string();
            wait(r, move |r| Ok(!Path::new(&p).exists() && idle(r)));
        }
        "expect-rows" => {
            let n: usize = arg.parse().map_err(|_| "bad number")?;
            let got = w.current_pane().item_count();
            if got != n {
                return Err(format!("{got} rows"));
            }
        }
        "expect-selected" => {
            let sel: Vec<String> = w.current_pane().selected_items().iter().map(|i| i.display_name()).collect();
            if sel != [arg.to_string()] {
                return Err(format!("selection is {sel:?}"));
            }
        }
        "expect-location" => {
            let loc = w.current_location().display();
            if loc != arg {
                return Err(format!("location is {loc}"));
            }
        }
        "wait-cell" => {
            // wait-cell <column> <row name> = <text>
            let (col, rest) = arg.split_once(' ').ok_or("usage: wait-cell <column> <name> = <text>")?;
            let (name, want) = rest.split_once(" =").ok_or("usage: wait-cell <column> <name> = <text>")?;
            let want = want.trim();
            let column = match col {
                "size" => super::model::COL_SIZE_TEXT,
                "dirsize" => super::model::COL_DIRSIZE_TEXT,
                "type" => super::model::COL_TYPE,
                "modified" => super::model::COL_MTIME_TEXT,
                _ => return Err("unknown column".into()),
            };
            let (name, want) = (name.to_string(), want.to_string());
            wait(r, move |r| Ok(r.window.current_pane().cell_text(&name, column).as_deref() == Some(want.as_str())));
        }
        "expect-tree-root" => {
            let sb = super::sidebar::for_window(&w).ok_or("no sidebar")?;
            let root = sb.root_path().map(|p| p.display().to_string()).unwrap_or_default();
            if root != arg {
                return Err(format!("tree root is {root}"));
            }
        }
        "wait-tree-selected" => {
            let want = arg.to_string();
            wait(r, move |r| {
                let sb = super::sidebar::for_window(&r.window).ok_or("no sidebar")?;
                Ok(sb.selected_path().is_some_and(|p| p.display().to_string() == want))
            });
        }
        "tree-click" => {
            let sb = super::sidebar::for_window(&w).ok_or("no sidebar")?;
            if !sb.click(Path::new(arg)) {
                return Err("folder not in tree".into());
            }
            wait(r, |r| Ok(idle(r)));
        }
        "screenshot" => {
            let file = arg.to_string();
            // Let GTK paint first.
            let status = std::process::Command::new("grim").arg(&file).status().map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("grim failed".into());
            }
        }
        "sleep" => {
            let ms: u64 = arg.parse().map_err(|_| "bad number")?;
            let until = Instant::now() + Duration::from_millis(ms);
            wait(r, move |_| Ok(Instant::now() >= until));
        }
        "size" => {
            let mut it = arg.split_whitespace().filter_map(|x| x.parse::<i32>().ok());
            let (Some(x), Some(y)) = (it.next(), it.next()) else { return Err("need width height".into()) };
            w.win.resize(x, y);
        }
        "quit" => {
            println!("SELFTEST OK");
            std::process::exit(0);
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}

fn wait(r: &mut Runner, check: impl Fn(&Runner) -> Result<bool, String> + 'static) {
    r.waiting = Some((Instant::now(), Box::new(check)));
}
