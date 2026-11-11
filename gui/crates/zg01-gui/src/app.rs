//! The application component: the shell and the device state machine.
//!
//! Startup discovers the ZG01 card, reads every control into the model and
//! shows the shell. A watcher reports the connection transitions, so
//! unplugging the card shows the connection view and plugging it back in
//! reloads the controls. The Mixage, Micro and Avancé screens are rebuilt
//! from the controls whenever they are read.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use gtk4::prelude::*;
use relm4::prelude::*;
use zg01_core::advanced::AdvancedModel;
use zg01_core::alsa::{
    find_card, AlsaBackend, AlsaError, CardInfo, ControlInfo, ControlKind, RealAlsa,
};
use zg01_core::device::{spawn_watcher, DeviceEvent};
use zg01_core::layout::{load_layout_str, Layout, LAYOUT_TOML};
use zg01_core::meters::{spawn_poller_with_generation, MeterSnapshot, METER_INTERVAL};
use zg01_core::mic::MicModel;
use zg01_core::mixage::{LinuxMeterId, MeterId, MixageModel};
use zg01_core::model::{
    spawn_reset, AppState, Control, ControlValue, DeviceState, StateRead, Unit, RESET_ACTION,
    SAVE_ACTION, STATE_READ_ACTION, STATE_READ_STATUS,
};
use zg01_core::pipewire::{
    spawn_linux_meters, LinuxMetersHandle, LinuxMetersSnapshot, LINUX_METER_INTERVAL,
};
use zg01_core::preset::{apply_plan_cancellable, ApplyCancellation, ApplyReport};
use zg01_core::state_read::{spawn_state_read, StateReadOutcome};
use zg01_core::units::Conv;
use zg01_core::writer::{ControlWriter, WriteOutcome};

use crate::i18n::{t, tf};
use crate::ui::{
    present_preset_dialog, present_preset_report, AdvancedEvent, AdvancedScreen, MicEvent,
    MicScreen, MixageEvent, MixageScreen, PresetChoice,
};
use crate::window::Shell;

/// Title of the main window.
const APP_TITLE: &str = "ZG01 Control";

/// Default width of the main window, in logical pixels.
const WINDOW_WIDTH: i32 = 1100;

/// Default height of the main window, in logical pixels.
const WINDOW_HEIGHT: i32 = 680;

/// How often the device watcher polls the card list.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);

/// The first delay before trying to read a card again after a failure.
const RETRY_FIRST: Duration = Duration::from_secs(1);

/// The longest delay between two retries.
const RETRY_MAX: Duration = Duration::from_secs(2);

/// How often the pump threads check their stop flag.
const PUMP_TICK: Duration = Duration::from_millis(200);

/// The screens of the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    /// The landing screen: stream sends, stream output and meters.
    Mixage,
    /// The microphone chain: gate, compressor, EQ and limiter.
    Micro,
    /// Every control the curated screens do not claim.
    Advanced,
}

impl Page {
    /// The screens, in sidebar order.
    pub const ALL: [Page; 3] = [Page::Mixage, Page::Micro, Page::Advanced];

    /// The row index of the screen.
    pub fn index(self) -> i32 {
        match self {
            Page::Mixage => 0,
            Page::Micro => 1,
            Page::Advanced => 2,
        }
    }

    /// The screen of a row index.
    pub fn from_index(index: i32) -> Option<Self> {
        Page::ALL.into_iter().find(|page| page.index() == index)
    }

    /// The title shown in the sidebar and the page.
    ///
    /// The literal is wrapped here rather than at the call sites so
    /// `xgettext` sees every page name.
    pub fn title(self) -> String {
        match self {
            Page::Mixage => t("Mixage"),
            Page::Micro => t("Micro"),
            Page::Advanced => t("Advanced"),
        }
    }

    /// What the screen shows, until its own task fills it in.
    pub fn description(self) -> String {
        match self {
            Page::Mixage => t("The three stream sends, the stream output and the meters."),
            Page::Micro => t("The microphone chain: GATE, COMP, EQ and LIMITER."),
            Page::Advanced => t("Every ALSA control the curated screens do not cover."),
        }
    }

    /// The icon of the sidebar row.
    pub fn icon(self) -> &'static str {
        match self {
            Page::Mixage => "audio-volume-high-symbolic",
            Page::Micro => "audio-input-microphone-symbolic",
            Page::Advanced => "emblem-system-symbolic",
        }
    }

    /// The name of the page in the content stack.
    pub fn stack_name(self) -> &'static str {
        match self {
            Page::Mixage => "mixage",
            Page::Micro => "micro",
            Page::Advanced => "advanced",
        }
    }
}

/// The application model.
pub(crate) struct App {
    /// The connection state, the controls and the layout table.
    state: AppState,
    /// The backend of the connected card.
    backend: Option<Arc<dyn AlsaBackend>>,
    /// The selected screen.
    page: Page,
    /// The shell widgets.
    shell: Shell,
    /// The Mixage screen, rebuilt whenever the controls are read.
    mixage: Option<MixageScreen>,
    /// The Micro screen, rebuilt whenever the controls are read.
    mic: Option<MicScreen>,
    /// The Avancé screen, rebuilt whenever the controls are read.
    advanced: Option<AdvancedScreen>,
    /// The coalescing writer of the connected card.
    writer: Option<ControlWriter>,
    /// The stop flag of the write-outcome pump, when it runs.
    writer_stop: Option<Arc<AtomicBool>>,
    /// The stop flag of the meter pump, when it runs.
    meters_stop: Option<Arc<AtomicBool>>,
    /// The PipeWire poller and its reaping handle, when it runs.
    linux_meters: Option<LinuxMetersHandle>,
    /// The epoch of the control writes, used to ignore outcomes that
    /// predate a reset.
    writes: WriteEpoch,
    /// The window, used as the parent of the About dialog.
    root: adw::ApplicationWindow,
    /// Serialises the asynchronous reloads with the presence events.
    reloads: ReloadTracker,
    /// The preset application in flight, when there is one.
    preset: Option<PresetOperation>,
    /// The id of the next operation (save, reset, preset).
    next_operation: u64,
    /// The save in flight, when there is one.
    save: Option<u64>,
    /// The reset in flight, when there is one.
    reset: Option<u64>,
    /// The MIC state read in flight, when there is one.
    state_read_op: Option<u64>,
    /// A failed/pending read result to apply after the reload.
    readback_override: Option<StateRead>,
    /// Whether the startup read was already followed.
    startup_read_attempted: bool,
    /// A Save requested while a preset was running.
    pending_save: bool,
    /// A Reset requested while a preset was running.
    pending_reset: bool,
    /// The last preset applied in this session, offered again on reconnect.
    last_preset: Option<PresetChoice>,
    /// The generation of the backend the screens describe.
    device_generation: u64,
}

/// One preset application in flight.
struct PresetOperation {
    /// The id that stamps its progress and completion messages.
    id: u64,
    /// Stops the worker before its next write.
    cancel: ApplyCancellation,
}

