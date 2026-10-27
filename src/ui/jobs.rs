//! Runs file operations on worker threads and shows them: a progress window that appears
//! only for operations taking longer than a moment, and dialogs for the questions they ask.
//! Copies show how far they are, a chart of their speed (in the window manager's chart
//! colors), the item being copied, the time and the items left.
//! A timer runs only while operations are active, so an idle window costs no CPU.

use super::speedchart::{self, SpeedChart};
use super::util;
use failbrauwser::wmtheme::ChartStyle;
use failbrauwser::fs::format::{human_size, human_time};
use failbrauwser::ops::fastcopy::is_cancelled_error;
use failbrauwser::ops::job::{Answer, JobCtx, Phase, PendingQuestion, Question, Reply};
use gtk::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What a finished operation reports back.
pub enum Outcome {
    /// Finished; the paths it created (to select them).
    Done(Vec<PathBuf>),
    Cancelled,
    Failed(String),
}

type Work = Box<dyn FnOnce(&JobCtx) -> std::io::Result<Vec<PathBuf>> + Send>;
type OnDone = Box<dyn FnOnce(&Outcome)>;

struct Job {
    ctx: Arc<JobCtx>,
    title: String,
    parent: glib::WeakRef<gtk::Window>,
    started: Instant,
    result: Arc<Mutex<Option<std::io::Result<Vec<PathBuf>>>>>,
    on_done: Option<OnDone>,
    row: Option<JobRow>,
    asking: bool,
    /// For the speed estimate: (time, bytes) a moment ago.
    last_sample: (Instant, u64),
    speed: f64,
    /// Operations with the same key (the target drive) run one after another.
    queue_key: Option<u64>,
    /// Not started yet: waiting for the operation ahead on the same drive.
    waiting: Option<Work>,
}

struct JobRow {
    widget: gtk::Box,
    /// "45% complete", "Paused", "Preparing…".
    status: gtk::Label,
    /// For operations that count items, not bytes (delete, trash).
    bar: gtk::ProgressBar,
    /// The chart and the lines below it, for operations that move bytes.
    details: gtk::Expander,
    chart: SpeedChart,
    name: gtk::Label,
    time_left: gtk::Label,
    items_left: gtk::Label,
    pause: gtk::ToggleButton,
}

