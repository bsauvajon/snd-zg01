//! The Micro screen: GATE, COMP, LIMIT and the four-band EQ.
//!
//! The screen is pure presentation: it is built from a [`MicModel`] and the
//! values of the [`AppState`], reports a fader, toggle or editor change
//! through [`MicEvent::Write`] and never touches ALSA itself. The
//! application owns the write worker and the meter poller and pushes the
//! results back with [`MicScreen::set_control`] and
//! [`MicScreen::set_meter`].
//!
//! The chain is three faders, not four: `Mic Level Status` is read-only and
//! absent, and the per-block out gains live in the advanced expander. The
//! EQ has four bands; the shelf shapes are the existing `EQ Low Shape` and
//! `EQ High Shape`. A control the driver does not publish is hidden. The
//! MIC IN and MIC OUT meters flank the chain: a meter that cannot be read
//! stays dashed, it never shows a fake zero. A writable value that is
//! unknown — after a reset — is dashed too but stays usable, so the user
//! can set it again.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::glib::Propagation;
use gtk4::prelude::*;
use gtk4::{self as gtk, Orientation};
use zg01_core::mixage::MeterId;
use zg01_core::model::{format_value, AppState, ControlValue, Unit};

use super::mic_model::{BlockModel, EqModel, MicModel};
use crate::i18n::{t, tf};
use crate::widgets::{Fader, FaderSpec, Meter, MeterSpec};

/// The full scale of the meter controls, in device steps.
const METER_FULL_SCALE: f64 = 127.0;

/// What the editor shows while the device value is unknown.
const UNKNOWN_TEXT: &str = "—";

/// What the screen asks the application to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MicEvent {
    /// A fader, a toggle or an editor asks for a new value.
    Write {
        /// The ALSA name of the control.
        name: String,
        /// The value to write.
        value: i64,
    },
    /// Re-read the MIC state dump from the device.
    ReadState,
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
    /// Whether the driver accepts writes; an unknown writable switch stays
    /// usable so a reset can be recovered by writing a value.
    writable: bool,
}

impl Toggle {
    /// Shows `value`.
    ///
    /// An unknown value is shown greyed with the last position; a writable
    /// toggle stays sensitive.
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

/// One vertical fader and the control it writes.
struct FaderSlot {
    name: String,
    fader: Fader,
    /// Whether the driver accepts writes; an unknown writable fader stays
    /// usable.
    writable: bool,
}

/// A compact horizontal editor for a numbered control.
struct Editor {
    name: String,
    unit: Unit,
    scale: gtk::Scale,
    value_label: gtk::Label,
    guard: Rc<Cell<bool>>,
    root: gtk::Box,
    /// Whether the driver accepts writes.
    writable: bool,
}

impl Editor {
    /// Builds an editor for `name`, labelled `label`.
    fn new(state: &AppState, name: &str, label: &str, on_event: &Rc<dyn Fn(MicEvent)>) -> Self {
        let control = state.control(name);
        let (min, max, step, unit, value) = match control {
            Some(control) => (
                control.info.min,
                control.info.max,
                control.info.step,
                control.unit,
                control.value,
            ),
            None => (0, 100, 1, Unit::Steps, ControlValue::Unknown),
        };
        let writable = control.is_some_and(|control| control.is_writable());

        let root = gtk::Box::new(Orientation::Vertical, 2);
        root.add_css_class("mic-editor");

        let header = gtk::Box::new(Orientation::Horizontal, 6);
        let tag = gtk::Label::new(Some(label));
        tag.add_css_class("tag-label");
        tag.set_xalign(0.0);
        tag.set_hexpand(true);
        header.append(&tag);

        let value_label = gtk::Label::new(None);
        value_label.add_css_class("fader-value");
        value_label.set_xalign(1.0);
        header.append(&value_label);
        root.append(&header);

        let start = value.known().unwrap_or(min).clamp(min, max);
        let adjustment = gtk::Adjustment::new(
            start as f64,
            min as f64,
            max as f64,
            step.max(1) as f64,
            0.0,
            0.0,
        );
        let scale = gtk::Scale::new(Orientation::Horizontal, Some(&adjustment));
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.set_digits(0);
        scale.add_css_class("mic-editor-scale");
        root.append(&scale);

        let guard = Rc::new(Cell::new(false));
        let handler_guard = guard.clone();
        let emit = on_event.clone();
        let control_name = name.to_string();
        scale.connect_value_changed(move |scale| {
            if handler_guard.get() {
                return;
            }
            emit(MicEvent::Write {
                name: control_name.clone(),
                value: scale.value().round() as i64,
            });
        });

        let editor = Editor {
            name: name.to_string(),
            unit,
            scale,
            value_label,
            guard,
            root,
            writable,
        };
        editor.set(value);
        editor
    }

