//! The Mixage screen: three stream sends, the stream output and the
//! preset bar.
//!
//! The screen is pure presentation: it is built from a [`MixageModel`] and
//! the values of the [`AppState`], reports a fader or toggle change through
//! [`MixageEvent::Write`] and never touches ALSA itself. The application
//! owns the write worker and the meter poller and pushes the results back
//! with [`MixageScreen::set_control`] and [`MixageScreen::set_meter`].
//!
//! The send levels are 0–127 device steps with no established dB mapping,
//! so they are shown as steps, never as decibels. The meters are card
//! meters: a meter whose control cannot be read stays dashed, it never
//! shows a fake zero. A writable value that is unknown — after a reset —
//! is drawn dashed as well, but the control stays usable: setting it
//! makes that one value known again.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::glib::Propagation;
use gtk4::prelude::*;
use gtk4::{self as gtk, Orientation};
use zg01_core::model::{AppState, ControlValue, Unit};

use super::mixage_model::{MeterId, MixageModel, StreamOutputModel, StripId, StripModel};
use crate::widgets::{Fader, FaderSpec, Meter, MeterSpec};

/// The full scale of the meter controls, in device steps.
const METER_FULL_SCALE: f64 = 127.0;

/// What the screen asks the application to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MixageEvent {
    /// The MIC strip was clicked: open the Micro screen.
    OpenMic,
    /// A fader or a toggle asks for a new value.
    Write {
        /// The ALSA name of the control.
        name: String,
        /// The value to write.
        value: i64,
    },
    /// The user asked to load a `.zgpreset` file.
    LoadPreset,
}

/// One card meter and the reading slot it uses.
struct Metered {
    id: MeterId,
    meter: Meter,
}

/// A pill switch, its control and the guard that keeps a programmatic
/// update from looking like a user change.
struct Toggle {
    name: String,
    switch: gtk::Switch,
    guard: Rc<Cell<bool>>,
    /// Whether the driver accepts writes; a read-only toggle stays
    /// disabled, an unknown writable one stays usable.
    writable: bool,
}

impl Toggle {
    /// Shows `value`.
    ///
    /// An unknown value is shown greyed with the last position; a writable
    /// toggle stays sensitive so the user can set it after a reset.
    fn set(&self, value: ControlValue) {
        match value {
            ControlValue::Known(value) => {
                self.guard.set(true);
                self.switch.set_active(value != 0);
                self.guard.set(false);
                self.switch.remove_css_class("unknown-value");
            }
            ControlValue::Unknown => self.switch.add_css_class("unknown-value"),
        }
        self.switch.set_sensitive(self.writable);
    }
}

/// One stream send fader and the control it writes.
struct Level {
    /// The ALSA name of the fader.
    name: String,
    /// The fader itself.
    fader: Fader,
    /// Whether the driver accepts writes; an unknown writable fader stays
    /// usable so a reset can be recovered by writing a value.
    writable: bool,
}

/// One stream send strip.
struct StripWidgets {
    /// The ALSA name of the fader and the fader itself.
    level: Option<Level>,
    /// The card meter of the strip, when it has one.
    meter: Option<Metered>,
    /// The mute switch.
    mute: Option<Toggle>,
    /// The FX switch.
    fx: Option<Toggle>,
}

/// The stream output block.
struct StreamOutputWidgets {
    /// The stream mix meter.
    meter: Option<Metered>,
    /// The USB, HDMI and monitor switches.
    toggles: Vec<Toggle>,
}

/// The Mixage screen.
pub(crate) struct MixageScreen {
    root: gtk::Box,
    strips: Vec<StripWidgets>,
    output: StreamOutputWidgets,
    /// The preset bar label naming the last applied preset.
    preset_name: gtk::Label,
}