/// What the application reacts to.
#[derive(Debug)]
pub(crate) enum Msg {
    /// A screen was selected in the sidebar.
    ShowPage(Page),
    /// The About dialog was requested.
    About,
    /// The Save action was clicked.
    Save,
    /// The Reset action was clicked.
    Reset,
    /// The preset chooser was requested by the keyboard shortcut.
    OpenPreset,
    /// Reapply the last preset after a reconnect.
    ReapplyPreset,
    /// Quit the application.
    Quit,
    /// A save finished.
    SaveFinished {
        /// The save operation it belongs to.
        id: u64,
        /// Whether the action reached the device.
        result: Result<(), String>,
    },
    /// A reset finished.
    ResetFinished {
        /// The reset operation it belongs to.
        id: u64,
        /// Whether the action reached the device.
        result: Result<(), String>,
    },
    /// A MIC state read finished.
    StateReadFinished {
        /// The read operation it belongs to.
        id: u64,
        /// The worker result.
        outcome: StateReadOutcome,
    },
    /// A preset application made progress.
    PresetProgress {
        /// The preset operation it belongs to.
        id: u64,
        /// How many values have been processed.
        done: usize,
        /// How many values the plan applies.
        total: usize,
        /// How many parameters the plan skips.
        skipped: usize,
    },
    /// The device watcher saw a transition.
    Device(DeviceEvent),
    /// Reading a card finished.
    ReloadFinished {
        /// The reload generation this completion belongs to.
        generation: u64,
        /// The card and its controls, or the reason the read failed.
        result: Result<Loaded, String>,
    },
    /// A retry timer fired.
    RetryDue {
        /// The reload generation the timer was scheduled for.
        generation: u64,
    },
    /// The Mixage screen reported a user action.
    Mixage(MixageEvent),
    /// The Micro screen reported a user action.
    Mic(MicEvent),
    /// The Avancé screen reported a user action.
    Advanced(AdvancedEvent),
    /// The control writer finished a write.
    WriteOutcome(WriteOutcome),
    /// The meter pump read the four meters.
    Meters(MeterSnapshot),
    /// The PipeWire pump read the three Linux sources.
    LinuxMeters(LinuxMetersSnapshot),
    /// The preset preview handed a plan to apply.
    PresetChosen(PresetChoice),
    /// Applying a preset finished.
    PresetApplied {
        /// The preset operation it belongs to.
        id: u64,
        /// The block and name of the preset, for the report.
        label: String,
        /// What landed and what did not.
        report: ApplyReport,
    },
}

/// A card and the controls that were read from it.
pub(crate) struct Loaded {
    card: CardInfo,
    backend: Arc<dyn AlsaBackend>,
    controls: Vec<Control>,
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Loaded")
            .field("card", &self.card)
            .field("controls", &self.controls.len())
            .finish()
    }
}

/// Why a reload completion cannot be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReloadCompletion {
    /// A disconnect or a newer reload invalidated this one.
    Stale,
    /// The generation is current, but the card is no longer present.
    CardGone,
    /// The controls can be installed.
    Accepted,
}

/// Serialises the asynchronous reloads with the device presence.
///
/// A reload reads several controls on a worker thread, so a disconnect or
/// a newer card can happen before it finishes. Every reload carries the
/// generation it was started with, and a completion is only installed
/// when that generation is still current and the card is still present.
#[derive(Debug, Default)]
struct ReloadTracker {
    generation: u64,
    in_flight: bool,
    retry_scheduled: bool,
    failures: u32,
}

impl ReloadTracker {
    /// Invalidates every reload and retry started so far.
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.in_flight = false;
        self.retry_scheduled = false;
        self.failures = 0;
    }

    /// Whether a reload is already running.
    fn in_flight(&self) -> bool {
        self.in_flight
    }

    /// Starts a reload and returns its generation.
    fn begin(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.in_flight = true;
        self.retry_scheduled = false;
        self.generation
    }

    /// Validates a failed reload and accepts a retry for it.
    fn failed(&mut self, generation: u64) -> bool {
        if generation != self.generation || !self.in_flight {
            return false;
        }
        self.in_flight = false;
        true
    }

    /// Validates a successful reload and the card presence.
    fn completion(
        &mut self,
        generation: u64,
        loaded_index: u32,
        present_index: Option<u32>,
    ) -> ReloadCompletion {
        if generation != self.generation || !self.in_flight {
            return ReloadCompletion::Stale;
        }
        self.in_flight = false;
        if present_index != Some(loaded_index) {
            return ReloadCompletion::CardGone;
        }
        self.failures = 0;
        ReloadCompletion::Accepted
    }

    /// Schedules one retry and returns its generation and delay.
    ///
    /// There is never more than one retry timer pending, and no retry is
    /// scheduled while a read is in flight.
    fn retry(&mut self) -> Option<(u64, Duration)> {
        if self.in_flight || self.retry_scheduled {
            return None;
        }
        self.retry_scheduled = true;
        self.failures = self.failures.saturating_add(1);
        Some((self.generation, retry_delay(self.failures)))
    }

    /// Accepts a retry timer, unless it was invalidated or superseded.
    fn retry_due(&mut self, generation: u64) -> bool {
        if !self.retry_scheduled || generation != self.generation {
            return false;
        }
        self.retry_scheduled = false;
        true
    }

    /// Forgets a retry timer whose thread could not start.
    fn cancel_retry(&mut self) {
        self.retry_scheduled = false;
    }
}

/// The delay before the next retry: one second, then two seconds at most.
fn retry_delay(failures: u32) -> Duration {
    if failures <= 1 {
        RETRY_FIRST
    } else {
        RETRY_MAX
    }
}

/// Tracks which write outcomes still describe the device.
///
/// A reset invalidates every write queued before it: the driver reloads
/// its persisted settings, so the value of a write that was in flight when
/// Reset was pressed no longer describes the device. Every write is
/// stamped with the current epoch, and an outcome is only trusted when its
/// control was written in the current epoch.
#[derive(Debug, Default)]
struct WriteEpoch {
    epoch: u64,
    issued: HashMap<String, u64>,
}

impl WriteEpoch {
    /// Stamps a write to `name` with the current epoch.
    fn issue(&mut self, name: &str) {
        self.issued.insert(name.to_string(), self.epoch);
    }

    /// Invalidates every write issued so far.
    fn reset(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Whether an outcome for `name` still describes the device.
    fn accepts(&self, name: &str) -> bool {
        self.issued.get(name) == Some(&self.epoch)
    }
}

#[relm4::component(pub(crate))]
impl SimpleComponent for App {
    type Init = ();
    type Input = Msg;
    type Output = ();