    /// Installs a value the device reported, or its failure.
    fn set(&self, value: ControlValue) {
        self.guard.set(true);
        match value {
            ControlValue::Known(value) => {
                self.scale.set_value(value as f64);
                self.value_label.set_text(&format_value(self.unit, value));
                self.value_label.remove_css_class("unknown");
            }
            ControlValue::Unknown => {
                self.value_label.set_text(UNKNOWN_TEXT);
                self.value_label.add_css_class("unknown");
            }
        }
        self.scale.set_sensitive(self.writable);
        self.guard.set(false);
    }
}

/// The Micro screen.
pub(crate) struct MicScreen {
    root: gtk::Box,
    meters: Vec<Metered>,
    toggles: Vec<Toggle>,
    faders: Vec<FaderSlot>,
    editors: Vec<Editor>,
}

impl MicScreen {
    /// Builds the screen from the model and the current control values.
    pub(crate) fn new(
        model: &MicModel,
        state: &AppState,
        on_event: impl Fn(MicEvent) + 'static,
    ) -> Self {
        let on_event: Rc<dyn Fn(MicEvent)> = Rc::new(on_event);

        let root = gtk::Box::new(Orientation::Vertical, 14);
        root.set_margin_top(24);
        root.set_margin_bottom(24);
        root.set_margin_start(24);
        root.set_margin_end(24);

        let title = gtk::Label::new(Some(&t("Micro")));
        title.add_css_class("page-title");
        title.set_xalign(0.0);

        let subtitle = gtk::Label::new(Some(&t("Processing chain: GATE › COMP › EQ › LIMITER.")));
        subtitle.add_css_class("page-subtitle");
        subtitle.set_xalign(0.0);

        let top = gtk::Box::new(Orientation::Horizontal, 12);
        let titles = gtk::Box::new(Orientation::Vertical, 4);
        titles.set_hexpand(true);
        titles.append(&title);
        titles.append(&subtitle);
        top.append(&titles);

        let read = gtk::Button::with_label(&t("Read from ZG01"));
        read.add_css_class("suggested-action");
        read.set_valign(gtk::Align::Center);
        read.connect_clicked({
            let on_event = on_event.clone();
            move |_| on_event(MicEvent::ReadState)
        });
        top.append(&read);
        root.append(&top);

        let stage = gtk::Box::new(Orientation::Horizontal, 16);
        stage.set_margin_top(6);
        stage.set_vexpand(true);

        let mut meters = Vec::new();

        let mic_in = build_meter(state, MeterId::MicIn, "MIC IN");
        stage.append(mic_in.meter.widget());
        meters.push(mic_in);

        let panel = gtk::Box::new(Orientation::Vertical, 16);
        panel.add_css_class("panel");
        panel.add_css_class("mic-panel");
        panel.set_hexpand(true);

        let mut toggles = Vec::new();
        let mut faders = Vec::new();
        let mut editors = Vec::new();
        panel.append(&build_chain(
            model,
            state,
            &on_event,
            &mut toggles,
            &mut faders,
            &mut editors,
        ));
        panel.append(&build_eq(
            &model.eq,
            state,
            &on_event,
            &mut toggles,
            &mut faders,
            &mut editors,
        ));
        stage.append(&panel);

        let mic_out = build_meter(state, MeterId::MicOut, "MIC OUT");
        stage.append(mic_out.meter.widget());
        meters.push(mic_out);

        // The Micro screen is taller than the default window: scroll it
        // instead of letting the content force a window larger than the
        // screen.
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&stage));
        root.append(&scroll);

        MicScreen {
            root,
            meters,
            toggles,
            faders,
            editors,
        }
    }

    /// The widget to install in the content stack.
    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Installs one meter reading, in 0–127 device steps.
    ///
    /// `None` is an unreadable meter: the widget draws the dashed unknown
    /// state, not a zero.
    pub(crate) fn set_meter(&self, id: MeterId, steps: Option<i64>) {
        let level = level_from_steps(steps);
        for metered in &self.meters {
            if metered.id == id {
                metered.meter.set_levels(level, None, 0.0);
            }
        }
    }

    /// Installs a value the device reported, or its failure.
    pub(crate) fn set_control(&self, name: &str, value: ControlValue) {
        for slot in &self.faders {
            if slot.name == name && slot.fader.value() != value {
                slot.fader.set_value(value);
                slot.fader.set_sensitive(slot.writable);
            }
        }
        for toggle in &self.toggles {
            if toggle.name == name {
                toggle.set(value);
            }
        }
        for editor in &self.editors {
            if editor.name == name {
                editor.set(value);
            }
        }
    }
}