impl MixageScreen {
    /// Builds the screen from the model and the current control values.
    pub(crate) fn new(
        model: &MixageModel,
        state: &AppState,
        on_event: impl Fn(MixageEvent) + 'static,
    ) -> Self {
        let on_event: Rc<dyn Fn(MixageEvent)> = Rc::new(on_event);

        let root = gtk::Box::new(Orientation::Vertical, 14);
        root.set_margin_top(24);
        root.set_margin_bottom(24);
        root.set_margin_start(24);
        root.set_margin_end(24);

        let title = gtk::Label::new(Some("Mixage"));
        title.add_css_class("page-title");
        title.set_xalign(0.0);
        root.append(&title);

        let subtitle = gtk::Label::new(Some("Départs stream, sortie stream et mètres."));
        subtitle.add_css_class("page-subtitle");
        subtitle.set_xalign(0.0);
        root.append(&subtitle);

        let panel = gtk::Box::new(Orientation::Horizontal, 0);
        panel.add_css_class("panel");
        panel.set_margin_top(6);

        let mut strips = Vec::new();
        for (index, strip) in model.strips.iter().enumerate() {
            if index > 0 {
                panel.append(&gtk::Separator::new(Orientation::Vertical));
            }
            let (node, widgets) = build_strip(strip, state, &on_event);
            panel.append(&node);
            strips.push(widgets);
        }

        panel.append(&gtk::Separator::new(Orientation::Vertical));
        let (node, output) = build_output(&model.output, state, &on_event);
        panel.append(&node);
        root.append(&panel);

        let (preset_bar, preset_name) = build_preset_bar(state, &on_event);
        root.append(&preset_bar);

        MixageScreen {
            root,
            strips,
            output,
            preset_name,
        }
    }

    /// The widget to install in the content stack.
    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Names the preset that was just applied in the preset bar.
    pub(crate) fn set_preset_name(&self, name: &str) {
        self.preset_name.set_text(name);
    }

    /// Installs one meter reading, in 0–127 device steps.
    ///
    /// `None` is an unreadable meter: the widget draws the dashed unknown
    /// state, not a zero.
    pub(crate) fn set_meter(&self, id: MeterId, steps: Option<i64>) {
        let level = level_from_steps(steps);
        for strip in &self.strips {
            if let Some(metered) = &strip.meter {
                if metered.id == id {
                    metered.meter.set_levels(level, None, 0.0);
                }
            }
        }
        if let Some(metered) = &self.output.meter {
            if metered.id == id {
                metered.meter.set_levels(level, None, 0.0);
            }
        }
    }

    /// Installs a value the device reported, or its failure.
    pub(crate) fn set_control(&self, name: &str, value: ControlValue) {
        for strip in &self.strips {
            if let Some(level) = &strip.level {
                if level.name == name {
                    level.fader.set_value(value);
                    level.fader.set_sensitive(level.writable);
                }
            }
            for toggle in strip.mute.iter().chain(strip.fx.iter()) {
                if toggle.name == name {
                    toggle.set(value);
                }
            }
        }
        for toggle in &self.output.toggles {
            if toggle.name == name {
                toggle.set(value);
            }
        }
    }
}

/// Builds one send strip.
fn build_strip(
    strip: &StripModel,
    state: &AppState,
    on_event: &Rc<dyn Fn(MixageEvent)>,
) -> (gtk::Box, StripWidgets) {
    let root = gtk::Box::new(Orientation::Vertical, 10);
    root.add_css_class("strip");
    root.set_width_request(150);
    root.set_hexpand(true);

    root.append(&build_strip_header(strip, on_event));

    let body = gtk::Box::new(Orientation::Horizontal, 12);
    body.set_halign(gtk::Align::Center);
    body.set_valign(gtk::Align::Center);
    body.set_vexpand(true);

    let meter = strip.meter.map(|id| {
        let meter = Meter::new(MeterSpec {
            label: "CARD".to_string(),
            card: meter_level(state, id),
            // v1 has no PipeWire source: the Linux bar is absent, not
            // unknown, so `linux_present` stays false.
            linux: None,
            peak: 0.0,
            linux_present: false,
        });
        body.append(meter.widget());
        Metered { id, meter }
    });

    let level = strip.level.as_ref().map(|name| {
        let fader = Fader::new(fader_spec(state, name, &strip.title), {
            let emit = on_event.clone();
            move |output| {
                emit(MixageEvent::Write {
                    name: output.name,
                    value: output.value,
                });
            }
        });
        // An unknown value is drawn dashed, but a writable fader stays
        // usable: after a reset the user sets it again, which makes that
        // one value known.
        let writable = state
            .control(name)
            .is_some_and(|control| control.is_writable());
        fader.set_sensitive(writable);
        body.append(fader.widget());
        Level {
            name: name.clone(),
            fader,
            writable,
        }
    });

    if meter.is_none() && level.is_none() {
        body.append(&unavailable_label());
    }
    root.append(&body);

    let tags = gtk::Box::new(Orientation::Horizontal, 8);
    tags.set_halign(gtk::Align::Center);
    let mute = strip.mute.as_ref().map(|name| {
        let (toggle, row) = build_toggle(state, name, "MUTE", on_event);
        tags.append(&row);
        toggle
    });
    let fx = strip.fx.as_ref().map(|name| {
        let (toggle, row) = build_toggle(state, name, "FX", on_event);
        tags.append(&row);
        toggle
    });
    root.append(&tags);

    (
        root,
        StripWidgets {
            level,
            meter,
            mute,
            fx,
        },
    )
}