    view! {
        adw::ApplicationWindow {
            set_title: Some(APP_TITLE),
            set_default_width: WINDOW_WIDTH,
            set_default_height: WINDOW_HEIGHT,
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let layout = match load_layout_str(LAYOUT_TOML) {
            Ok(layout) => layout,
            Err(err) => {
                tracing::error!("the compiled layout table is invalid: {err}");
                Layout {
                    screens: Vec::new(),
                }
            }
        };

        let shell = Shell::build(&root, sender.clone());

        // Startup reads the card on a worker: the GTK loop must not wait
        // for a USB control read (or for a card that is not there yet).
        shell.show_loading();
        shell.set_save_enabled(false);

        spawn_device_watcher(sender.clone());

        let mut model = App {
            state: AppState::new(DeviceState::Disconnected, Vec::new(), layout),
            backend: None,
            page: Page::Mixage,
            mixage: None,
            mic: None,
            advanced: None,
            writer: None,
            writer_stop: None,
            meters_stop: None,
            linux_meters: None,
            writes: WriteEpoch::default(),
            shell,
            root: root.clone(),
            reloads: ReloadTracker::default(),
            preset: None,
            next_operation: 0,
            save: None,
            reset: None,
            state_read_op: None,
            readback_override: None,
            startup_read_attempted: false,
            pending_save: false,
            pending_reset: false,
            last_preset: None,
            device_generation: 0,
        };

        model.start_reload(sender);

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            Msg::ShowPage(page) => {
                tracing::debug!("page selected: {page:?}");
                self.page = page;
                if self.state.device.is_connected() || !self.state.controls.is_empty() {
                    self.shell.set_page(page);
                }
            }
            Msg::About => {
                self.shell
                    .present_about(&self.root, self.state.device.card().map(|card| card.index));
            }
            Msg::Save => self.save(sender),
            Msg::Reset => self.reset(sender),
            Msg::OpenPreset => self.open_preset(sender),
            Msg::ReapplyPreset => {
                if let Some(choice) = self.last_preset.clone() {
                    self.apply_preset(choice, sender);
                }
            }
            Msg::Quit => {
                self.stop_meters();
                self.stop_writer();
                relm4::main_application().quit();
            }
            Msg::SaveFinished { id, result } => self.save_finished(id, result),
            Msg::ResetFinished { id, result } => self.reset_finished(id, result, sender),
            Msg::StateReadFinished { id, outcome } => self.state_read_finished(id, outcome, sender),
            Msg::PresetProgress {
                id,
                done,
                total,
                skipped,
            } => self.preset_progress(id, done, total, skipped),
            Msg::Device(DeviceEvent::Disconnected) => self.disconnect(),
            Msg::Device(DeviceEvent::Connected(card)) => self.connect(card, sender),
            Msg::ReloadFinished { generation, result } => {
                self.reload_finished(generation, result, sender);
            }
            Msg::RetryDue { generation } => self.retry_due(generation, sender),
            Msg::Mixage(event) => match event {
                MixageEvent::OpenMic => self.open_mic(),
                MixageEvent::Write { name, value } => self.write_control(&name, value),
                MixageEvent::LoadPreset => self.open_preset(sender),
                MixageEvent::CancelPreset => self.cancel_preset(),
            },
            Msg::Mic(MicEvent::Write { name, value }) => self.write_control(&name, value),
            Msg::Mic(MicEvent::ReadState) => self.read_state(sender),
            Msg::Advanced(AdvancedEvent::Write { name, value }) => self.write_control(&name, value),
            Msg::WriteOutcome(outcome) => self.write_outcome(outcome, sender),
            Msg::Meters(snapshot) => self.update_meters(snapshot, sender),
            Msg::LinuxMeters(snapshot) => self.update_linux_meters(snapshot),
            Msg::PresetChosen(choice) => self.apply_preset(choice, sender),
            Msg::PresetApplied { id, label, report } => {
                self.preset_applied(id, &label, &report, sender);
            }
        }
    }
}

impl App {
    /// Writes the Save action to the device, off the GTK thread.
    ///
    /// Save goes through the `ControlWriter`: the worker first flushes the
    /// coalesced fader and preset values, then writes `Save to ZG01` in the
    /// same serialized command. That is what keeps a Save from persisting
    /// a value that is still queued behind it. A Save requested while a
    /// preset is running is queued until the preset finishes.
    fn save(&mut self, sender: ComponentSender<Self>) {
        if self.state_read_op.is_some() {
            self.shell
                .toast(&t("A state read is in progress; wait for it to finish"));
            return;
        }
        if self.preset.is_some() {
            self.pending_save = true;
            self.shell
                .toast(&t("Save queued until the preset finishes"));
            return;
        }
        if !self.state.can_save() {
            self.shell
                .toast(&t("Cannot save: unknown values or no card"));
            return;
        }
        let Some(writer) = self.writer.clone() else {
            self.shell.toast(&t("No ZG01 connected"));
            return;
        };

        let id = self.next_operation_id();
        self.save = Some(id);
        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-save".to_string())
            .spawn(move || {
                let result = writer.flush_set_and_wait(SAVE_ACTION, 1);
                sender.input(Msg::SaveFinished { id, result });
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the save thread: {err}");
            self.save = None;
            self.shell.toast(&t("Could not start the save"));
        }
    }

    /// Handles the completion of a Save.
    fn save_finished(&mut self, id: u64, result: Result<(), String>) {
        if self.save != Some(id) {
            return;
        }
        self.save = None;
        match result {
            Ok(()) => self.shell.toast(&t("Settings saved to the ZG01")),
            Err(err) => {
                tracing::warn!("the save action failed: {err}");
                self.shell
                    .toast(&tf("Save failed: {error}", &[("{error}", &err)]));
            }
        }
    }

    /// Writes the Reset action and forgets every displayed value.
    ///
    /// The flush and the awaited action run on a worker: a busy USB card
    /// must not freeze the window for up to ten seconds. A Reset requested
    /// while a preset is running cancels the preset and runs afterwards,
    /// so no preset write can follow the reset and silently undo it.
    fn reset(&mut self, sender: ComponentSender<Self>) {
        if self.state_read_op.is_some() {
            self.shell
                .toast(&t("A state read is in progress; wait for it to finish"));
            return;
        }
        if self.preset.is_some() {
            if let Some(operation) = &self.preset {
                operation.cancel.cancel();
            }
            self.pending_reset = true;
            // A Save queued behind the same preset would persist the
            // values the Reset is about to discard.
            self.pending_save = false;
            self.shell
                .toast(&t("Reset queued until the preset finishes"));
            return;
        }
        if self.reset.is_some() {
            return;
        }
        let Some(writer) = self.writer.clone() else {
            self.shell.toast(&t("No ZG01 connected"));
            return;
        };
        // Every write issued before the reset is superseded by it; its
        // outcome must not mark a value known afterwards.
        self.writes.reset();
        let id = self.next_operation_id();
        self.reset = Some(id);
        self.pending_save = false;
        self.shell.set_busy(true, &t("Resetting the ZG01…"));
        self.set_screens_sensitive(false);

        // Poll the worker from the GTK loop instead of starting a second
        // thread: there is no forwarding-thread spawn that could fail after
        // the Reset was already sent to the device.
        let completion = spawn_reset(writer);
        gtk4::glib::timeout_add_local(
            std::time::Duration::from_millis(25),
            move || match completion.try_recv() {
                Ok(result) => {
                    sender.input(Msg::ResetFinished { id, result });
                    gtk4::glib::ControlFlow::Break
                }
                Err(TryRecvError::Empty) => gtk4::glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    sender.input(Msg::ResetFinished {
                        id,
                        result: Err("the reset thread did not answer".to_string()),
                    });
                    gtk4::glib::ControlFlow::Break
                }
            },
        );
    }

    /// Handles the completion of a Reset.
    ///
    /// Whatever the outcome, every writable value becomes unknown: the
    /// driver reloaded its persisted settings without refreshing the cache,
    /// and a failed action must not leave the GUI claiming to know them.
    fn reset_finished(
        &mut self,
        id: u64,
        result: Result<(), String>,
        sender: ComponentSender<Self>,
    ) {
        if self.reset != Some(id) {
            return;
        }
        self.reset = None;
        self.state.mark_all_unknown();
        self.refresh_controls();
        self.shell.set_busy(false, "");
        self.set_screens_sensitive(self.state.device.is_connected());
        self.sync_shell_state();
        match result {
            Ok(()) => self.shell.toast(&t("ZG01 reset — reading the MIC state…")),
            Err(err) => {
                tracing::warn!("the reset action failed: {err}");
                self.shell
                    .toast(&tf("Reset failed: {error}", &[("{error}", &err)]));
            }
        }
        self.read_state(sender);
    }