/// Builds the three dynamics blocks side by side.
fn build_chain(
    model: &MicModel,
    state: &AppState,
    on_event: &Rc<dyn Fn(MicEvent)>,
    toggles: &mut Vec<Toggle>,
    faders: &mut Vec<FaderSlot>,
    editors: &mut Vec<Editor>,
) -> gtk::Box {
    let chain = gtk::Box::new(Orientation::Horizontal, 12);
    chain.set_homogeneous(true);
    for (title, block) in [
        ("GATE", &model.gate),
        ("COMP", &model.comp),
        ("LIMIT", &model.limiter),
    ] {
        chain.append(&build_block(
            title, block, state, on_event, toggles, faders, editors,
        ));
    }
    chain
}

/// Builds one dynamics block: its switch, its fader and its sub-parameters.
fn build_block(
    title: &str,
    block: &BlockModel,
    state: &AppState,
    on_event: &Rc<dyn Fn(MicEvent)>,
    toggles: &mut Vec<Toggle>,
    faders: &mut Vec<FaderSlot>,
    editors: &mut Vec<Editor>,
) -> gtk::Box {
    let card = gtk::Box::new(Orientation::Vertical, 10);
    card.add_css_class("mic-block");
    card.set_hexpand(true);

    let header = gtk::Box::new(Orientation::Horizontal, 8);
    let name = gtk::Label::new(Some(title));
    name.add_css_class("strip-title");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    header.append(&name);
    if let Some(control) = &block.toggle {
        let (toggle, row) = build_toggle(state, control, "ON", on_event);
        header.append(&row);
        toggles.push(toggle);
    }
    card.append(&header);

    if let Some(control) = &block.value {
        let fader = build_fader(state, control, &t("Level"), on_event);
        card.append(fader.widget());
        faders.push(FaderSlot {
            name: control.clone(),
            writable: is_writable(state, control),
            fader,
        });
    }

    if !block.advanced.is_empty() {
        let list = gtk::Box::new(Orientation::Vertical, 8);
        for control in &block.advanced {
            let editor = Editor::new(state, control, &advanced_label(control), on_event);
            list.append(&editor.root);
            editors.push(editor);
        }
        let expander = gtk::Expander::new(Some(&t("Advanced")));
        expander.add_css_class("mic-advanced");
        expander.set_child(Some(&list));
        card.append(&expander);
    }

    card
}

/// Builds the four-band EQ: one gain fader and the editors per band.
fn build_eq(
    eq: &EqModel,
    state: &AppState,
    on_event: &Rc<dyn Fn(MicEvent)>,
    toggles: &mut Vec<Toggle>,
    faders: &mut Vec<FaderSlot>,
    editors: &mut Vec<Editor>,
) -> gtk::Box {
    let section = gtk::Box::new(Orientation::Vertical, 10);
    section.add_css_class("mic-eq");

    let header = gtk::Box::new(Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some(&t("EQ")));
    title.add_css_class("strip-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    if let Some(control) = &eq.toggle {
        let (toggle, row) = build_toggle(state, control, "ON", on_event);
        header.append(&row);
        toggles.push(toggle);
    }
    section.append(&header);

    if eq.bands.is_empty() {
        section.append(&unavailable_label());
        return section;
    }

    let bands = gtk::Box::new(Orientation::Horizontal, 12);
    bands.set_homogeneous(true);
    for (index, band) in eq.bands.iter().enumerate() {
        let number = band_number(&band.gain).unwrap_or(index as u32 + 1);
        let column = gtk::Box::new(Orientation::Vertical, 8);
        column.add_css_class("eq-band");
        column.set_hexpand(true);

        let name = gtk::Label::new(Some(&tf(
            "BAND {number}",
            &[("{number}", &number.to_string())],
        )));
        name.add_css_class("strip-subtitle");
        name.set_xalign(0.0);
        column.append(&name);

        let fader = build_fader(state, &band.gain, &t("Gain"), on_event);
        column.append(fader.widget());
        faders.push(FaderSlot {
            name: band.gain.clone(),
            writable: is_writable(state, &band.gain),
            fader,
        });

        let frequency = Editor::new(state, &band.frequency, &t("Frequency"), on_event);
        column.append(&frequency.root);
        editors.push(frequency);

        let q = Editor::new(state, &band.q, &t("Q"), on_event);
        column.append(&q.root);
        editors.push(q);

        if let Some(control) = &band.shape {
            let label = if number == 1 {
                t("Low shape")
            } else {
                t("High shape")
            };
            let (toggle, row) = build_toggle(state, control, &label, on_event);
            column.append(&row);
            toggles.push(toggle);
        }

        bands.append(&column);
    }
    section.append(&bands);
    section
}

