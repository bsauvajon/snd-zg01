//! The device watcher.
//!
//! The ZG01 is a USB device: it can be unplugged while the GUI is open.
//! The watcher polls the card list and reports the *transitions* only, so
//! the GUI can show the connection view and come back without being woken
//! up by an event it already knows about.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use crate::alsa::CardInfo;

/// What happened to the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    /// The device is there, or came back.
    Connected(CardInfo),
    /// The device is gone.
    Disconnected,
}

/// Watches the presence of the device in a worker thread.
///
/// The presence is polled every `interval`; the first poll happens right
/// away, so a device that is already connected is reported at once. Only
/// the transitions are sent: a device that stays connected produces one
/// [`DeviceEvent::Connected`], and a device that stays gone produces
/// nothing.
///
/// The watcher stops when the receiver is dropped and the next change is
/// reported; the thread also ends with the process.
pub fn spawn_watcher<F>(presence: F, interval: Duration) -> Receiver<DeviceEvent>
where
    F: Fn() -> Option<CardInfo> + Send + 'static,
{
    let (events, receiver) = mpsc::channel::<DeviceEvent>();
    std::thread::Builder::new()
        .name("zg01-device".to_string())
        .spawn(move || run(presence, interval, events))
        .expect("could not start the device watcher");
    receiver
}

/// The body of the watcher thread.
fn run<F>(presence: F, interval: Duration, events: Sender<DeviceEvent>)
where
    F: Fn() -> Option<CardInfo> + Send + 'static,
{
    // The device is not known to be there yet: only a card that shows up
    // is a transition.
    let mut connected: Option<CardInfo> = None;

    loop {
        let current = presence();
        let changes = match (&connected, &current) {
            // The card did not change: nothing to report.
            (Some(previous), Some(now)) if same_card(previous, now) => Vec::new(),
            // The device came back.
            (None, Some(now)) => vec![DeviceEvent::Connected(now.clone())],
            // Another card took the place of the device: the old one is
            // gone and the new one is there.
            (Some(_), Some(now)) => vec![
                DeviceEvent::Disconnected,
                DeviceEvent::Connected(now.clone()),
            ],
            // The device is gone.
            (Some(_), None) => vec![DeviceEvent::Disconnected],
            // The device was never there.
            (None, None) => Vec::new(),
        };
        for change in changes {
            if events.send(change).is_err() {
                // Nobody listens any more.
                return;
            }
        }
        connected = current;

        std::thread::sleep(interval);
    }
}

/// Whether two cards are the same device.
///
/// The USB generation is part of the identity: a quick unplug/replug keeps
/// the ALSA index and id but gets a new `devnum`, and the old backend must
/// not be reused.
fn same_card(left: &CardInfo, right: &CardInfo) -> bool {
    left.index == right.index && left.id == right.id && left.generation == right.generation
}

#[cfg(test)]
mod tests {
    use super::{same_card, CardInfo, DeviceEvent};

    fn card() -> CardInfo {
        CardInfo {
            index: 2,
            id: "zg01".to_string(),
            driver: "zg01_usb".to_string(),
            generation: Some("1:5".to_string()),
        }
    }

    #[test]
    fn a_card_is_itself() {
        assert!(same_card(&card(), &card()));
        let mut other = card();
        other.index = 3;
        assert!(!same_card(&card(), &other));
    }

    #[test]
    fn a_quick_replug_is_a_new_card() {
        let mut replugged = card();
        replugged.generation = Some("1:7".to_string());
        assert!(!same_card(&card(), &replugged));
    }

    #[test]
    fn an_event_shows_the_card() {
        let event = DeviceEvent::Connected(card());
        assert_eq!(event, DeviceEvent::Connected(card()));
        assert_ne!(event, DeviceEvent::Disconnected);
    }
}