    /// Re-reads the MIC state dump and reloads the controls.
    ///
    /// The State Read action returns before the device finishes the dump,
    /// so a worker polls `State Read Status` and only then triggers a
    /// reload. The driver refuses a second read with `-EBUSY`; that is not
    /// a failure: the pending read is the one being waited for.
    fn read_state(&mut self, sender: ComponentSender<Self>) {
        if self.state_read_op.is_some() {
            return;
        }
        if self.reset.is_some() {
            self.shell
                .toast(&t("A reset is in progress; wait for it to finish"));
            return;
        }
        if self.preset.is_some() {
            self.shell
                .toast(&t("A preset is being applied; wait for it to finish"));
            return;
        }
        let Some(backend) = self.backend.clone() else {
            self.shell.toast(&t("No ZG01 connected"));
            return;
        };
        let Some(writer) = self.writer.clone() else {
            self.shell.toast(&t("No ZG01 connected"));
            return;
        };

        let id = self.next_operation_id();
        self.state_read_op = Some(id);
        self.shell.set_busy(true, &t("Reading the MIC state…"));
        self.set_screens_sensitive(false);

        let results = spawn_state_read(backend, writer);
        let sender = sender.clone();
        gtk4::glib::timeout_add_local(Duration::from_millis(25), move || {
            match results.try_recv() {
                Ok(outcome) => {
                    sender.input(Msg::StateReadFinished { id, outcome });
                    gtk4::glib::ControlFlow::Break
                }
                Err(TryRecvError::Empty) => gtk4::glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    sender.input(Msg::StateReadFinished {
                        id,
                        outcome: StateReadOutcome {
                            status: -1,
                            pending_before: false,
                            wrote: Err("the state read worker did not answer".to_string()),
                        },
                    });
                    gtk4::glib::ControlFlow::Break
                }
            }
        });
    }

    /// Handles the completion of a MIC state read.
    fn state_read_finished(
        &mut self,
        id: u64,
        outcome: StateReadOutcome,
        sender: ComponentSender<Self>,
    ) {
        if self.state_read_op != Some(id) {
            return;
        }
        self.state_read_op = None;
        self.shell.set_busy(false, "");
        self.set_screens_sensitive(self.state.device.is_connected());
        let succeeded = outcome.succeeded();
        if succeeded {
            self.shell.toast(&t("MIC state read from the ZG01"));
        } else {
            match &outcome.wrote {
                Err(err) => {
                    self.shell.toast(&tf(
                        "Could not read the MIC state: {error}",
                        &[("{error}", err)],
                    ));
                }
                Ok(()) => self.shell.toast(&t("Could not read the MIC state")),
            }
        }
        self.readback_override = if succeeded {
            None
        } else if outcome.status == 0 {
            Some(StateRead::Pending)
        } else {
            Some(StateRead::Failed)
        };
        self.start_reload(sender);
    }

    /// Shows or hides the network of controls while an operation runs.
    ///
    /// The Mixage preset bar stays usable so its Cancel action can stop a
    /// running preset; only the fader and meter panel is disabled.
    fn set_screens_sensitive(&self, sensitive: bool) {
        if let Some(screen) = &self.mixage {
            screen.set_controls_sensitive(sensitive);
        }
        if let Some(screen) = &self.mic {
            screen.widget().set_sensitive(sensitive);
        }
        if let Some(screen) = &self.advanced {
            screen.widget().set_sensitive(sensitive);
        }
    }

    /// The id of the next operation.
    fn next_operation_id(&mut self) -> u64 {
        self.next_operation = self.next_operation.wrapping_add(1);
        self.next_operation
    }

    /// Forgets every operation that belongs to the card that just left.
    fn invalidate_operations(&mut self) {
        if let Some(operation) = self.preset.take() {
            operation.cancel.cancel();
        }
        if let Some(screen) = &self.mixage {
            screen.set_preset_running(false);
        }
        self.save = None;
        self.reset = None;
        self.state_read_op = None;
        self.pending_save = false;
        self.pending_reset = false;
        self.shell.set_busy(false, "");
    }

    /// Builds the Mixage screen from the current controls and installs it.
    fn install_mixage(&mut self, sender: ComponentSender<Self>) {
        let model = MixageModel::from_controls(&self.state.controls);
        let screen_sender = sender.clone();
        let screen = MixageScreen::new(&model, &self.state, move |event| {
            screen_sender.input(Msg::Mixage(event));
        });
        self.shell.install_mixage(screen.widget());
        self.mixage = Some(screen);
    }

    /// Builds the Micro screen from the current controls and installs it.
    fn install_micro(&mut self, sender: ComponentSender<Self>) {
        let model = MicModel::from_controls(&self.state.controls);
        let screen_sender = sender.clone();
        let screen = MicScreen::new(&model, &self.state, move |event| {
            screen_sender.input(Msg::Mic(event));
        });
        self.shell.install_micro(screen.widget());
        self.mic = Some(screen);
    }

    /// Builds the Avancé screen from the current controls and installs it.
    fn install_advanced(&mut self, sender: ComponentSender<Self>) {
        let mixage = MixageModel::from_controls(&self.state.controls);
        let mic = MicModel::from_controls(&self.state.controls);
        let model =
            AdvancedModel::from_controls(&self.state.controls, &self.state.layout, &mixage, &mic);
        let screen_sender = sender.clone();
        let screen = AdvancedScreen::new(model, &self.state, move |event| {
            screen_sender.input(Msg::Advanced(event));
        });
        self.shell.install_advanced(screen.widget());
        self.advanced = Some(screen);
    }

