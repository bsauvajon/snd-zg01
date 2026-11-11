//! The MIC state read worker.
//!
//! The ALSA side of a read is deliberately outside the GTK loop: the
//! action write and the status polling can take up to the driver's timeout.
//! The application starts this worker and forwards the single outcome to
//! its own component.

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::alsa::AlsaBackend;
use crate::model::{STATE_READ_ACTION, STATE_READ_STATUS};
use crate::writer::ControlWriter;

/// The default wait for the driver's state read status.
///
/// The driver times the dump at 2 s; the GUI adds a little grace for the
/// final status publication.
pub const STATE_READ_TIMEOUT: Duration = Duration::from_millis(2500);

/// How often the worker polls the driver status.
pub const STATE_READ_POLL: Duration = Duration::from_millis(25);

/// One finished state read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateReadOutcome {
    /// The driver status: 1 success, 0 pending, negative error.
    pub status: i64,
    /// Whether a read was already pending when the worker started.
    pub pending_before: bool,
    /// Whether the State Read action reached the device.
    pub wrote: Result<(), String>,
}

impl StateReadOutcome {
    /// Whether the read delivered a device state.
    ///
    /// A read that was already pending when the worker started is a success
    /// even though the action itself returned `-EBUSY`. A failed action
    /// with no pending read is not.
    pub fn succeeded(&self) -> bool {
        self.status == 1 && (self.wrote.is_ok() || self.pending_before)
    }
}

/// Starts a state read and returns its one-shot outcome channel.
pub fn spawn_state_read(
    backend: Arc<dyn AlsaBackend>,
    writer: ControlWriter,
) -> Receiver<StateReadOutcome> {
    spawn_state_read_with(backend, writer, STATE_READ_TIMEOUT, STATE_READ_POLL)
}

/// Starts a state read with an explicit timeout and poll interval.
///
/// The short values exist for tests; the application uses the defaults.
pub fn spawn_state_read_with(
    backend: Arc<dyn AlsaBackend>,
    writer: ControlWriter,
    timeout: Duration,
    poll: Duration,
) -> Receiver<StateReadOutcome> {
    let (results, receiver) = mpsc::channel();
    let thread_results = results.clone();
    let spawned = thread::Builder::new()
        .name("zg01-state-read".to_string())
        .spawn(move || {
            let pending_before = matches!(backend.read(STATE_READ_STATUS), Ok(0));
            let wrote = if pending_before {
                Ok(())
            } else {
                writer.flush_set_and_wait(STATE_READ_ACTION, 1)
            };
            let deadline = Instant::now() + timeout;
            let mut status = if pending_before { 0 } else { -1 };
            loop {
                if let Ok(value) = backend.read(STATE_READ_STATUS) {
                    status = value;
                    if value != 0 {
                        break;
                    }
                }
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(poll);
            }
            let _ = thread_results.send(StateReadOutcome {
                status,
                pending_before,
                wrote,
            });
        });
    if let Err(err) = spawned {
        let _ = results.send(StateReadOutcome {
            status: -1,
            pending_before: false,
            wrote: Err(format!("could not start the state read thread: {err}")),
        });
    }
    receiver
}

#[cfg(test)]
mod tests {
    use super::StateReadOutcome;

    #[test]
    fn a_pending_read_can_succeed_with_an_busy_action() {
        assert!(StateReadOutcome {
            status: 1,
            pending_before: true,
            wrote: Err("busy".to_string()),
        }
        .succeeded());
    }

    #[test]
    fn a_failed_action_with_a_stale_success_status_is_not_success() {
        assert!(!StateReadOutcome {
            status: 1,
            pending_before: false,
            wrote: Err("busy".to_string()),
        }
        .succeeded());
        assert!(!StateReadOutcome {
            status: -110,
            pending_before: false,
            wrote: Ok(()),
        }
        .succeeded());
        assert!(!StateReadOutcome {
            status: 0,
            pending_before: false,
            wrote: Ok(()),
        }
        .succeeded());
    }
}
