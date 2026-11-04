//! Native document reads and preparation run away from the frame thread.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use crate::RecoveryStore;
use pdfcraft_engine::{OpenContext, PreparedDocument, SourceKind};
use pdfcraft_render::OpenError;

const MAX_QUEUED_OPENS: usize = 16;

pub(crate) enum Input {
    Path(PathBuf),
    RemotePath(PathBuf),
    Recovery(RecoveryStore, String),
    Bytes(Arc<Vec<u8>>),
}

pub(crate) enum ResultKind {
    Conversion(Arc<Vec<u8>>),
    Prepared { bytes: Arc<Vec<u8>>, result: Box<Result<PreparedDocument, OpenError>> },
    ReadFailed(String),
    RemoteFailed(String),
    RecoveryFailed(String),
    Failed(String),
}

pub(crate) struct ResultMessage {
    pub token: u64,
    pub kind: ResultKind,
}

struct Job {
    token: u64,
    context: OpenContext,
    name: String,
    path: Option<String>,
    password: Option<String>,
    input: Input,
    reply: Sender<ResultMessage>,
    #[cfg(test)]
    gate: Option<Arc<PrepareGate>>,
}

struct Queue {
    jobs: VecDeque<Job>,
    shutdown: bool,
    repaint: Option<egui::Context>,
}

pub(crate) struct OpenWorker {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    #[cfg(test)]
    gate: Option<Arc<PrepareGate>>,
}

impl OpenWorker {
    pub fn new(#[cfg(test)] gate: Option<Arc<PrepareGate>>) -> std::io::Result<Self> {
        let queue = Arc::new((Mutex::new(Queue { jobs: VecDeque::new(), shutdown: false, repaint: None }), Condvar::new()));
        let worker_queue = queue.clone();
        std::thread::Builder::new().name("pdfcraft-open".into()).spawn(move || run(worker_queue))?;
        Ok(Self {
            queue,
            #[cfg(test)]
            gate,
        })
    }

    pub fn set_context(&self, ctx: &egui::Context) {
        let (lock, _) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(PoisonError::into_inner);
        queue.repaint = Some(ctx.clone());
    }

    pub fn submit(
        &self,
        token: u64,
        context: OpenContext,
        name: String,
        path: Option<String>,
        password: Option<String>,
        input: Input,
    ) -> Result<Receiver<ResultMessage>, String> {
        let (reply, receiver) = mpsc::channel();
        let job = Job {
            token,
            context,
            name,
            path,
            password,
            input,
            reply,
            #[cfg(test)]
            gate: self.gate.clone(),
        };
        let (lock, ready) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(PoisonError::into_inner);
        if queue.shutdown {
            return Err("the document-open worker has stopped".into());
        }
        if queue.jobs.len() >= MAX_QUEUED_OPENS {
            return Err("too many documents are waiting to open".into());
        }
        queue.jobs.push_back(job);
        ready.notify_one();
        Ok(receiver)
    }
}

impl Drop for OpenWorker {
    fn drop(&mut self) {
        let (lock, ready) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(PoisonError::into_inner);
        queue.shutdown = true;
        queue.jobs.clear();
        queue.repaint = None;
        ready.notify_one();
    }
}

fn run(queue: Arc<(Mutex<Queue>, Condvar)>) {
    loop {
        let job = {
            let (lock, ready) = &*queue;
            let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
            while state.jobs.is_empty() && !state.shutdown {
                state = ready.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
            if state.shutdown {
                return;
            }
            state.jobs.pop_front()
        };
        let Some(job) = job else { continue };
        #[cfg(test)]
        if let Some(gate) = &job.gate {
            gate.pause_before_prepare();
        }
        let token = job.token;
        let reply = job.reply.clone();
        #[cfg(test)]
        let gate = job.gate.clone();
        let kind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(job)))
            .unwrap_or_else(|_| ResultKind::Failed("opening failed unexpectedly in the background worker".into()));
        let _ = reply.send(ResultMessage { token, kind });
        let repaint = {
            let (lock, _) = &*queue;
            lock.lock().unwrap_or_else(PoisonError::into_inner).repaint.clone()
        };
        if let Some(ctx) = repaint {
            ctx.request_repaint();
        }
        #[cfg(test)]
        if let Some(gate) = &gate {
            let _ = gate.finished.send(());
        }
    }
}