thread_local! {
    /// Self-test: new operations start paused.
    static START_PAUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn set_start_paused(on: bool) {
    START_PAUSED.with(|s| s.set(on));
}

pub struct Jobs {
    jobs: RefCell<Vec<Job>>,
    window: RefCell<Option<(gtk::Window, gtk::Box)>>,
    /// The window manager's chart colors, read each time the window shows (None: the GTK
    /// theme's).
    chart_style: Rc<RefCell<Option<ChartStyle>>>,
    ticking: RefCell<Option<glib::SourceId>>,
    weak: RefCell<Weak<Jobs>>,
}

/// Operations quicker than this never show the progress window.
const SHOW_AFTER: Duration = Duration::from_millis(500);

impl Jobs {
    pub fn new() -> Rc<Jobs> {
        let j = Rc::new(Jobs { jobs: RefCell::new(Vec::new()), window: RefCell::new(None), chart_style: Rc::new(RefCell::new(None)), ticking: RefCell::new(None), weak: RefCell::new(Weak::new()) });
        *j.weak.borrow_mut() = Rc::downgrade(&j);
        j
    }

    pub fn is_busy(&self) -> bool {
        !self.jobs.borrow().is_empty()
    }

    /// (running, waiting) operations (tests).
    pub fn counts(&self) -> (usize, usize) {
        let jobs = self.jobs.borrow();
        let waiting = jobs.iter().filter(|j| j.waiting.is_some()).count();
        (jobs.len() - waiting, waiting)
    }

    /// The progress window while it is shown (tests).
    pub fn progress_window(&self) -> Option<gtk::Window> {
        self.window.borrow().as_ref().map(|(w, _)| w.clone()).filter(|w| w.is_visible())
    }

    /// The most points a speed chart has drawn (tests).
    pub fn chart_points(&self) -> usize {
        self.jobs.borrow().iter().filter_map(|j| j.row.as_ref()).map(|r| if r.details.is_visible() { r.chart.samples.borrow().points.len() } else { 0 }).max().unwrap_or(0)
    }

    pub fn pause_all(&self, on: bool) {
        for j in self.jobs.borrow().iter() {
            if on { j.ctx.pause() } else { j.ctx.resume() }
        }
    }

    /// Starts `work` on a new thread. `on_done` runs on the UI thread afterwards.
    pub fn start(
        &self,
        parent: &gtk::Window,
        title: impl Into<String>,
        work: impl FnOnce(&JobCtx) -> std::io::Result<Vec<PathBuf>> + Send + 'static,
        on_done: impl FnOnce(&Outcome) + 'static,
    ) {
        self.start_queued(parent, title, None, work, on_done);
    }

    /// Like [`Jobs::start`], but waits while another operation with the same key runs —
    /// two copies to one USB stick at once only make both slower.
    pub fn start_queued(
        &self,
        parent: &gtk::Window,
        title: impl Into<String>,
        queue_key: Option<u64>,
        work: impl FnOnce(&JobCtx) -> std::io::Result<Vec<PathBuf>> + Send + 'static,
        on_done: impl FnOnce(&Outcome) + 'static,
    ) {
        let ctx = JobCtx::new();
        if START_PAUSED.with(|s| s.get()) {
            ctx.pause();
        }
        let result = Arc::new(Mutex::new(None));
        let work: Work = Box::new(work);
        let busy = queue_key.is_some() && self.jobs.borrow().iter().any(|j| j.queue_key == queue_key);
        let waiting = if busy {
            Some(work)
        } else {
            Self::spawn(&ctx, &result, work);
            None
        };
        self.jobs.borrow_mut().push(Job {
            ctx,
            title: title.into(),
            parent: parent.downgrade(),
            started: Instant::now(),
            result,
            on_done: Some(Box::new(on_done)),
            row: None,
            asking: false,
            last_sample: (Instant::now(), 0),
            speed: 0.0,
            queue_key,
            waiting,
        });
        self.ensure_ticking();
    }

    fn spawn(ctx: &Arc<JobCtx>, result: &Arc<Mutex<Option<std::io::Result<Vec<PathBuf>>>>>, work: Work) {
        let (c2, r2) = (ctx.clone(), result.clone());
        std::thread::Builder::new()
            .name("fb-job".into())
            .spawn(move || {
                let r = work(&c2);
                c2.set_phase(Phase::Done);
                *r2.lock().unwrap() = Some(r);
            })
            .expect("spawn job thread");
    }

    /// Starts waiting operations whose drive became free; cancelled waiting ones end.
    fn start_waiting(&self) {
        let mut jobs = self.jobs.borrow_mut();
        let running: Vec<Option<u64>> = jobs.iter().filter(|j| j.waiting.is_none()).map(|j| j.queue_key).collect();
        let mut started: Vec<Option<u64>> = Vec::new();
        for j in jobs.iter_mut() {
            if j.waiting.is_none() {
                continue;
            }
            if j.ctx.is_cancelled() {
                j.waiting = None;
                *j.result.lock().unwrap() = Some(Err(failbrauwser::ops::fastcopy::cancelled()));
                continue;
            }
            if running.contains(&j.queue_key) || started.contains(&j.queue_key) {
                continue;
            }
            let work = j.waiting.take().unwrap();
            j.started = Instant::now();
            Self::spawn(&j.ctx, &j.result, work);
            started.push(j.queue_key);
        }
    }

    fn ensure_ticking(&self) {
        if self.ticking.borrow().is_some() {
            return;
        }
        let weak = self.weak.borrow().clone();
        let id = glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(j) = weak.upgrade() else { return glib::ControlFlow::Break };
            if j.tick() {
                glib::ControlFlow::Continue
            } else {
                j.ticking.borrow_mut().take();
                glib::ControlFlow::Break
            }
        });
        *self.ticking.borrow_mut() = Some(id);
    }

    /// One timer step; false when nothing is left to watch.
    fn tick(&self) -> bool {
        self.start_waiting();
        let mut finished = Vec::new();
        let mut questions = Vec::new();
        {
            let mut jobs = self.jobs.borrow_mut();
            let mut i = 0;
            while i < jobs.len() {
                let done = jobs[i].result.lock().unwrap().is_some();
                if done {
                    finished.push(jobs.remove(i));
                    continue;
                }
                let job = &mut jobs[i];
                if !job.asking {
                    if let Some(q) = job.ctx.take_question() {
                        job.asking = true;
                        questions.push((job.ctx.clone(), job.parent.upgrade(), q));
                    }
                }
                i += 1;
            }
        }
        for (ctx, parent, q) in questions {
            self.ask(ctx, parent, q);
        }
        for mut job in finished {
            if let Some(row) = job.row.take() {
                if let Some(parent) = row.widget.parent().and_then(|p| p.downcast::<gtk::Container>().ok()) {
                    parent.remove(&row.widget);
                }
            }
            let res = job.result.lock().unwrap().take().unwrap();
            let outcome = match res {
                Ok(paths) => Outcome::Done(paths),
                Err(e) if is_cancelled_error(&e) || job.ctx.is_cancelled() => Outcome::Cancelled,
                Err(e) => Outcome::Failed(e.to_string()),
            };
            let errors = job.ctx.errors();
            if let Some(parent) = job.parent.upgrade() {
                match &outcome {
                    // A missing password is asked for by the operation itself.
                    Outcome::Failed(msg) if msg.starts_with(super::archive::PASSWORD_MARK) => {}
                    Outcome::Failed(msg) => util::show_error(&parent, &format!("{} failed", job.title), msg),
                    _ if !errors.is_empty() => {
                        let mut text = errors.iter().take(15).cloned().collect::<Vec<_>>().join("\n");
                        if errors.len() > 15 {
                            text.push_str(&format!("\n… and {} more", errors.len() - 15));
                        }
                        util::show_error(&parent, &format!("{}: some items were skipped", job.title), &text);
                    }
                    _ => {}
                }
            }
            if let Some(f) = job.on_done.take() {
                f(&outcome);
            }
        }
        self.update_window();
        !self.jobs.borrow().is_empty()
    }

    fn update_window(&self) {
        let now = Instant::now();
        let show = self.jobs.borrow().iter().any(|j| now - j.started >= SHOW_AFTER && !j.asking);
        if self.jobs.borrow().is_empty() {
            if let Some((w, _)) = self.window.borrow().as_ref() {
                w.hide();
            }
            return;
        }
        if !show && self.window.borrow().as_ref().is_none_or(|(w, _)| !w.is_visible()) {
            return;
        }
        self.ensure_window();
        let (win, list) = self.window.borrow().clone().unwrap();
        if !win.is_visible() {
            // The window manager's theme may have changed since.
            let under = speedchart::window_bg(&win);
            *self.chart_style.borrow_mut() = failbrauwser::wmtheme::load().map(|t| ChartStyle::from_theme(&t, under));
        }
        let mut jobs = self.jobs.borrow_mut();
        for job in jobs.iter_mut() {
            if job.row.is_none() {
                job.row = Some(Self::make_row(&list, job, &self.chart_style));
            }
            Self::update_row(job, now);
        }
        if !win.is_visible() {
            if let Some(p) = jobs.first().and_then(|j| j.parent.upgrade()) {
                win.set_transient_for(Some(&p));
            }
            win.show_all();
        }
    }

    fn ensure_window(&self) {
        if self.window.borrow().is_some() {
            return;
        }
        let win = gtk::Window::new(gtk::WindowType::Toplevel);
        win.set_title("File Operation Progress");
        win.set_type_hint(gtk::gdk::WindowTypeHint::Dialog);
        win.set_default_size(500, -1);
        win.set_resizable(false);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 12);
        list.set_border_width(12);
        win.add(&list);
        // Closing the window only hides it; operations keep running.
        win.connect_delete_event(|w, _| {
            w.hide();
            glib::Propagation::Stop
        });
        *self.window.borrow_mut() = Some((win, list));
    }

    fn make_row(list: &gtk::Box, job: &Job, style: &Rc<RefCell<Option<ChartStyle>>>) -> JobRow {
        let left = |text: &str| {
            let l = gtk::Label::new(Some(text));
            l.set_xalign(0.0);
            l
        };
        let title = left(&job.title);
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.style_context().add_class("dim-label");
        let status = left("");
        let attrs = gtk::pango::AttrList::new();
        attrs.insert(gtk::pango::AttrFloat::new_scale(1.25));
        status.set_attributes(Some(&attrs));
        let bar = gtk::ProgressBar::new();
        bar.set_show_text(true);
        bar.set_ellipsize(gtk::pango::EllipsizeMode::End);
        bar.set_no_show_all(true);
        let cancel = gtk::Button::from_icon_name(Some("process-stop"), gtk::IconSize::Button);
        cancel.set_tooltip_text(Some("Cancel"));
        cancel.set_valign(gtk::Align::Center);
        let ctx = job.ctx.clone();
        cancel.connect_clicked(move |b| {
            ctx.cancel();
            b.set_sensitive(false);
        });
        let pause = gtk::ToggleButton::new();
        pause.set_image(Some(&gtk::Image::from_icon_name(Some("media-playback-pause"), gtk::IconSize::Button)));
        pause.set_tooltip_text(Some("Pause"));
        pause.set_valign(gtk::Align::Center);
        pause.set_active(job.ctx.is_paused());
        let ctx = job.ctx.clone();
        pause.connect_toggled(move |b| {
            if b.is_active() {
                ctx.pause();
                b.set_tooltip_text(Some("Continue"));
            } else {
                ctx.resume();
                b.set_tooltip_text(Some("Pause"));
            }
        });
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 2);
        texts.pack_start(&title, false, false, 0);
        texts.pack_start(&status, false, false, 0);
        head.pack_start(&texts, true, true, 0);
        head.pack_start(&pause, false, false, 0);
        head.pack_start(&cancel, false, false, 0);
        // The chart, then name, time and items left, as in a two-column form.
        let chart = SpeedChart::new(style.clone());
        let facts = gtk::Grid::new();
        facts.set_column_spacing(12);
        facts.set_row_spacing(2);
        let fact = |row: i32, key: &str| {
            let k = left(key);
            k.style_context().add_class("dim-label");
            let v = left("");
            v.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            v.set_hexpand(true);
            facts.attach(&k, 0, row, 1, 1);
            facts.attach(&v, 1, row, 1, 1);
            v
        };
        let name = fact(0, "Name:");
        let time_left = fact(1, "Time remaining:");
        let items_left = fact(2, "Items remaining:");
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.set_margin_top(6);
        inner.pack_start(&chart.area, false, false, 0);
        inner.pack_start(&facts, false, false, 0);
        let details = gtk::Expander::new(Some("Details"));
        details.set_expanded(true);
        details.add(&inner);
        details.set_no_show_all(true);
        inner.show_all();
        let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
        row.pack_start(&head, false, false, 0);
        row.pack_start(&bar, false, false, 0);
        row.pack_start(&details, false, false, 0);
        list.pack_start(&row, false, false, 0);
        row.show_all();
        JobRow { widget: row, status, bar, details, chart, name, time_left, items_left, pause }
    }

    fn update_row(job: &mut Job, now: Instant) {
        let Some(row) = &job.row else { return };
        let s = job.ctx.snapshot();
        let dt = now.duration_since(job.last_sample.0).as_secs_f64();
        let sampled = dt >= 0.5;
        if sampled {
            let inst = s.bytes_done.saturating_sub(job.last_sample.1) as f64 / dt;
            job.speed = if job.speed == 0.0 { inst } else { job.speed * 0.7 + inst * 0.3 };
            job.last_sample = (now, s.bytes_done);
        }
        let by_bytes = s.bytes_total > 0;
        row.details.set_visible(by_bytes);
        row.bar.set_visible(!by_bytes);
        if job.waiting.is_some() {
            row.status.set_text("Waiting for the operation before it on this drive");
            row.bar.set_fraction(0.0);
            row.bar.set_text(None);
            row.pause.set_sensitive(false);
            return;
        }
        row.pause.set_sensitive(true);
        let fraction = if by_bytes {
            (s.bytes_done as f64 / s.bytes_total as f64).clamp(0.0, 1.0)
        } else if s.files_total > 0 {
            (s.files_done as f64 / s.files_total as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let percent = format!("{}% complete", (fraction * 100.0).floor() as u32);
        if job.ctx.is_paused() {
            row.status.set_text(&format!("Paused — {percent}"));
            row.pause.set_active(true);
            row.time_left.set_text("—");
            return;
        }
        match job.ctx.phase() {
            Phase::Preparing => {
                row.status.set_text("Preparing…");
                row.bar.pulse();
                row.bar.set_text(None);
            }
            Phase::Flushing => {
                row.status.set_text("Writing cached data to the drive…");
                row.bar.pulse();
                row.time_left.set_text("A moment");
            }
            _ if by_bytes || s.files_total > 0 => {
                row.status.set_text(&percent);
                row.bar.set_fraction(fraction);
                row.bar.set_text(Some(&format!("{} of {} items", s.files_done, s.files_total)));
            }
            _ => {
                row.status.set_text("Working…");
                row.bar.pulse();
                row.bar.set_text(None);
            }
        }
        if by_bytes {
            if sampled && job.ctx.phase() == Phase::Working {
                row.chart.push(fraction, job.speed);
            }
            let left = s.bytes_total.saturating_sub(s.bytes_done);
            if job.ctx.phase() == Phase::Working {
                row.time_left.set_text(&if job.speed > 1.0 {
                    format!("About {}", human_duration((left as f64 / job.speed).ceil() as u64))
                } else {
                    "Working it out…".into()
                });
            }
            let items = s.files_total.saturating_sub(s.files_done);
            row.items_left.set_text(&format!("{items} ({})", human_size(left)));
        }
        row.name.set_text(&s.current);
        row.name.set_tooltip_text(Some(&s.current));
    }

    fn ask(&self, ctx: Arc<JobCtx>, parent: Option<gtk::Window>, pending: PendingQuestion) {
        let weak = self.weak.borrow().clone();
        let done = move || {
            if let Some(j) = weak.upgrade() {
                for job in j.jobs.borrow_mut().iter_mut() {
                    if Arc::ptr_eq(&job.ctx, &ctx) {
                        job.asking = false;
                    }
                }
            }
        };
        question_dialog(parent.as_ref(), pending, done);
    }
}

pub fn human_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{} min {} s", secs / 60, secs % 60),
        _ => format!("{} h {} min", secs / 3600, (secs % 3600) / 60),
    }
}

