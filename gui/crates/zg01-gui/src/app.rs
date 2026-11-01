//! The application component: the shell and the device state machine.
//!
//! Startup discovers the ZG01 card, reads every control into the model and
//! shows the shell. A watcher reports the connection transitions, so
//! unplugging the card shows the connection view and plugging it back in
//! reloads the controls. The Mixage screen is rebuilt from the controls
//! whenever they are read; the Micro and Avancé bodies are placeholders
//! until their own tasks replace them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::Duration;

use gtk4::prelude::*;
use relm4::prelude::*;
use zg01_core::alsa::{
    find_card, AlsaBackend, AlsaError, CardInfo, ControlInfo, ControlKind, RealAlsa,
};
use zg01_core::device::{spawn_watcher, DeviceEvent};
use zg01_core::layout::{load_layout_str, Layout, LAYOUT_TOML};
use zg01_core::meters::{spawn_poller, MeterSnapshot, METER_INTERVAL};
use zg01_core::mixage::{MeterId, MixageModel};
use zg01_core::model::{AppState, Control, ControlValue, DeviceState, Unit};
use zg01_core::writer::{ControlWriter, WriteOutcome};

use crate::ui::{MixageEvent, MixageScreen};
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
    pub fn title(self) -> &'static str {
        match self {
            Page::Mixage => "Mixage",
            Page::Micro => "Micro",
            Page::Advanced => "Avancé",
        }
    }

    /// What the screen shows, until its own task fills it in.
    pub fn description(self) -> &'static str {
        match self {
            Page::Mixage => "Les trois départs de stream, la sortie stream et les mètres.",
            Page::Micro => "La chaîne micro : GATE, COMP, EQ et LIMITEUR.",
            Page::Advanced => "Tous les contrôles ALSA que les écrans curatés ne couvrent pas.",
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
    /// The coalescing writer of the connected card.
    writer: Option<ControlWriter>,
    /// The stop flag of the write-outcome pump, when it runs.
    writer_stop: Option<Arc<AtomicBool>>,
    /// The stop flag of the meter pump, when it runs.
    meters_stop: Option<Arc<AtomicBool>>,
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
    /// The control writer finished a write.
    WriteOutcome(WriteOutcome),
    /// The meter pump read the four meters.
    Meters(MeterSnapshot),
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

        spawn_device_watcher(sender.clone());

        let mut model = App {
            state,
            backend,
            page: Page::Mixage,
            mixage: None,
            writer: None,
            writer_stop: None,
            meters_stop: None,
            shell,
            root: root.clone(),
            reloads: ReloadTracker::default(),
        };

        if model.state.device.is_connected() {
            model.install_mixage(sender.clone());
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
                self.page = page;
                if self.state.device.is_connected() {
                    self.shell.set_page(page);
                }
            }
            Msg::About => {
                self.shell
                    .present_about(&self.root, self.state.device.card().map(|card| card.index));
            }
            Msg::Save => self.save(sender),
            Msg::SaveFinished(Ok(())) => {
                self.shell.toast("Réglages sauvegardés sur le ZG01");
            }
            Msg::SaveFinished(Err(err)) => {
                tracing::warn!("the save action failed: {err}");
                self.shell.toast(&format!("Échec de la sauvegarde : {err}"));
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
            },
            Msg::WriteOutcome(outcome) => self.write_outcome(outcome),
            Msg::Meters(snapshot) => self.update_meters(snapshot),
        }
    }
}

impl App {
    /// Writes the Save action to the device, off the GTK thread.
    fn save(&self, sender: ComponentSender<Self>) {
        if !self.state.can_save() {
            self.shell
                .toast("Sauvegarde impossible : valeurs inconnues ou carte absente");
            return;
        }
        let Some(backend) = self.backend.clone() else {
            self.shell.toast("Aucun ZG01 connecté");
            return;
        };

        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-save".to_string())
            .spawn(move || {
                let result = backend
                    .write("Save to ZG01", 1)
                    .map_err(|err| err.to_string());
                sender.input(Msg::SaveFinished(result));
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the save thread: {err}");
            self.shell.toast("Impossible de lancer la sauvegarde");
        }
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
        if !self.state.device.is_connected() {
            return;
        }
        self.page = Page::Micro;
        self.shell.set_page(Page::Micro);
    }

    /// Queues one control write on the coalescing writer.
    fn write_control(&mut self, name: &str, value: i64) {
        match &self.writer {
            Some(writer) => writer.set(name, value),
            None => {
                self.shell.toast("Aucun ZG01 connecté");
                self.revert_control(name);
            }
        }
    }

    /// Puts a widget back to the value the model still holds.
    fn revert_control(&self, name: &str) {
        if let Some(screen) = &self.mixage {
            let value = self
                .state
                .control(name)
                .map(|control| control.value)
                .unwrap_or(ControlValue::Unknown);
            screen.set_control(name, value);
        }
    }

    /// Handles the outcome of a control write.
    fn write_outcome(&mut self, outcome: WriteOutcome) {
        match outcome {
            WriteOutcome::Ok { name, value } => {
                if let Some(control) = self.state.control_mut(&name) {
                    control.value = ControlValue::Known(value);
                }
                if let Some(screen) = &self.mixage {
                    screen.set_control(&name, ControlValue::Known(value));
                }
            }
            WriteOutcome::Err { name, error } => {
                tracing::warn!("could not write the control {name:?}: {error}");
                self.shell
                    .toast(&format!("Écriture de {name} impossible : {error}"));
                self.revert_control(&name);
            }
        }
    }

    /// Installs the meter readings on the Mixage screen.
    fn update_meters(&self, snapshot: MeterSnapshot) {
        let Some(screen) = &self.mixage else {
            return;
        };
        for id in [
            MeterId::MicIn,
            MeterId::MicOut,
            MeterId::PcIn,
            MeterId::StreamMix,
        ] {
            screen.set_meter(id, snapshot.value(id.index()));
        }
    }

    /// Shows the connection view when the card goes away.
    ///
    /// Always invalidates the reload tracker, even when the state is
    /// already disconnected: a read started before this event must not
    /// install the card afterwards.
    fn disconnect(&mut self) {
        self.reloads.invalidate();
        self.show_disconnected(true);
    }

    /// Drops the controls and shows the connection view.
    fn show_disconnected(&mut self, notify: bool) {
        let was_connected = self.state.device.is_connected();
        let layout = self.state.layout.clone();
        self.state = AppState::new(DeviceState::Disconnected, Vec::new(), layout);
        self.backend = None;
        self.stop_writer();
        self.stop_meters();
        self.mixage = None;
        self.shell.show_disconnected();
        self.shell.set_save_enabled(false);
        if notify && was_connected {
            self.shell.toast("ZG01 déconnecté");
        }
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
                            .toast("ZG01 détecté, mais ses contrôles sont illisibles");
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

        if let Some(card) = self.state.device.card() {
            self.shell.show_connected(card);
        }
        self.shell.set_save_enabled(self.state.can_save());
        self.install_mixage(sender.clone());
        self.start_writer(&sender);
        self.start_meters(&sender);
        self.shell.set_page(self.page);
        self.shell.toast("ZG01 connecté");
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

/// The unit of a control: the layout table when it claims it, a sensible
/// default otherwise.
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
    match kind {
        ControlKind::Boolean => Unit::Bool,
        ControlKind::Integer | ControlKind::Bytes => Unit::Steps,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        read_controls, retry_delay, unit_for, Page, ReloadCompletion, ReloadTracker, RETRY_FIRST,
        RETRY_MAX,
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
            "a control the table does not claim falls back to its kind"
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
}