fn prepare(job: Job) -> ResultKind {
    let bytes = match &job.input {
        Input::Path(path) => match std::fs::read(path) {
            Ok(bytes) => Arc::new(bytes),
            Err(error) => return ResultKind::ReadFailed(read_error_text(&error)),
        },
        Input::RemotePath(path) => match pdfcraft_engine::read_remote_document(path) {
            Ok(bytes) => Arc::new(bytes),
            Err(error) => return ResultKind::RemoteFailed(error.to_string()),
        },
        Input::Recovery(store, key) => match store.read(key) {
            Ok(bytes) => Arc::new(bytes),
            Err(error) => return ResultKind::RecoveryFailed(error.to_string()),
        },
        Input::Bytes(bytes) => bytes.clone(),
    };
    if matches!(pdfcraft_engine::source_kind(&job.name, &bytes), Some(SourceKind::Image | SourceKind::Text)) {
        return ResultKind::Conversion(bytes);
    }
    let result = job.context.prepare_open(job.name, job.path, bytes.clone(), job.password.as_deref());
    ResultKind::Prepared { bytes, result: Box::new(result) }
}

/// Windows `ERROR_CLOUD_FILE_PROVIDER_NOT_RUNNING`: the file is a cloud placeholder (OneDrive
/// "Available when online") and the sync app that owns it is not connected.
const CLOUD_PROVIDER_NOT_RUNNING: i32 = 362;

/// Why a file could not be read, in words a person can act on. An online-only cloud file whose sync
/// app is not running cannot be read by any program (and the native Open dialog greys its Open
/// button for it); the OS message alone doesn't say what to do.
pub(crate) fn read_error_text(error: &std::io::Error) -> String {
    let text = error.to_string();
    if cfg!(windows) && error.raw_os_error() == Some(CLOUD_PROVIDER_NOT_RUNNING) {
        format!("{} Start OneDrive (or the app that syncs this file) and try again, or open a copy that is saved on this PC.", text.trim_end())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_cloud_provider_gets_an_actionable_message() {
        let stopped = std::io::Error::from_raw_os_error(CLOUD_PROVIDER_NOT_RUNNING);
        let text = read_error_text(&stopped);
        if cfg!(windows) {
            assert!(text.contains("Start OneDrive"), "{text}");
            assert!(text.starts_with(stopped.to_string().trim_end()), "{text}");
        } else {
            assert_eq!(text, stopped.to_string());
        }
    }

    #[test]
    fn other_read_errors_keep_the_system_message() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(read_error_text(&missing), missing.to_string());
        let denied = std::io::Error::from_raw_os_error(5);
        assert_eq!(read_error_text(&denied), denied.to_string());
    }
}

#[cfg(test)]
pub(crate) struct PrepareGate {
    pub started: Sender<()>,
    release: Mutex<Receiver<()>>,
    pub finished: Sender<()>,
}

#[cfg(test)]
impl PrepareGate {
    pub fn pause_before_prepare(&self) {
        let _ = self.started.send(());
        if let Ok(release) = self.release.lock() {
            let _ = release.recv();
        }
    }
}

#[cfg(test)]
pub(crate) fn prepare_gate() -> (Arc<PrepareGate>, Receiver<()>, Sender<()>, Receiver<()>) {
    let (started, started_rx) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let (finished, finished_rx) = mpsc::channel();
    (Arc::new(PrepareGate { started, release: Mutex::new(release), finished }), started_rx, release_tx, finished_rx)
}