    /// Starts the coalescing writer and the pump that forwards its
    /// outcomes.
    fn start_writer(&mut self, sender: &ComponentSender<Self>) {
        self.stop_writer();
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let (writer, outcomes) =
            ControlWriter::spawn_with_generation(backend, self.device_generation);
        self.writer = Some(writer);

        let stop = Arc::new(AtomicBool::new(false));
        self.writer_stop = Some(stop.clone());
        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-write-events".to_string())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match outcomes.recv_timeout(PUMP_TICK) {
                        Ok(outcome) => sender.input(Msg::WriteOutcome(outcome)),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the write event thread: {err}");
        }
    }

    /// Drops the writer, which flushes what is pending and stops.
    fn stop_writer(&mut self) {
        if let Some(stop) = self.writer_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        self.writer = None;
    }

    /// Starts the meter poller and the pump that forwards its snapshots.
    fn start_meters(&mut self, sender: &ComponentSender<Self>) {
        self.stop_meters();
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        self.meters_stop = Some(stop.clone());
        let events = spawn_poller_with_generation(backend, METER_INTERVAL, self.device_generation);
        let pump_sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-meter-events".to_string())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match events.recv_timeout(PUMP_TICK) {
                        Ok(snapshot) => pump_sender.input(Msg::Meters(snapshot)),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the meter event thread: {err}");
        }
        self.start_linux_meters(sender);
    }

    /// Asks the meter pump to stop; the poller stops when its receiver is
    /// dropped.
    fn stop_meters(&mut self) {
        if let Some(stop) = self.meters_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        self.stop_linux_meters();
    }

    /// Starts the PipeWire source poller and its forwarding pump.
    fn start_linux_meters(&mut self, sender: &ComponentSender<Self>) {
        self.stop_linux_meters();
        let stop = Arc::new(AtomicBool::new(false));
        let (events, handle) =
            spawn_linux_meters(self.device_generation, LINUX_METER_INTERVAL, stop.clone());
        self.linux_meters = Some(handle);
        let pump_sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-linux-meter-events".to_string())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match events.recv_timeout(PUMP_TICK) {
                        Ok(snapshot) => pump_sender.input(Msg::LinuxMeters(snapshot)),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the Linux meter event thread: {err}");
        }
    }

    /// Stops the PipeWire pump and waits for it to reap every recorder.
    fn stop_linux_meters(&mut self) {
        if let Some(mut handle) = self.linux_meters.take() {
            handle.stop();
        }
    }

    /// Shows the Micro screen, from a click on the MIC strip.
    fn open_mic(&mut self) {
        tracing::debug!("MIC strip clicked");
        if !self.state.device.is_connected() {
            return;
        }
        self.page = Page::Micro;
        self.shell.set_page(Page::Micro);
    }

    /// Opens the preset chooser against the current controls.
    fn open_preset(&mut self, sender: ComponentSender<Self>) {
        if !self.state.device.is_connected() {
            self.shell.toast(&t("No ZG01 connected"));
            return;
        }
        if self.preset.is_some() {
            self.shell.toast(&t("A preset is already being applied"));
            return;
        }
        if self.reset.is_some() {
            self.shell
                .toast(&t("A reset is in progress; wait for it to finish"));
            return;
        }
        if self.state_read_op.is_some() {
            self.shell
                .toast(&t("A state read is in progress; wait for it to finish"));
            return;
        }
        let controls = self.state.controls.clone();
        let sender = sender.clone();
        present_preset_dialog(&self.root, controls, move |choice| {
            sender.input(Msg::PresetChosen(choice));
        });
    }

    /// Applies a confirmed plan, off the GTK thread.
    ///
    /// Each write is stamped with the current epoch before the thread runs,
    /// so its outcome may mark the value known. The report comes back as a
    /// message; the values stay unsaved until the header Save action.
    fn apply_preset(&mut self, choice: PresetChoice, sender: ComponentSender<Self>) {
        if self.preset.is_some() {
            self.shell.toast(&t("A preset is already being applied"));
            return;
        }
        if self.reset.is_some() {
            self.shell
                .toast(&t("A reset is in progress; wait for it to finish"));
            return;
        }
        if self.state_read_op.is_some() {
            self.shell
                .toast(&t("A state read is in progress; wait for it to finish"));
            return;
        }
        let Some(writer) = self.writer.clone() else {
            self.shell.toast(&t("No ZG01 connected"));
            return;
        };
        for apply in &choice.plan.applies {
            self.writes.issue(&apply.name);
        }

        let id = self.next_operation_id();
        let cancellation = ApplyCancellation::new();
        self.preset = Some(PresetOperation {
            id,
            cancel: cancellation.clone(),
        });
        self.last_preset = Some(choice.clone());

        let label = preset_label(&choice.block, &choice.name);
        let plan = choice.plan;
        let total = plan.applies.len();
        let skipped = plan.skipped.len();
        if let Some(screen) = &self.mixage {
            screen.set_preset_running(true);
            screen.set_preset_progress(0, total, skipped);
        }
        self.set_screens_sensitive(false);

        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-preset".to_string())
            .spawn(move || {
                let report = apply_plan_cancellable(&plan, &writer, &cancellation, |report| {
                    sender.input(Msg::PresetProgress {
                        id,
                        done: report.applied.len() + report.failed.len(),
                        total,
                        skipped: report.skipped.len(),
                    });
                });
                sender.input(Msg::PresetApplied { id, label, report });
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the preset thread: {err}");
            self.preset = None;
            self.set_screens_sensitive(self.state.device.is_connected());
            if let Some(screen) = &self.mixage {
                screen.set_preset_running(false);
            }
            self.shell.toast(&t("Could not load the preset"));
        }
    }

    /// Updates the progress row of the preset bar.
    fn preset_progress(&self, id: u64, done: usize, total: usize, skipped: usize) {
        if self.preset.as_ref().map(|operation| operation.id) != Some(id) {
            return;
        }
        if let Some(screen) = &self.mixage {
            screen.set_preset_progress(done, total, skipped);
        }
    }

    /// Asks the running preset to stop before its next write.
    fn cancel_preset(&self) {
        if let Some(operation) = &self.preset {
            operation.cancel.cancel();
            self.shell.toast(&t("Cancelling the preset…"));
        }
    }

    /// Handles the completion of a preset application.
    fn preset_applied(
        &mut self,
        id: u64,
        label: &str,
        report: &ApplyReport,
        sender: ComponentSender<Self>,
    ) {
        if self.preset.as_ref().map(|operation| operation.id) != Some(id) {
            // A disconnect, a reset or a newer operation made this stale.
            return;
        }
        self.preset = None;

        tracing::info!(
            "preset {label:?}: {} applied, {} skipped, {} failed, cancelled: {}",
            report.applied.len(),
            report.skipped.len(),
            report.failed.len(),
            report.cancelled
        );
        // The report is the completion of the writes: mark the applied
        // values known here as well, so a Save queued behind the preset can
        // run at once even if an outcome pump message is still in flight.
        for applied in &report.applied {
            self.state.mark_known(&applied.name, applied.value);
            if let Some(screen) = &self.mixage {
                screen.set_control(&applied.name, ControlValue::Known(applied.value));
            }
            if let Some(screen) = &self.mic {
                screen.set_control(&applied.name, ControlValue::Known(applied.value));
            }
            if let Some(screen) = &self.advanced {
                screen.set_control(&applied.name, ControlValue::Known(applied.value));
            }
        }
        if !report.cancelled && !report.applied.is_empty() {
            if let Some(screen) = &self.mixage {
                screen.set_preset_name(label);
            }
        }
        if let Some(screen) = &self.mixage {
            screen.set_preset_running(false);
        }
        self.set_screens_sensitive(self.state.device.is_connected());
        self.sync_shell_state();
        present_preset_report(&self.root, label, report);

        if report.cancelled {
            self.shell.toast(&t("Preset cancelled"));
        }

        if self.pending_reset {
            self.pending_reset = false;
            self.reset(sender);
        } else if self.pending_save {
            self.pending_save = false;
            self.save(sender);
        }
    }

    /// Queues one control write on the coalescing writer.
    fn write_control(&mut self, name: &str, value: i64) {
        let Some(writer) = self.writer.clone() else {
            self.shell.toast(&t("No ZG01 connected"));
            self.revert_control(name);
            return;
        };
        self.writes.issue(name);
        writer.set(name, value);
    }

    /// Puts a widget back to the value the model still holds.
    fn revert_control(&self, name: &str) {
        let value = self
            .state
            .control(name)
            .map(|control| control.value)
            .unwrap_or(ControlValue::Unknown);
        if let Some(screen) = &self.mixage {
            screen.set_control(name, value);
        }
        if let Some(screen) = &self.mic {
            screen.set_control(name, value);
        }
        if let Some(screen) = &self.advanced {
            screen.set_control(name, value);
        }
    }

    /// Pushes every known or unknown value to the screens.
    fn refresh_controls(&self) {
        for control in &self.state.controls {
            let name = control.name();
            let value = control.value;
            if let Some(screen) = &self.mixage {
                screen.set_control(name, value);
            }
            if let Some(screen) = &self.mic {
                screen.set_control(name, value);
            }
            if let Some(screen) = &self.advanced {
                screen.set_control(name, value);
            }
        }
    }

    /// Matches the header to the state: Save, the unknown-state banner and
    /// the reconnect banner.
    fn sync_shell_state(&self) {
        let connected = self.state.device.is_connected();
        self.shell.set_save_enabled(self.state.can_save());
        self.shell.set_readback(self.state.state_read);
        self.shell.set_unknown(connected && !self.state.can_save());
        self.shell
            .set_device_lost(!connected && !self.state.controls.is_empty());
    }

    /// Handles the outcome of a control write.
    fn write_outcome(&mut self, outcome: WriteOutcome, sender: ComponentSender<Self>) {
        if !outcome_is_current(&outcome, self.device_generation) {
            tracing::debug!(
                "ignoring a stale lost-device outcome (current generation is {})",
                self.device_generation
            );
            return;
        }
        match outcome {
            // A liveness failure is not a control error: the card is gone
            // (or was replaced by one with the same ALSA index).
            WriteOutcome::DeviceLost { name, error, .. } => {
                tracing::warn!("the card went away while writing {name:?}: {error}");
                self.device_lost(&sender);
            }
            // The action outcomes are reported by their own completion
            // messages: success and failure were already handled there.
            WriteOutcome::Ok { name, .. } if name == RESET_ACTION => {
                tracing::debug!("the reset action is on the device");
            }
            WriteOutcome::Err { name, error } if name == RESET_ACTION => {
                tracing::debug!("the reset action failed: {error}");
            }
            WriteOutcome::Ok { name, .. } if name == SAVE_ACTION => {
                tracing::debug!("the save action is on the device");
            }
            WriteOutcome::Err { name, error } if name == SAVE_ACTION => {
                tracing::debug!("the save action failed: {error}");
            }
            WriteOutcome::Ok { name, .. } if name == STATE_READ_ACTION => {
                tracing::debug!("the state read action is on the device");
            }
            WriteOutcome::Err { name, error } if name == STATE_READ_ACTION => {
                tracing::debug!("the state read action failed: {error}");
            }
            WriteOutcome::Ok { name, value } => {
                if !self.writes.accepts(&name) {
                    tracing::debug!(
                        "ignoring the outcome of {name:?}: it was written before the reset"
                    );
                    return;
                }
                self.state.mark_known(&name, value);
                if let Some(screen) = &self.mixage {
                    screen.set_control(&name, ControlValue::Known(value));
                }
                if let Some(screen) = &self.mic {
                    screen.set_control(&name, ControlValue::Known(value));
                }
                if let Some(screen) = &self.advanced {
                    screen.set_control(&name, ControlValue::Known(value));
                }
                self.sync_shell_state();
            }
            WriteOutcome::Err { name, error } => {
                tracing::warn!("could not write the control {name:?}: {error}");
                self.shell.toast(&tf(
                    "Could not write {name}: {error}",
                    &[("{name}", &name), ("{error}", &error)],
                ));
                self.revert_control(&name);
            }
        }
    }

    /// Installs the meter readings on the screens.
    fn update_meters(&mut self, snapshot: MeterSnapshot, sender: ComponentSender<Self>) {
        if snapshot.generation != self.device_generation {
            tracing::debug!(
                "ignoring a meter snapshot from generation {} (current is {})",
                snapshot.generation,
                self.device_generation
            );
            return;
        }
        if snapshot.lost {
            self.device_lost(&sender);
            return;
        }
        for id in [
            MeterId::MicIn,
            MeterId::MicOut,
            MeterId::PcIn,
            MeterId::StreamMix,
        ] {
            let value = snapshot
                .value(id.index())
                .map(ControlValue::Known)
                .unwrap_or(ControlValue::Unknown);
            if let Some(control) = self.state.control_mut(id.control()) {
                control.value = value;
            }
            if let Some(screen) = &self.mixage {
                screen.set_meter(id, snapshot.value(id.index()));
            }
            if let Some(screen) = &self.mic {
                screen.set_meter(id, snapshot.value(id.index()));
            }
            // Avancé lists the meter controls too: keep their readout live.
            if let Some(screen) = &self.advanced {
                screen.set_control(id.control(), value);
            }
        }
    }

    /// Installs the Linux readings on the screens.
    fn update_linux_meters(&mut self, snapshot: LinuxMetersSnapshot) {
        if !self.state.device.is_connected() {
            return;
        }
        if !snapshot.belongs_to(self.device_generation) {
            tracing::debug!(
                "ignoring a Linux meter snapshot from generation {} (current is {})",
                snapshot.generation,
                self.device_generation
            );
            return;
        }
        tracing::trace!("Linux meter snapshot: {snapshot:?}");
        for id in LinuxMeterId::ALL {
            let available = snapshot.is_available(id);
            let level = snapshot.level(id);
            if let Some(screen) = &self.mixage {
                screen.set_linux_meter(id, available, level);
            }
            if let Some(screen) = &self.mic {
                screen.set_linux_meter(id, available, level);
            }
        }
    }

    /// Hides the Linux bars without touching the card meters.
    fn clear_linux_meters(&mut self) {
        if let Some(screen) = &self.mixage {
            screen.clear_linux_meters();
        }
        if let Some(screen) = &self.mic {
            screen.clear_linux_meters();
        }
    }

    /// Keeps the last values, disables the inputs and shows the reconnect
    /// banner when the card goes away.
    ///
    /// Always invalidates the reload tracker, even when the state is
    /// already disconnected: a read started before this event must not
    /// install the card afterwards.
    fn disconnect(&mut self) {
        self.reloads.invalidate();
        self.invalidate_operations();
        if !self.state.device.is_connected() {
            return;
        }

        self.state.disconnect();
        self.backend = None;
        // Writes queued on the card that went away must not be trusted if
        // their outcome arrives after the next card is read.
        self.writes.reset();
        self.stop_writer();
        self.stop_meters();
        self.clear_linux_meters();
        // The values stay visible, but nothing can be changed until the
        // card is back and its controls have been read again. Unlike a
        // running preset, there is no Cancel action to keep usable.
        self.set_screens_sensitive(false);
        if let Some(screen) = &self.mixage {
            screen.widget().set_sensitive(false);
        }
        self.shell.set_reapply(false, "");
        self.shell.show_device_lost();
        self.sync_shell_state();
        self.shell.toast(&t("ZG01 disconnected"));
    }

    /// Drops the card that stopped answering and reads it again.
    ///
    /// A read or a write that returns `ENODEV`/`EIO` is the only signal a
    /// quick same-index replug leaves: the watcher may see the same card
    /// twice. The old backend is dropped before the reload so no stale
    /// handle is reused.
    fn device_lost(&mut self, sender: &ComponentSender<Self>) {
        if !self.state.device.is_connected() {
            return;
        }
        tracing::warn!("the ZG01 stopped answering; dropping the backend and reloading");
        self.disconnect();
        self.shell.toast(&t("ZG01 connection lost — reconnecting"));
        self.start_reload(sender.clone());
    }

    /// Reads a card that just appeared, off the GTK thread.
    fn connect(&mut self, card: CardInfo, sender: ComponentSender<Self>) {
        if self.state.device.card() == Some(&card) {
            return;
        }
        // The startup reload already reads the card the watcher reports.
        if self.reloads.in_flight() {
            return;
        }
        if self.state.device.is_connected() {
            self.disconnect();
        }
        self.start_reload(sender);
    }

    /// Starts one reload and remembers the generation it belongs to.
    fn start_reload(&mut self, sender: ComponentSender<Self>) {
        let generation = self.reloads.begin();
        let layout = self.state.layout.clone();
        let thread_sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-reload".to_string())
            .spawn(move || {
                let result = load_device(&layout);
                thread_sender.input(Msg::ReloadFinished { generation, result });
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the reload thread: {err}");
            self.reload_finished(
                generation,
                Err(format!("could not start the reload thread: {err}")),
                sender,
            );
        }
    }

    /// Handles the completion of a reload, if it is still current.
    fn reload_finished(
        &mut self,
        generation: u64,
        result: Result<Loaded, String>,
        sender: ComponentSender<Self>,
    ) {
        match result {
            Ok(loaded) => {
                let present = find_card(None).ok().map(|card| card.index);
                match self
                    .reloads
                    .completion(generation, loaded.card.index, present)
                {
                    ReloadCompletion::Stale => {}
                    ReloadCompletion::CardGone => self.disconnect(),
                    ReloadCompletion::Accepted => self.reloaded(loaded, sender),
                }
            }
            Err(err) => {
                if !self.reloads.failed(generation) {
                    return;
                }
                let first_failure = self.reloads.failures == 0;
                if first_failure {
                    tracing::warn!("could not read the ZG01: {err}");
                } else {
                    tracing::debug!("the ZG01 retry failed: {err}");
                }
                if find_card(None).is_err() {
                    if self.state.device.is_connected() {
                        self.disconnect();
                    } else if self.state.controls.is_empty() {
                        self.shell.show_disconnected();
                        self.sync_shell_state();
                    } else {
                        // The card stopped answering after a session:
                        // keep its last values visible under the reconnect
                        // banner until it comes back.
                        self.shell.show_device_lost();
                        self.sync_shell_state();
                    }
                    return;
                }

                if let Some((generation, delay)) = self.reloads.retry() {
                    if first_failure {
                        self.shell
                            .toast(&t("ZG01 detected, but its controls cannot be read"));
                    }
                    self.spawn_retry(generation, delay, sender);
                }
            }
        }
    }

    /// Waits for the retry delay and asks the component to try again.
    fn spawn_retry(&mut self, generation: u64, delay: Duration, sender: ComponentSender<Self>) {
        let spawned = std::thread::Builder::new()
            .name("zg01-retry".to_string())
            .spawn(move || {
                std::thread::sleep(delay);
                sender.input(Msg::RetryDue { generation });
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the retry thread: {err}");
            self.reloads.cancel_retry();
        }
    }

    /// Runs a retry timer, unless it was invalidated or superseded.
    fn retry_due(&mut self, generation: u64, sender: ComponentSender<Self>) {
        if !self.reloads.retry_due(generation) {
            return;
        }
        if find_card(None).is_ok() {
            self.start_reload(sender);
        } else {
            self.disconnect();
        }
    }

    /// Installs the state of a card that was read and is still present.
    fn reloaded(&mut self, loaded: Loaded, sender: ComponentSender<Self>) {
        // A completion from the card that just left cannot own an operation.
        self.invalidate_operations();
        let first_connection = !self.state.device.is_connected();
        if first_connection {
            self.startup_read_attempted = false;
        }
        let layout = self.state.layout.clone();
        tracing::info!(
            "reading card hw:{}: {} controls",
            loaded.card.index,
            loaded.controls.len()
        );
        // From here on, only outcomes tagged with this generation describe
        // the card the screens show.
        self.device_generation = self.reloads.generation;
        self.state = AppState::new(DeviceState::Connected(loaded.card), loaded.controls, layout);
        self.state.apply_readback();
        if let Some(override_state) = self.readback_override.take() {
            if !override_state.is_known() {
                self.state.mark_mic_unknown();
            }
            self.state.state_read = override_state;
        }
        self.backend = Some(loaded.backend);
        // The values were just read; an outcome from before the reload
        // must not overwrite one of them.
        self.writes.reset();

        if let Some(card) = self.state.device.card() {
            self.shell.show_connected(card);
        }
        self.sync_shell_state();
        self.install_mixage(sender.clone());
        self.install_micro(sender.clone());
        self.install_advanced(sender.clone());
        self.start_writer(&sender);
        self.start_meters(&sender);
        self.shell.set_page(self.page);
        self.shell.set_reapply(
            self.last_preset.is_some(),
            self.last_preset
                .as_ref()
                .map(|choice| preset_label(&choice.block, &choice.name))
                .as_deref()
                .unwrap_or(""),
        );
        if first_connection {
            self.shell.toast(&t("ZG01 connected"));
        }
        if !self.startup_read_attempted && self.state.state_read == StateRead::Pending {
            self.startup_read_attempted = true;
            self.read_state(sender);
        }
    }
}

/// Whether a write outcome describes the card the screens currently show.
///
/// Only a liveness event carries a generation; a normal value outcome keeps
/// its existing epoch check. A delayed `DeviceLost` from the backend that
/// was just replaced must be ignored, not tear down the new card.
fn outcome_is_current(outcome: &WriteOutcome, generation: u64) -> bool {
    match outcome {
        WriteOutcome::DeviceLost {
            generation: event, ..
        } => *event == generation,
        _ => true,
    }
}

/// The name the preset bar shows for a preset.
fn preset_label(block: &str, name: &str) -> String {
    match (block.is_empty(), name.is_empty()) {
        (true, true) => t("Unnamed preset"),
        (true, false) => name.to_string(),
        (false, true) => block.to_string(),
        (false, false) => format!("{block} · {name}"),
    }
}

/// Starts the watcher that forwards the connection transitions.
fn spawn_device_watcher(sender: ComponentSender<App>) {
    let events = spawn_watcher(|| find_card(None).ok(), WATCH_INTERVAL);
    let spawned = std::thread::Builder::new()
        .name("zg01-device-events".to_string())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                sender.input(Msg::Device(event));
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("could not start the device event thread: {err}");
    }
}

/// Opens the card and reads every control.
fn load_device(layout: &Layout) -> Result<Loaded, String> {
    let card = find_card(None).map_err(|err| format!("card discovery failed: {err}"))?;
    let backend = RealAlsa::new(card.index)
        .map_err(|err| format!("could not open hw:{}: {err}", card.index))?;
    let backend: Arc<dyn AlsaBackend> = Arc::new(backend);
    let infos = backend
        .list_controls()
        .map_err(|err| format!("could not list the controls: {err}"))?;
    /*
     * A startup dump can complete while the controls are read, leaving
     * the direct values from before the dump with a fresh success status.
     * Read the status around the snapshot and retry if it moved.
     */
    let status_before = backend.read(STATE_READ_STATUS).ok();
    let controls = read_controls(backend.as_ref(), infos, layout);
    let status_after = backend.read(STATE_READ_STATUS).ok();
    if status_before != status_after {
        return Err("the MIC state changed while the controls were read".to_string());
    }
    Ok(Loaded {
        card,
        backend,
        controls,
    })
}

/// Reads every control, turning an unreadable value into [`ControlValue::Unknown`].
fn read_controls(
    backend: &dyn AlsaBackend,
    infos: Vec<ControlInfo>,
    layout: &Layout,
) -> Vec<Control> {
    infos
        .into_iter()
        .map(|info| {
            let value = match backend.read(&info.name) {
                Ok(value) => ControlValue::Known(value),
                Err(AlsaError::NoData(_)) => {
                    tracing::debug!("the control {:?} has no value", info.name);
                    ControlValue::Unknown
                }
                Err(err) => {
                    tracing::warn!("could not read the control {:?}: {err}", info.name);
                    ControlValue::Unknown
                }
            };
            let unit = unit_for(layout, &info.name, info.kind);
            Control { info, value, unit }
        })
        .collect()
}

/// The unit of a control: the layout table when it claims it, the control
/// map's conversion otherwise, a sensible default last.
fn unit_for(layout: &Layout, name: &str, kind: ControlKind) -> Unit {
    if let Some(unit) = layout
        .screens
        .iter()
        .flat_map(|screen| screen.groups.iter())
        .flat_map(|group| group.controls.iter())
        .find(|widget| widget.name == name)
        .and_then(|widget| widget.unit)
    {
        return unit;
    }
    if let Some(entry) = zg01_core::preset::control_map()
        .iter()
        .find(|entry| entry.control_name == name)
    {
        match entry.conv {
            Conv::Gain => return Unit::Db,
            Conv::Freq => return Unit::Hz,
            Conv::Q => return Unit::Q,
            Conv::Id => {}
        }
    }
    match kind {
        ControlKind::Boolean => Unit::Bool,
        ControlKind::Integer | ControlKind::Bytes => Unit::Steps,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        outcome_is_current, preset_label, read_controls, retry_delay, unit_for, Page,
        ReloadCompletion, ReloadTracker, WriteEpoch, RETRY_FIRST, RETRY_MAX,
    };
    use std::time::Duration;
    use zg01_core::alsa::{ControlInfo, ControlKind, FakeAlsa};
    use zg01_core::layout::{load_layout_str, LAYOUT_TOML};
    use zg01_core::model::{ControlValue, Unit};
    use zg01_core::writer::WriteOutcome;

    fn info(name: &str, kind: ControlKind, writable: bool) -> ControlInfo {
        ControlInfo {
            name: name.to_string(),
            kind,
            min: 0,
            max: 100,
            step: 0,
            count: 1,
            writable,
        }
    }

    fn layout() -> zg01_core::layout::Layout {
        load_layout_str(LAYOUT_TOML).expect("the compiled table parses")
    }

    #[test]
    fn the_preset_label_names_the_block_and_the_preset() {
        assert_eq!(preset_label("MIC INPUT", "Studio"), "MIC INPUT · Studio");
        assert_eq!(preset_label("MIC INPUT", ""), "MIC INPUT");
        assert_eq!(preset_label("", "Studio"), "Studio");
        assert_eq!(preset_label("", ""), "Unnamed preset");
    }

    #[test]
    fn the_pages_round_trip_through_their_index() {
        for page in Page::ALL {
            assert_eq!(Page::from_index(page.index()), Some(page));
        }
        assert_eq!(Page::from_index(-1), None);
        assert_eq!(Page::from_index(3), None);
    }

    #[test]
    fn the_units_come_from_the_layout_table() {
        assert_eq!(
            unit_for(&layout(), "EQ Band 1 Gain", ControlKind::Integer),
            Unit::Db
        );
        assert_eq!(
            unit_for(&layout(), "EQ Band 1 Frequency", ControlKind::Integer),
            Unit::Hz
        );
        assert_eq!(
            unit_for(&layout(), "Gate Capture Switch", ControlKind::Boolean),
            Unit::Bool
        );
        assert_eq!(
            unit_for(&layout(), "Streaming Mic Level Steps", ControlKind::Integer),
            Unit::Steps,
            "a Steps control with an id conversion stays in steps"
        );
        assert_eq!(
            unit_for(&layout(), "Game EQ Band 1 Gain", ControlKind::Integer),
            Unit::Db,
            "a control the layout does not claim takes the map's conversion"
        );
        assert_eq!(
            unit_for(&layout(), "Game EQ Band 1 Frequency", ControlKind::Integer),
            Unit::Hz
        );
        assert_eq!(
            unit_for(&layout(), "Game EQ Band 1 Q", ControlKind::Integer),
            Unit::Q
        );
        assert_eq!(
            unit_for(&layout(), "Not In The Map", ControlKind::Boolean),
            Unit::Bool,
            "an unknown control falls back to its ALSA kind"
        );
    }

    #[test]
    fn every_control_is_read_and_an_unreadable_one_is_unknown() {
        let infos = vec![
            info("Gate", ControlKind::Integer, true),
            info("Meter Mic In", ControlKind::Integer, false),
        ];
        let backend = FakeAlsa::new(infos.clone());
        backend.set_value("Gate", 42);

        let controls = read_controls(&backend, infos, &layout());

        assert_eq!(controls.len(), 2);
        assert_eq!(controls[0].name(), "Gate");
        assert_eq!(controls[0].value, ControlValue::Known(42));
        assert_eq!(controls[1].name(), "Meter Mic In");
        assert_eq!(controls[1].value, ControlValue::Unknown);
    }

    #[test]
    fn a_reload_started_before_a_disconnect_cannot_install_the_card() {
        let mut reloads = ReloadTracker::default();
        let generation = reloads.begin();

        // The card goes away while the read is in flight.
        reloads.invalidate();

        assert_eq!(
            reloads.completion(generation, 2, Some(2)),
            ReloadCompletion::Stale,
            "a completion from before the disconnect must not install the card"
        );
    }

    #[test]
    fn a_reload_completed_after_the_card_vanished_is_rejected() {
        let mut reloads = ReloadTracker::default();
        let generation = reloads.begin();

        // The watcher event has not arrived yet, but the card is gone.
        assert_eq!(
            reloads.completion(generation, 2, None),
            ReloadCompletion::CardGone
        );
    }

    #[test]
    fn a_transient_failure_is_retried_and_the_retry_installs_the_card() {
        let mut reloads = ReloadTracker::default();
        let first = reloads.begin();

        // The first read fails while the card is still present.
        assert!(reloads.failed(first));
        let (retry_generation, delay) = reloads.retry().expect("a retry is scheduled");
        assert_eq!(delay, RETRY_FIRST);

        // The timer fires, the retry starts, and the second read succeeds.
        assert!(reloads.retry_due(retry_generation));
        let second = reloads.begin();
        assert_ne!(second, first);
        assert_eq!(
            reloads.completion(second, 2, Some(2)),
            ReloadCompletion::Accepted
        );
        assert_eq!(reloads.failures, 0, "a success forgets the failures");
    }

    #[test]
    fn the_retry_backoff_is_short_and_capped() {
        assert_eq!(retry_delay(1), RETRY_FIRST);
        assert_eq!(retry_delay(2), RETRY_MAX);
        assert_eq!(retry_delay(9), RETRY_MAX);
        assert!(RETRY_MAX <= Duration::from_secs(2));
    }

    #[test]
    fn a_retry_timer_is_ignored_after_the_card_goes_away() {
        let mut reloads = ReloadTracker::default();
        let first = reloads.begin();
        assert!(reloads.failed(first));
        let (retry_generation, _) = reloads.retry().expect("a retry is scheduled");

        reloads.invalidate();

        assert!(
            !reloads.retry_due(retry_generation),
            "a retry scheduled before the disconnect must not run"
        );
    }

    #[test]
    fn a_lost_device_event_from_the_old_backend_is_ignored_after_a_reload() {
        let old = WriteOutcome::DeviceLost {
            generation: 1,
            name: "Gate".to_string(),
            error: "ENODEV".to_string(),
        };
        let current = WriteOutcome::DeviceLost {
            generation: 2,
            name: "Gate".to_string(),
            error: "ENODEV".to_string(),
        };

        assert!(
            !outcome_is_current(&old, 2),
            "a delayed failure from the replaced backend must not disconnect the new card"
        );
        assert!(
            outcome_is_current(&current, 2),
            "the current backend's failure still forces a reload"
        );
    }

    #[test]
    fn a_write_issued_before_a_reset_no_longer_describes_the_device() {
        let mut writes = WriteEpoch::default();
        writes.issue("Gate");
        assert!(writes.accepts("Gate"));

        writes.reset();

        assert!(
            !writes.accepts("Gate"),
            "the driver reloaded its persisted settings; that write is stale"
        );
        assert!(!writes.accepts("Gate Threshold"));

        writes.issue("Gate");
        assert!(
            writes.accepts("Gate"),
            "a write issued after the reset describes the device again"
        );
        assert!(
            !writes.accepts("Gate Threshold"),
            "writing one control after the reset says nothing about the others"
        );
    }
}
