//! The application component: the shell and the device state machine.
//!
//! Startup discovers the ZG01 card, reads every control into the model and
//! shows the shell. A watcher reports the connection transitions, so
//! unplugging the card shows the connection view and plugging it back in
//! reloads the controls. The Mixage, Micro and Avancé screens are rebuilt
//! from the controls whenever they are read.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
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
use zg01_core::meters::{spawn_poller, MeterSnapshot, METER_INTERVAL};
use zg01_core::mic::MicModel;
use zg01_core::mixage::{MeterId, MixageModel};
use zg01_core::model::{
    apply_reset, AppState, Control, ControlValue, DeviceState, Unit, RESET_ACTION, SAVE_ACTION,
};
use zg01_core::preset::{apply_plan, ApplyReport};
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
    /// The epoch of the control writes, used to ignore outcomes that
    /// predate a reset.
    writes: WriteEpoch,
    /// The window, used as the parent of the About dialog.
    root: adw::ApplicationWindow,
    /// Serialises the asynchronous reloads with the presence events.
    reloads: ReloadTracker,
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
    /// A save finished.
    SaveFinished(Result<(), String>),
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
    /// The preset preview handed a plan to apply.
    PresetChosen(PresetChoice),
    /// Applying a preset finished.
    PresetApplied {
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

        let (state, backend) = match load_device(&layout) {
            Ok(loaded) => {
                tracing::info!(
                    "reading card hw:{}: {} controls",
                    loaded.card.index,
                    loaded.controls.len()
                );
                let state =
                    AppState::new(DeviceState::Connected(loaded.card), loaded.controls, layout);
                (state, Some(loaded.backend))
            }
            Err(err) => {
                tracing::warn!("no ZG01 card available: {err}");
                (
                    AppState::new(DeviceState::Disconnected, Vec::new(), layout),
                    None,
                )
            }
        };

        if let Some(card) = state.device.card() {
            shell.show_connected(card);
        } else {
            shell.show_disconnected();
        }
        shell.set_save_enabled(state.can_save());
        shell.set_unknown(state.device.is_connected() && !state.can_save());

        spawn_device_watcher(sender.clone());

        let mut model = App {
            state,
            backend,
            page: Page::Mixage,
            mixage: None,
            mic: None,
            advanced: None,
            writer: None,
            writer_stop: None,
            meters_stop: None,
            writes: WriteEpoch::default(),
            shell,
            root: root.clone(),
            reloads: ReloadTracker::default(),
        };

        if model.state.device.is_connected() {
            model.install_mixage(sender.clone());
            model.install_micro(sender.clone());
            model.install_advanced(sender.clone());
            model.start_writer(&sender);
            model.start_meters(&sender);
            model.shell.set_page(Page::Mixage);
        }

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
            Msg::Reset => self.reset(),
            Msg::SaveFinished(Ok(())) => {
                self.shell.toast(&t("Settings saved to the ZG01"));
            }
            Msg::SaveFinished(Err(err)) => {
                tracing::warn!("the save action failed: {err}");
                self.shell
                    .toast(&tf("Save failed: {error}", &[("{error}", &err)]));
            }
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
            },
            Msg::Mic(MicEvent::Write { name, value }) => self.write_control(&name, value),
            Msg::Advanced(AdvancedEvent::Write { name, value }) => self.write_control(&name, value),
            Msg::WriteOutcome(outcome) => self.write_outcome(outcome),
            Msg::Meters(snapshot) => self.update_meters(snapshot),
            Msg::PresetChosen(choice) => self.apply_preset(choice, sender),
            Msg::PresetApplied { label, report } => self.preset_applied(&label, &report),
        }
    }
}

impl App {
    /// Writes the Save action to the device, off the GTK thread.
    fn save(&self, sender: ComponentSender<Self>) {
        if !self.state.can_save() {
            self.shell
                .toast(&t("Cannot save: unknown values or no card"));
            return;
        }
        let Some(backend) = self.backend.clone() else {
            self.shell.toast(&t("No ZG01 connected"));
            return;
        };

        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-save".to_string())
            .spawn(move || {
                let result = backend.write(SAVE_ACTION, 1).map_err(|err| err.to_string());
                sender.input(Msg::SaveFinished(result));
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the save thread: {err}");
            self.shell.toast(&t("Could not start the save"));
        }
    }

    /// Writes the Reset action and forgets every displayed value.
    ///
    /// `apply_reset` flushes the coalesced writes, writes and awaits the
    /// action, then marks every writable value unknown: the driver reloads
    /// its persisted settings without refreshing the cache. A failed
    /// action is reported and the values stay unknown rather than guessing
    /// that the reset did not happen.
    fn reset(&mut self) {
        let Some(writer) = self.writer.clone() else {
            self.shell.toast(&t("No ZG01 connected"));
            return;
        };
        // Every write issued before the reset is superseded by it; its
        // outcome must not mark a value known afterwards.
        self.writes.reset();
        match apply_reset(&mut self.state, &writer) {
            Ok(()) => self
                .shell
                .toast(&t("ZG01 reset — the displayed values are unknown")),
            Err(err) => {
                tracing::warn!("the reset action failed: {err}");
                self.shell
                    .toast(&tf("Reset failed: {error}", &[("{error}", &err)]));
            }
        }
        self.refresh_controls();
        self.sync_shell_state();
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
        let (writer, outcomes) = ControlWriter::spawn(backend);
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
        let events = spawn_poller(backend, METER_INTERVAL);
        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-meter-events".to_string())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match events.recv_timeout(PUMP_TICK) {
                        Ok(snapshot) => sender.input(Msg::Meters(snapshot)),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the meter event thread: {err}");
        }
    }

