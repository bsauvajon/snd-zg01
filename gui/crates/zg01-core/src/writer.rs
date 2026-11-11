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
    /// The card went away while the write was in flight.
    ///
    /// The GUI must drop the backend and re-read the card, not just revert
    /// the widget: a quick unplug/replug with the same ALSA index would
    /// otherwise keep using the old handle. `generation` identifies the
    /// backend that emitted the event, so a stale event cannot tear down a
    /// card that was already reloaded.
    DeviceLost {
        /// The generation of the backend that saw the failure.
        generation: u64,
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
    /// Write every pending value now and confirm it.
    Flush {
        /// Where the result is sent back once the pending values are on
        /// the device.
        reply: Sender<Result<(), String>>,
    },
    /// Flush every pending value, then write one value and confirm it.
    ///
    /// This is the Save path: a Save must not reach the device before the
    /// edits that were queued before it. Doing both in the worker makes
    /// the barrier and the action one serialized command.
    FlushSetAndWait {
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
        Self::spawn_with_generation(backend, 0)
    }

    /// Starts the writer thread for one backend generation.
    ///
    /// The generation is copied into every [`WriteOutcome::DeviceLost`], so
    /// the GUI can tell a failure of the card it just replaced from one of
    /// the current card.
    pub fn spawn_with_generation(
        backend: Arc<dyn AlsaBackend>,
        generation: u64,
    ) -> (ControlWriter, Receiver<WriteOutcome>) {
        let (commands, inbox) = mpsc::channel::<Command>();
        let (outcomes, receiver) = mpsc::channel::<WriteOutcome>();
        std::thread::Builder::new()
            .name("zg01-writer".to_string())
            .spawn(move || run(backend, inbox, outcomes, generation))
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
    /// the call returns. Values queued for **other** controls are not
    /// touched; use [`ControlWriter::flush_and_wait`] for a global barrier.
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

    /// Writes every pending coalesced value and waits for them.
    ///
    /// This is a global barrier: when it returns, every value that was
    /// pending when the command reached the writer thread has been written,
    /// and their outcomes have been sent on the channel. It does not
    /// reorder the pending values among themselves (the writer never
    /// promised an order across controls); what matters is that none of
    /// them is left behind.
    ///
    /// Used before an action that must take effect after the edits queued
    /// before it: a Reset reloads the persisted settings, so a fader write
    /// flushed after the reset would silently undo it.
    pub fn flush_and_wait(&self) -> Result<(), String> {
        let (reply, answer) = mpsc::channel();
        self.commands
            .send(Command::Flush { reply })
            .map_err(|_| "the writer thread is gone".to_string())?;
        answer
            .recv_timeout(ANSWER_TIMEOUT)
            .map_err(|err| format!("the writer thread did not answer: {err}"))?
    }

    /// Flushes every pending value, then writes `name` and waits for it.
    ///
    /// This is the Save action: it is a global barrier followed by one
    /// awaited write, performed by the same worker. A fader, a toggle or a
    /// preset value queued before it is on the device when the call
    /// returns, so the persisted settings cannot miss it. If the flush
    /// fails, the action is **not** written: saving would persist a value
    /// the user did not ask for.
    pub fn flush_set_and_wait(&self, name: &str, value: i64) -> Result<(), String> {
        let (reply, answer) = mpsc::channel();
        self.commands
            .send(Command::FlushSetAndWait {
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
fn run(
    backend: Arc<dyn AlsaBackend>,
    inbox: Receiver<Command>,
    outcomes: Sender<WriteOutcome>,
    generation: u64,
) {
    let mut pending: HashMap<String, i64> = HashMap::new();
    // When the pending values have to be written. `None` means the writer
    // is idle.
    let mut deadline: Option<Instant> = None;

    loop {
        let now = Instant::now();
        if let Some(at) = deadline {
            if now >= at {
                let _ = flush(&*backend, &mut pending, &outcomes, generation);
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
                report(&outcomes, &name, value, &result, generation);
                let _ = reply.send(result.map_err(|err| err.to_string()));
            }
            Ok(Command::Flush { reply }) => {
                let result = flush(&*backend, &mut pending, &outcomes, generation);
                deadline = None;
                let _ = reply.send(result);
            }
            Ok(Command::FlushSetAndWait { name, value, reply }) => {
                // The barrier must succeed before the action is written:
                // saving after a failed flush would persist a stale value.
                match flush(&*backend, &mut pending, &outcomes, generation) {
                    Ok(()) => {
                        let result = backend.write(&name, value);
                        report(&outcomes, &name, value, &result, generation);
                        let _ = reply.send(result.map_err(|err| err.to_string()));
                    }
                    Err(error) => {
                        let _ =
                            reply.send(Err(format!("could not flush the pending writes: {error}")));
                    }
                }
                deadline = None;
            }
            Err(RecvTimeoutError::Timeout) => {
                let _ = flush(&*backend, &mut pending, &outcomes, generation);
                deadline = None;
            }
            Err(RecvTimeoutError::Disconnected) => {
                // Nobody talks to the writer any more: write what is left.
                let _ = flush(&*backend, &mut pending, &outcomes, generation);
                break;
            }
        }
    }
}

/// Reports one write on the outcome channel, keeping liveness errors apart.
fn report(
    outcomes: &Sender<WriteOutcome>,
    name: &str,
    value: i64,
    result: &Result<(), crate::alsa::AlsaError>,
    generation: u64,
) {
    let outcome = match result {
        Ok(()) => WriteOutcome::Ok {
            name: name.to_string(),
            value,
        },
        Err(err) if err.is_liveness() => WriteOutcome::DeviceLost {
            generation,
            name: name.to_string(),
            error: err.to_string(),
        },
        Err(err) => WriteOutcome::Err {
            name: name.to_string(),
            error: err.to_string(),
        },
    };
    let _ = outcomes.send(outcome);
}

/// Writes the last value of every pending control.
///
/// Returns the first failure, joined with the others so a caller that
/// waited for the flush knows which controls did not land. The caller may
/// ignore it: a normal flush reports every failure on the outcome channel
/// anyway.
fn flush(
    backend: &dyn AlsaBackend,
    pending: &mut HashMap<String, i64>,
    outcomes: &Sender<WriteOutcome>,
    generation: u64,
) -> Result<(), String> {
    if pending.is_empty() {
        return Ok(());
    }
    let batch: Vec<(String, i64)> = pending.drain().collect();
    let mut failures = Vec::new();
    for (name, value) in batch {
        let result = backend.write(&name, value);
        if let Err(err) = &result {
            failures.push(err.to_string());
        }
        report(outcomes, &name, value, &result, generation);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}