/// Builds one labelled switch.
fn build_toggle(
    state: &AppState,
    name: &str,
    label: &str,
    on_event: &Rc<dyn Fn(MicEvent)>,
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
            emit(MicEvent::Write {
                name: control_name.clone(),
                value: i64::from(active),
            });
        }
        Propagation::Proceed
    });

    let tag = gtk::Label::new(Some(label));
    tag.add_css_class("tag-label");
    let row = gtk::Box::new(Orientation::Horizontal, 6);
    row.set_halign(gtk::Align::End);
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

/// Builds a vertical fader over the control's real range and unit.
fn build_fader(
    state: &AppState,
    name: &str,
    label: &str,
    on_event: &Rc<dyn Fn(MicEvent)>,
) -> Fader {
    let emit = on_event.clone();
    let fader = Fader::new(fader_spec(state, name, label), move |output| {
        emit(MicEvent::Write {
            name: output.name,
            value: output.value,
        });
    });
    // An unknown value is drawn dashed by the fader itself; a writable
    // fader stays usable so the user can recover from a reset by writing.
    fader.set_sensitive(is_writable(state, name));
    fader
}

/// Whether the driver accepts writes for `name`.
fn is_writable(state: &AppState, name: &str) -> bool {
    state
        .control(name)
        .is_some_and(|control| control.is_writable())
}

/// Builds one meter, with the dashed unknown state when it cannot be read.
fn build_meter(state: &AppState, id: MeterId, label: &str) -> Metered {
    let meter = Meter::new(MeterSpec {
        label: label.to_string(),
        card: meter_level(state, id),
        card_present: true,
        // v1 has no PipeWire source: the Linux bar is absent, not unknown.
        linux: None,
        peak: 0.0,
        linux_present: false,
    });
    // The chain is taller than the meters: keep the bar and its label at
    // the top so both stay visible while the panel scrolls. The width
    // request keeps the full `MIC IN` / `MIC OUT` label under the bar.
    let widget = meter.widget();
    widget.set_valign(gtk::Align::Start);
    widget.set_width_request(64);
    Metered { id, meter }
}

/// A muted fallback for a block with no usable control.
fn unavailable_label() -> gtk::Label {
    let label = gtk::Label::new(Some(&t("Unavailable")));
    label.add_css_class("fader-value");
    label.add_css_class("unknown");
    label
}

/// The fader spec of one mic control.
fn fader_spec(state: &AppState, name: &str, label: &str) -> FaderSpec {
    let control = state.control(name);
    let (min, max, step, unit, value) = match control {
        Some(control) => (
            control.info.min,
            control.info.max,
            control.info.step,
            control.unit,
            control.value,
        ),
        None => (0, 100, 1, Unit::Steps, ControlValue::Unknown),
    };
    FaderSpec {
        name: name.to_string(),
        label: label.to_string(),
        unit,
        min,
        max,
        step,
        value,
    }
}

/// The user label of an advanced sub-parameter.
fn advanced_label(name: &str) -> String {
    let suffix = match name.split_once(" Out ") {
        Some(_) => "Output gain",
        None => name.rsplit(' ').next().unwrap_or(name),
    };
    match suffix {
        "Output gain" => t("Output gain"),
        "Threshold" => t("Threshold"),
        "Range" => t("Range"),
        "Attack" => t("Attack"),
        "Hold" => t("Hold"),
        "Decay" => t("Decay"),
        "Ratio" => t("Ratio"),
        "Release" => t("Release"),
        "Knee" => t("Knee"),
        other => other.to_string(),
    }
}

/// The band number of an EQ gain control, e.g. `EQ Band 2 Gain` is 2.
fn band_number(gain: &str) -> Option<u32> {
    gain.split_whitespace().nth(2).and_then(|n| n.parse().ok())
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
    use super::{advanced_label, band_number, level_from_steps};

    #[test]
    fn meter_steps_are_normalised_to_the_unit_interval() {
        assert_eq!(level_from_steps(None), None);
        assert_eq!(level_from_steps(Some(0)), Some(0.0));
        assert_eq!(level_from_steps(Some(127)), Some(1.0));
        assert_eq!(level_from_steps(Some(-1)), Some(0.0));
    }

    #[test]
    fn the_advanced_labels_strip_the_block_prefix() {
        assert_eq!(advanced_label("Gate Threshold"), "Threshold");
        assert_eq!(advanced_label("Compressor Attack"), "Attack");
        assert_eq!(advanced_label("Limiter Release"), "Release");
        assert_eq!(advanced_label("Compressor Out Gain"), "Output gain");
        assert_eq!(advanced_label("Limiter Out Gain"), "Output gain");
        assert_eq!(advanced_label("Gate Decay"), "Decay");
    }

    #[test]
    fn the_band_number_comes_from_the_control_name() {
        assert_eq!(band_number("EQ Band 1 Gain"), Some(1));
        assert_eq!(band_number("EQ Band 4 Gain"), Some(4));
        assert_eq!(band_number("Gate"), None);
    }
}