    /// Asks the meter pump to stop; the poller stops when its receiver is
    /// dropped.
    fn stop_meters(&mut self) {
        if let Some(stop) = self.meters_stop.take() {
            stop.store(true, Ordering::Relaxed);
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
        let Some(writer) = self.writer.clone() else {
            self.shell.toast(&t("No ZG01 connected"));
            return;
        };
        for apply in &choice.plan.applies {
            self.writes.issue(&apply.name);
        }

        let label = preset_label(&choice.block, &choice.name);
        let plan = choice.plan;
        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-preset".to_string())
            .spawn(move || {
                let report = apply_plan(&plan, &writer);
                sender.input(Msg::PresetApplied { label, report });
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the preset thread: {err}");
            self.shell.toast(&t("Could not load the preset"));
        }
    }

    /// Names the applied preset in the bar and shows the apply report.
    fn preset_applied(&self, label: &str, report: &ApplyReport) {
        tracing::info!(
            "preset {label:?}: {} applied, {} skipped, {} failed",
            report.applied.len(),
            report.skipped.len(),
            report.failed.len()
        );
        if !report.applied.is_empty() {
            if let Some(screen) = &self.mixage {
                screen.set_preset_name(label);
            }
        }
        present_preset_report(&self.root, label, report);
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
        self.shell.set_unknown(connected && !self.state.can_save());
        self.shell
            .set_device_lost(!connected && !self.state.controls.is_empty());
    }

    /// Handles the outcome of a control write.
    fn write_outcome(&mut self, outcome: WriteOutcome) {
        match outcome {
            // `apply_reset` handles the action outcome: success and failure
            // were already reported, and the action is not a value.
            WriteOutcome::Ok { name, .. } if name == RESET_ACTION => {
                tracing::debug!("the reset action is on the device");
            }
            WriteOutcome::Err { name, error } if name == RESET_ACTION => {
                tracing::debug!("the reset action failed: {error}");
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
    fn update_meters(&self, snapshot: MeterSnapshot) {
        for id in [
            MeterId::MicIn,
            MeterId::MicOut,
            MeterId::PcIn,
            MeterId::StreamMix,
        ] {
            if let Some(screen) = &self.mixage {
                screen.set_meter(id, snapshot.value(id.index()));
            }
            if let Some(screen) = &self.mic {
                screen.set_meter(id, snapshot.value(id.index()));
            }
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
        // The values stay visible, but nothing can be changed until the
        // card is back and its controls have been read again.
        if let Some(screen) = &self.mixage {
            screen.widget().set_sensitive(false);
        }
        if let Some(screen) = &self.mic {
            screen.widget().set_sensitive(false);
        }
        if let Some(screen) = &self.advanced {
            screen.widget().set_sensitive(false);
        }
        self.shell.show_device_lost();
        self.sync_shell_state();
        self.shell.toast(&t("ZG01 disconnected"));
    }

    /// Reads a card that just appeared, off the GTK thread.
    fn connect(&mut self, card: CardInfo, sender: ComponentSender<Self>) {
        if self.state.device.card().map(|current| current.index) == Some(card.index) {
            return;
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
                    self.disconnect();
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
        let layout = self.state.layout.clone();
        self.state = AppState::new(DeviceState::Connected(loaded.card), loaded.controls, layout);
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
        self.shell.toast(&t("ZG01 connected"));
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
    let controls = read_controls(backend.as_ref(), infos, layout);
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
        preset_label, read_controls, retry_delay, unit_for, Page, ReloadCompletion, ReloadTracker,
        WriteEpoch, RETRY_FIRST, RETRY_MAX,
    };
    use std::time::Duration;
    use zg01_core::alsa::{ControlInfo, ControlKind, FakeAlsa};
    use zg01_core::layout::{load_layout_str, LAYOUT_TOML};
    use zg01_core::model::{ControlValue, Unit};

    fn info(name: &str, kind: ControlKind, writable: bool) -> ControlInfo {
        ControlInfo {
            name: name.to_string(),
            kind,
            min: 0,
            max: 100,
            step: 0,
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