/// Builds the channel title and, for MIC, the link to the Micro screen.
fn build_strip_header(strip: &StripModel, on_event: &Rc<dyn Fn(MixageEvent)>) -> gtk::Box {
    let title = gtk::Label::new(Some(&strip.title));
    title.add_css_class("strip-title");
    title.set_xalign(0.5);

    let subtitle = match strip.id {
        StripId::Mic => "GATE › COMP › EQ › LIMIT",
        StripId::Voice => "VOICE · USB",
        StripId::Game => "GAME · HDMI/USB",
    };

    let header = gtk::Box::new(Orientation::Vertical, 2);
    header.set_halign(gtk::Align::Center);

    if strip.id == StripId::Mic {
        let hint = gtk::Label::new(Some(subtitle));
        hint.add_css_class("strip-subtitle");
        let content = gtk::Box::new(Orientation::Vertical, 2);
        content.append(&title);
        content.append(&hint);

        let button = gtk::Button::new();
        button.add_css_class("strip-title-button");
        button.set_has_frame(false);
        button.set_tooltip_text(Some("Ouvrir la chaîne micro"));
        button.set_child(Some(&content));
        let emit = on_event.clone();
        button.connect_clicked(move |_| emit(MixageEvent::OpenMic));
        header.append(&button);
    } else {
        header.append(&title);
        let hint = gtk::Label::new(Some(subtitle));
        hint.add_css_class("strip-subtitle");
        header.append(&hint);
    }

    header
}

/// Builds the STREAM OUT block: the stream meter and the three switches.
fn build_output(
    output: &StreamOutputModel,
    state: &AppState,
    on_event: &Rc<dyn Fn(MixageEvent)>,
) -> (gtk::Box, StreamOutputWidgets) {
    let root = gtk::Box::new(Orientation::Vertical, 10);
    root.add_css_class("strip");
    root.add_css_class("output-strip");
    root.set_width_request(190);

    let title = gtk::Label::new(Some("STREAM OUT"));
    title.add_css_class("strip-title");
    root.append(&title);

    let body = gtk::Box::new(Orientation::Horizontal, 14);
    body.set_valign(gtk::Align::Center);
    body.set_vexpand(true);

    let meter = output.meter.map(|id| {
        let meter = Meter::new(MeterSpec {
            label: "CARD".to_string(),
            card: meter_level(state, id),
            linux: None,
            peak: 0.0,
            linux_present: false,
        });
        body.append(meter.widget());
        Metered { id, meter }
    });

    let switches = gtk::Box::new(Orientation::Vertical, 8);
    switches.set_valign(gtk::Align::Center);
    let mut toggles = Vec::new();
    for (name, label) in [
        (output.usb.as_deref(), "USB"),
        (output.hdmi.as_deref(), "HDMI"),
        (output.monitor.as_deref(), "MON"),
    ] {
        if let Some(name) = name {
            let (toggle, row) = build_toggle(state, name, label, on_event);
            switches.append(&row);
            toggles.push(toggle);
        }
    }
    if toggles.is_empty() {
        switches.append(&unavailable_label());
    }
    body.append(&switches);
    root.append(&body);

    (root, StreamOutputWidgets { meter, toggles })
}

