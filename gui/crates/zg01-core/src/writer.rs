//! The coalescing control writer.
//!
//! A write to the ZG01 can block: the control frame goes over USB and the
//! driver may wait for the device to answer. The GUI must not wait for it
//! on the GTK thread, so every write goes through a worker thread.
//!
//! The thread keeps the pending value of each control and flushes them
//! every [`FLUSH_INTERVAL`]: a drag over one control produces one write
//! per interval, not one write per mouse event. The last value of a
//! control always wins, and the outcome of every write is reported on a
//! channel so the GUI can revert an optimistic value when the write
//! failed.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::alsa::AlsaBackend;

/// How long the writer waits before it flushes the pending values.
const FLUSH_INTERVAL: Duration = Duration::from_millis(20);

/// How long the writer waits when it has nothing to do.
const IDLE_WAIT: Duration = Duration::from_millis(500);

/// How long [`ControlWriter::set_and_wait`] waits for the outcome.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// The outcome of one write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The control now holds the value.
    Ok {
        /// The name of the control.
        name: String,
        /// The value that was written.
        value: i64,
    },
    /// The write failed and the control keeps its previous value.
    Err {
        /// The name of the control.
        name: String,
        /// What the driver answered.
        error: String,
    },
}

/// A request for the writer thread.
enum Command {
    /// Queue a value; it may be coalesced with the next ones.
    Set {
        /// The name of the control.
        name: String,
        /// The value to write.
        value: i64,
    },
    /// Write a value now and wait for the outcome.
    SetAndWait {
        /// The name of the control.
        name: String,
        /// The value to write.
        value: i64,
        /// Where the outcome is sent back.
        reply: Sender<Result<(), String>>,
    },
}

/// Writes the controls of the device from a worker thread.
///
/// The writer is cheap to clone: every clone writes to the same thread.
#[derive(Clone)]
pub struct ControlWriter {
    commands: Sender<Command>,
}

impl ControlWriter {
    /// Starts the writer thread.
    ///
    /// The returned receiver reports the outcome of every write the thread
    /// performs.
    pub fn spawn(backend: Arc<dyn AlsaBackend>) -> (ControlWriter, Receiver<WriteOutcome>) {
        let (commands, inbox) = mpsc::channel::<Command>();
        let (outcomes, receiver) = mpsc::channel::<WriteOutcome>();
        std::thread::Builder::new()
            .name("zg01-writer".to_string())
            .spawn(move || run(backend, inbox, outcomes))
            .expect("could not start the writer thread");
        (ControlWriter { commands }, receiver)
    }

    /// Queues a value for the control named `name`.
    ///
    /// The write is fire-and-forget: it may be coalesced with the values
    /// that follow it, and the outcome is reported on the channel of
    /// [`ControlWriter::spawn`]. The caller never waits for the device.
    pub fn set(&self, name: &str, value: i64) {
        // The channel is unbounded, so this never blocks the GTK thread.
        let _ = self.commands.send(Command::Set {
            name: name.to_string(),
            value,
        });
    }

    /// Writes a value to the control named `name` and waits for it.
    ///
    /// The write skips the coalescing: it is sent to the device right
    /// away, and a value queued for the same control before it is
    /// dropped, so the value asked for is the one the device holds when
    /// the call returns.
    ///
    /// The outcome is returned to the caller **and** reported on the
    /// channel of [`ControlWriter::spawn`]: the thread sends one
    /// [`WriteOutcome`] per write, whatever the path it took. A caller
    /// that only uses the returned result must drain that outcome (or
    /// ignore it), otherwise the outcomes pile up in the channel.
    pub fn set_and_wait(&self, name: &str, value: i64) -> Result<(), String> {
        let (reply, answer) = mpsc::channel();
        self.commands
            .send(Command::SetAndWait {
                name: name.to_string(),
                value,
                reply,
            })
            .map_err(|_| "the writer thread is gone".to_string())?;
        answer
            .recv_timeout(ANSWER_TIMEOUT)
            .map_err(|err| format!("the writer thread did not answer: {err}"))?
    }
}

/// The body of the writer thread.
fn run(backend: Arc<dyn AlsaBackend>, inbox: Receiver<Command>, outcomes: Sender<WriteOutcome>) {
    let mut pending: HashMap<String, i64> = HashMap::new();
    // When the pending values have to be written. `None` means the writer
    // is idle.
    let mut deadline: Option<Instant> = None;

    loop {
        let now = Instant::now();
        if let Some(at) = deadline {
            if now >= at {
                flush(&*backend, &mut pending, &outcomes);
                deadline = None;
                continue;
            }
        }
        let wait = match deadline {
            Some(at) => at.saturating_duration_since(now),
            None => IDLE_WAIT,
        };

        match inbox.recv_timeout(wait) {
            Ok(Command::Set { name, value }) => {
                pending.insert(name, value);
                if deadline.is_none() {
                    deadline = Some(Instant::now() + FLUSH_INTERVAL);
                }
            }
            Ok(Command::SetAndWait { name, value, reply }) => {
                // The value asked for wins over the queued one.
                pending.remove(&name);
                let result = backend.write(&name, value);
                let _ = outcomes.send(match &result {
                    Ok(()) => WriteOutcome::Ok {
                        name: name.clone(),
                        value,
                    },
                    Err(err) => WriteOutcome::Err {
                        name: name.clone(),
                        error: err.to_string(),
                    },
                });
                let _ = reply.send(result.map_err(|err| err.to_string()));
            }
            Err(RecvTimeoutError::Timeout) => {
                flush(&*backend, &mut pending, &outcomes);
                deadline = None;
            }
            Err(RecvTimeoutError::Disconnected) => {
                // Nobody talks to the writer any more: write what is left.
                flush(&*backend, &mut pending, &outcomes);
                break;
            }
        }
    }
}

/// Writes the last value of every pending control.
fn flush(
    backend: &dyn AlsaBackend,
    pending: &mut HashMap<String, i64>,
    outcomes: &Sender<WriteOutcome>,
) {
    if pending.is_empty() {
        return;
    }
    let batch: Vec<(String, i64)> = pending.drain().collect();
    for (name, value) in batch {
        let _ = outcomes.send(match backend.write(&name, value) {
            Ok(()) => WriteOutcome::Ok { name, value },
            Err(err) => WriteOutcome::Err {
                name,
                error: err.to_string(),
            },
        });
    }
}
