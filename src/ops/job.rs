//! Shared state between a running file operation and whoever shows it.
//!
//! Workers update atomics (cheap, no locking per chunk); the UI samples them on a timer.
//! When a worker needs a decision it blocks in [`JobCtx::ask`] until the UI answers.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Preparing = 0,
    Working = 1,
    /// Waiting for a slow device to write back what is still cached.
    Flushing = 2,
    Done = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    /// `dst` exists already.
    Conflict { src: PathBuf, dst: PathBuf, src_size: u64, dst_size: u64, src_mtime: i64, dst_mtime: i64, dst_is_dir: bool },
    /// Something failed; go on without it?
    Error { path: PathBuf, message: String },
    /// Moving to the trash is not possible here.
    TrashUnsupported { path: PathBuf, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Replace,
    Skip,
    /// Keep both: the new one gets a free name.
    KeepBoth,
    Retry,
    /// Delete permanently (for [`Question::TrashUnsupported`]).
    DeleteInstead,
    Cancel,
}

/// The answer plus whether it applies to all further questions of the same kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reply {
    pub answer: Answer,
    pub for_all: bool,
}

pub struct PendingQuestion {
    pub question: Question,
    reply: mpsc::Sender<Reply>,
}

impl PendingQuestion {
    pub fn answer(self, reply: Reply) {
        let _ = self.reply.send(reply);
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub current: String,
}

#[derive(Default)]
pub struct JobCtx {
    cancelled: AtomicBool,
    phase: AtomicU8,
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    current: Mutex<String>,
    pending: Mutex<Option<PendingQuestion>>,
    /// Serializes questions: one dialog at a time even with many workers.
    asking: Mutex<()>,
    conflict_for_all: Mutex<Option<Answer>>,
    error_for_all: Mutex<Option<Answer>>,
    trash_for_all: Mutex<Option<Answer>>,
    errors: Mutex<Vec<String>>,
    /// Test hook: answers questions without a UI.
    auto_answer: Mutex<Option<Box<dyn Fn(&Question) -> Reply + Send>>>,
}

impl JobCtx {
    pub fn new() -> Arc<JobCtx> {
        Arc::new(JobCtx::default())
    }

    /// A context that answers every question with `f` (tests, headless use).
    pub fn with_answers(f: impl Fn(&Question) -> Reply + Send + 'static) -> Arc<JobCtx> {
        let ctx = JobCtx::default();
        *ctx.auto_answer.lock().unwrap() = Some(Box::new(f));
        Arc::new(ctx)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        // Unblock a worker waiting for an answer.
        if let Some(p) = self.pending.lock().unwrap().take() {
            p.answer(Reply { answer: Answer::Cancel, for_all: true });
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn set_phase(&self, p: Phase) {
        self.phase.store(p as u8, Ordering::SeqCst);
    }

    pub fn phase(&self) -> Phase {
        match self.phase.load(Ordering::SeqCst) {
            0 => Phase::Preparing,
            1 => Phase::Working,
            2 => Phase::Flushing,
            _ => Phase::Done,
        }
    }

    pub fn set_current(&self, s: impl Into<String>) {
        *self.current.lock().unwrap() = s.into();
    }

    pub fn add_bytes(&self, n: u64) {
        self.bytes_done.fetch_add(n, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
            bytes_total: self.bytes_total.load(Ordering::Relaxed),
            files_done: self.files_done.load(Ordering::Relaxed),
            files_total: self.files_total.load(Ordering::Relaxed),
            current: self.current.lock().unwrap().clone(),
        }
    }

    pub fn record_error(&self, msg: impl Into<String>) {
        self.errors.lock().unwrap().push(msg.into());
    }

    pub fn errors(&self) -> Vec<String> {
        self.errors.lock().unwrap().clone()
    }

    /// The question waiting for the UI, if any.
    pub fn take_question(&self) -> Option<PendingQuestion> {
        self.pending.lock().unwrap().take()
    }

    pub fn has_question(&self) -> bool {
        self.pending.lock().unwrap().is_some()
    }

    fn remembered(&self, q: &Question) -> &Mutex<Option<Answer>> {
        match q {
            Question::Conflict { .. } => &self.conflict_for_all,
            Question::Error { .. } => &self.error_for_all,
            Question::TrashUnsupported { .. } => &self.trash_for_all,
        }
    }

    /// Asks the user and blocks until answered. "For all" answers are remembered and
    /// returned without asking again. A cancelled job answers `Cancel`.
    pub fn ask(&self, q: Question) -> Answer {
        if self.is_cancelled() {
            return Answer::Cancel;
        }
        let _one_at_a_time = self.asking.lock().unwrap();
        if let Some(a) = *self.remembered(&q).lock().unwrap() {
            return a;
        }
        let reply = if let Some(f) = self.auto_answer.lock().unwrap().as_ref() {
            f(&q)
        } else {
            let (tx, rx) = mpsc::channel();
            *self.pending.lock().unwrap() = Some(PendingQuestion { question: q.clone(), reply: tx });
            if self.is_cancelled() {
                self.pending.lock().unwrap().take();
                return Answer::Cancel;
            }
            rx.recv().unwrap_or(Reply { answer: Answer::Cancel, for_all: true })
        };
        if reply.answer == Answer::Cancel {
            self.cancel();
        } else if reply.for_all && reply.answer != Answer::Retry {
            *self.remembered(&q).lock().unwrap() = Some(reply.answer);
        }
        reply.answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn remembered_answers_are_not_asked_again() {
        let asked = Arc::new(AtomicUsize::new(0));
        let a2 = asked.clone();
        let ctx = JobCtx::with_answers(move |_| {
            a2.fetch_add(1, Ordering::SeqCst);
            Reply { answer: Answer::Skip, for_all: true }
        });
        let q = || Question::Error { path: "/x".into(), message: "m".into() };
        assert_eq!(ctx.ask(q()), Answer::Skip);
        assert_eq!(ctx.ask(q()), Answer::Skip);
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        // Other kinds of questions are still asked.
        let c = Question::TrashUnsupported { path: "/x".into(), message: String::new() };
        ctx.ask(c);
        assert_eq!(asked.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn ui_answers_through_pending_question() {
        let ctx = JobCtx::new();
        let c2 = ctx.clone();
        let t = std::thread::spawn(move || c2.ask(Question::Error { path: "/y".into(), message: "boom".into() }));
        let pending = loop {
            if let Some(p) = ctx.take_question() {
                break p;
            }
            std::thread::yield_now();
        };
        assert!(matches!(pending.question, Question::Error { .. }));
        pending.answer(Reply { answer: Answer::Retry, for_all: false });
        assert_eq!(t.join().unwrap(), Answer::Retry);
    }

    #[test]
    fn cancel_unblocks_waiting_worker() {
        let ctx = JobCtx::new();
        let c2 = ctx.clone();
        let t = std::thread::spawn(move || c2.ask(Question::Error { path: "/y".into(), message: String::new() }));
        while !ctx.has_question() {
            std::thread::yield_now();
        }
        ctx.cancel();
        assert_eq!(t.join().unwrap(), Answer::Cancel);
        assert!(ctx.is_cancelled());
    }
}