/// Shows the dialog for one question and hands the answer back to the worker.
fn question_dialog(parent: Option<&gtk::Window>, pending: PendingQuestion, done: impl Fn() + 'static) {
    if let Some(answer) = super::selftest::scripted_answer() {
        pending.answer(Reply { answer, for_all: true });
        done();
        return;
    }
    let now = glib::real_time() / 1_000_000;
    let name = |p: &std::path::Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
    let (title, detail, buttons): (String, String, Vec<(&str, Answer)>) = match &pending.question {
        Question::Conflict { src: _, dst, src_size, dst_size, src_mtime, dst_mtime, dst_is_dir } => {
            let folder = dst.parent().map(name).unwrap_or_default();
            let kind = if *dst_is_dir { "A folder" } else { "A file" };
            (
                format!("Replace “{}”?", name(dst)),
                format!(
                    "{kind} with this name already exists in “{folder}”.\n\nExisting: {}, modified {}\nNew: {}, modified {}",
                    human_size(*dst_size),
                    human_time(*dst_mtime, now),
                    human_size(*src_size),
                    human_time(*src_mtime, now)
                ),
                vec![("_Cancel", Answer::Cancel), ("_Skip", Answer::Skip), ("_Keep Both", Answer::KeepBoth), ("_Replace", Answer::Replace)],
            )
        }
        Question::Error { path, message } => (
            format!("Error with “{}”", name(path)),
            message.clone(),
            vec![("_Cancel", Answer::Cancel), ("_Skip", Answer::Skip), ("_Retry", Answer::Retry)],
        ),
        Question::TrashUnsupported { path, message } => (
            format!("“{}” cannot be moved to the trash", name(path)),
            format!("{message}\n\nDelete it permanently instead?"),
            vec![("_Cancel", Answer::Cancel), ("_Skip", Answer::Skip), ("_Delete", Answer::DeleteInstead)],
        ),
    };
    let d = gtk::MessageDialog::new(parent, gtk::DialogFlags::DESTROY_WITH_PARENT, gtk::MessageType::Question, gtk::ButtonsType::None, &title);
    d.set_secondary_text(Some(&detail));
    for (i, (label, _)) in buttons.iter().enumerate() {
        d.add_button(label, gtk::ResponseType::Other(i as u16));
    }
    d.set_default_response(gtk::ResponseType::Other(buttons.len() as u16 - 1));
    let all = gtk::CheckButton::with_mnemonic("_Apply to all");
    all.set_margin_start(12);
    if let Some(area) = d.message_area().downcast::<gtk::Box>().ok() {
        area.pack_start(&all, false, false, 0);
    }
    d.show_all();
    let pending = RefCell::new(Some(pending));
    d.connect_response(move |d, resp| {
        let answer = match resp {
            gtk::ResponseType::Other(i) => buttons.get(i as usize).map(|b| b.1).unwrap_or(Answer::Cancel),
            _ => Answer::Cancel,
        };
        if let Some(p) = pending.borrow_mut().take() {
            p.answer(Reply { answer, for_all: all.is_active() });
        }
        done();
        d.close();
    });
}