/// Builds one labelled switch.
fn build_toggle(
    state: &AppState,
    name: &str,
    label: &str,
    on_event: &Rc<dyn Fn(MixageEvent)>,
) -> (Toggle, gtk::Box) {
    let control = state.control(name);
    let value = control
        .map(|control| control.value)
        .unwrap_or(ControlValue::Unknown);
    let writable = control.is_some_and(|control| control.is_writable());

    let switch = gtk::Switch::new();
    switch.set_valign(gtk::Align::Center);
    match value {
        ControlValue::Known(on) => switch.set_active(on != 0),
        ControlValue::Unknown => switch.add_css_class("unknown-value"),
    }
    switch.set_sensitive(writable);

    let guard = Rc::new(Cell::new(false));
    let handler_guard = guard.clone();
    let emit = on_event.clone();
    let control_name = name.to_string();
    switch.connect_state_set(move |_, active| {
        if !handler_guard.get() {
            emit(MixageEvent::Write {
                name: control_name.clone(),
                value: i64::from(active),
            });
        }
        Propagation::Proceed
    });

    let tag = gtk::Label::new(Some(label));
    tag.add_css_class("tag-label");
    let row = gtk::Box::new(Orientation::Horizontal, 6);
    row.set_halign(gtk::Align::Start);
    row.append(&tag);
    row.append(&switch);

    (
        Toggle {
            name: name.to_string(),
            switch,
            guard,
            writable,
        },
        row,
    )
}

/// The preset bar: the load action and the last applied preset.
fn build_preset_bar(
    state: &AppState,
    on_event: &Rc<dyn Fn(MixageEvent)>,
) -> (gtk::Box, gtk::Label) {
    let bar = gtk::Box::new(Orientation::Horizontal, 12);
    bar.add_css_class("panel");
    bar.add_css_class("preset-bar");

    let label = gtk::Label::new(Some("Preset"));
    label.add_css_class("tag-label");
    bar.append(&label);

    let name = gtk::Label::new(Some("Aucun preset chargé"));
    name.add_css_class("preset-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    bar.append(&name);

    let recent = gtk::Button::with_label("Récent");
    recent.set_sensitive(false);
    recent.set_tooltip_text(Some("La liste des presets récents arrive plus tard"));
    bar.append(&recent);

    let load = gtk::Button::with_label("Charger");
    load.add_css_class("suggested-action");
    load.set_sensitive(state.device.is_connected());
    load.set_tooltip_text(Some(if state.device.is_connected() {
        "Charger un preset .zgpreset et appliquer ses réglages"
    } else {
        "Le chargement d'un preset demande un ZG01 connecté"
    }));
    let emit = on_event.clone();
    load.connect_clicked(move |_| emit(MixageEvent::LoadPreset));
    bar.append(&load);

    (bar, name)
}

/// A muted fallback for a strip or a block with no usable control.
fn unavailable_label() -> gtk::Label {
    let label = gtk::Label::new(Some("Indisponible"));
    label.add_css_class("fader-value");
    label.add_css_class("unknown");
    label
}

/// The fader spec of one send level.
fn fader_spec(state: &AppState, name: &str, label: &str) -> FaderSpec {
    let control = state.control(name);
    let (min, max, step, value) = match control {
        Some(control) => (
            control.info.min,
            control.info.max,
            control.info.step,
            control.value,
        ),
        None => (0, 127, 1, ControlValue::Unknown),
    };
    FaderSpec {
        name: name.to_string(),
        label: label.to_string(),
        // Stream levels are device steps: no dB conversion exists.
        unit: Unit::Steps,
        min,
        max,
        step,
        value,
    }
}

/// The card reading of a meter, normalised to `0.0..=1.0`.
fn meter_level(state: &AppState, id: MeterId) -> Option<f64> {
    level_from_steps(
        state
            .control(id.control())
            .and_then(|control| control.value()),
    )
}

/// Normalises 0–127 device steps to the meter's `0.0..=1.0`.
fn level_from_steps(steps: Option<i64>) -> Option<f64> {
    steps.map(|steps| (steps as f64 / METER_FULL_SCALE).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::level_from_steps;

    #[test]
    fn meter_steps_are_normalised_to_the_unit_interval() {
        assert_eq!(level_from_steps(None), None);
        assert_eq!(level_from_steps(Some(0)), Some(0.0));
        assert_eq!(level_from_steps(Some(127)), Some(1.0));
        let half = level_from_steps(Some(64)).expect("a reading");
        assert!((half - 64.0 / 127.0).abs() < 1e-12);
        assert_eq!(level_from_steps(Some(-1)), Some(0.0));
        assert_eq!(level_from_steps(Some(999)), Some(1.0));
    }
}
