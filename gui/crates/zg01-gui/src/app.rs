//! The application component: the shell and the device state machine.
//!
//! Startup discovers the ZG01 card, reads every control into the model and
//! shows the shell. A watcher reports the connection transitions, so
//! unplugging the card shows the connection view and plugging it back in
//! reloads the controls. The Mixage, Micro and Avancé bodies are
//! placeholders until their own tasks replace them.

use std::sync::Arc;
use std::time::Duration;

use gtk4::prelude::*;
use relm4::prelude::*;
use zg01_core::alsa::{
    find_card, AlsaBackend, AlsaError, CardInfo, ControlInfo, ControlKind, RealAlsa,
};
use zg01_core::device::{spawn_watcher, DeviceEvent};
use zg01_core::layout::{load_layout_str, Layout, LAYOUT_TOML};
use zg01_core::model::{AppState, Control, ControlValue, DeviceState, Unit};

use crate::window::Shell;

/// Title of the main window.
const APP_TITLE: &str = "ZG01 Control";

/// Default width of the main window, in logical pixels.
const WINDOW_WIDTH: i32 = 1100;

/// Default height of the main window, in logical pixels.
const WINDOW_HEIGHT: i32 = 680;

/// How often the device watcher polls the card list.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);

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
    /// The window, used as the parent of the About dialog.
    root: adw::ApplicationWindow,
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
    ReloadFinished(Result<Loaded, String>),
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

        match state.device.card() {
            Some(card) => {
                shell.show_connected(card);
                shell.set_page(Page::Mixage);
            }
            None => shell.show_disconnected(),
        }
        shell.set_save_enabled(state.can_save());

        spawn_device_watcher(sender.clone());

        let model = App {
            state,
            backend,
            page: Page::Mixage,
            shell,
            root: root.clone(),
        };

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
            Msg::ReloadFinished(Ok(loaded)) => self.reloaded(loaded),
            Msg::ReloadFinished(Err(err)) => {
                tracing::warn!("could not read the ZG01: {err}");
                self.shell
                    .toast("ZG01 détecté, mais ses contrôles sont illisibles");
            }
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

    /// Shows the connection view when the card goes away.
    fn disconnect(&mut self) {
        if !self.state.device.is_connected() {
            return;
        }
        let layout = self.state.layout.clone();
        self.state = AppState::new(DeviceState::Disconnected, Vec::new(), layout);
        self.backend = None;
        self.shell.show_disconnected();
        self.shell.set_save_enabled(false);
        self.shell.toast("ZG01 déconnecté");
    }

    /// Reads a card that just appeared, off the GTK thread.
    fn connect(&self, card: CardInfo, sender: ComponentSender<Self>) {
        if self.state.device.card().map(|current| current.index) == Some(card.index) {
            return;
        }

        let layout = self.state.layout.clone();
        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("zg01-reload".to_string())
            .spawn(move || {
                let result = load_device(&layout);
                sender.input(Msg::ReloadFinished(result));
            });
        if let Err(err) = spawned {
            tracing::warn!("could not start the reload thread: {err}");
        }
    }

    /// Installs the state of a card that was read.
    fn reloaded(&mut self, loaded: Loaded) {
        let layout = self.state.layout.clone();
        self.state = AppState::new(DeviceState::Connected(loaded.card), loaded.controls, layout);
        self.backend = Some(loaded.backend);

        if let Some(card) = self.state.device.card() {
            self.shell.show_connected(card);
        }
        self.shell.set_save_enabled(self.state.can_save());
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
    use super::{read_controls, unit_for, Page};
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
}
