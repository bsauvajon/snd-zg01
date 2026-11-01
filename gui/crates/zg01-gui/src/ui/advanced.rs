//! The Avancé screen: the escape-hatch table of the remaining controls.
//!
//! The screen is pure presentation: it is built from an [`AdvancedModel`] and
//! the values of the [`AppState`], reports an editor change through
//! [`AdvancedEvent::Write`] and never touches ALSA itself. The application
//! owns the write worker and pushes the results back with
//! [`AdvancedScreen::set_control`].
//!
//! Every row shows the control name, its console section, a type-aware
//! editor, the formatted value, the unit and the type. A raw row — the
//! generated control map's `provenance` is `raw` — carries a warning icon
//! and a tooltip: the value can be written, but its meaning is not decoded.
//! A read-only row keeps its value visible and its editor disabled. A value
//! the GUI no longer knows is drawn as `—`, never as a zero.
//!
//! The search field filters on the name, case-insensitively; the section
//! drop-down narrows the same rows by group. Both filters hide rows, they
//! never rewrite the model.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{self as gtk, glib::Propagation, Orientation};
use zg01_core::model::{format_value, AppState, ControlValue, Unit};

use super::advanced_model::{AdvancedModel, AdvancedRow};

/// What the editor shows while the device value is unknown.
const UNKNOWN_TEXT: &str = "—";

/// The tooltip of a raw row's warning icon.
const RAW_TOOLTIP: &str =
    "Paramètre brut : la sémantique de ce contrôle n'est pas décodée. Vérifiez la valeur avant de l'écrire.";

/// The width of the warning-icon column.
const BADGE_WIDTH: i32 = 18;
/// The width of the section column.
const GROUP_WIDTH: i32 = 92;
/// The minimum width of the name column.
const NAME_WIDTH: i32 = 220;
/// The width of the editor column.
const EDITOR_WIDTH: i32 = 220;
/// The width of the value column.
const VALUE_WIDTH: i32 = 104;
/// The width of the unit column.
const UNIT_WIDTH: i32 = 54;
/// The width of the type column.
const TYPE_WIDTH: i32 = 96;

/// The widest range a scale can still step through usefully.
///
/// The raw controls are 21-bit fields (`0..=2097151`): a scale would move
/// by thousands per pixel, so they get a number entry instead.
const SCALE_MAX_RANGE: i64 = 10_000;

/// The editor a row gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorKind {
    /// A switch for a boolean control.
    Switch,
    /// A scale for a range that fits on screen.
    Scale,
    /// A number entry for a wide range.
    Number,
}

/// The editor of a control, from its unit and range.
fn editor_kind(unit: Unit, min: i64, max: i64) -> EditorKind {
    if unit == Unit::Bool {
        EditorKind::Switch
    } else if max.saturating_sub(min) > SCALE_MAX_RANGE {
        EditorKind::Number
    } else {
        EditorKind::Scale
    }
}

/// What the screen asks the application to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdvancedEvent {
    /// An editor asks for a new value.
    Write {
        /// The ALSA name of the control.
        name: String,
        /// The value to write.
        value: i64,
    },
}

/// A switch, a scale or a number entry over one control, and the readout
/// beside it.
struct ControlEditor {
    /// The ALSA name of the control.
    name: String,
    /// The unit the readout uses.
    unit: Unit,
    /// The switch of a boolean control.
    switch: Option<gtk::Switch>,
    /// The scale of a numeric control.
    scale: Option<gtk::Scale>,
    /// The number entry of a wide-range control.
    spin: Option<gtk::SpinButton>,
    /// The formatted value.
    value_label: gtk::Label,
    /// Keeps a programmatic update from looking like a user change.
    guard: Rc<Cell<bool>>,
    /// Whether the driver accepts writes.
    writable: bool,
}

impl ControlEditor {
    /// Builds the editor of `row` and the widget to put in the table.
    fn new(
        row: &AdvancedRow,
        state: &AppState,
        on_event: &Rc<dyn Fn(AdvancedEvent)>,
    ) -> (Self, gtk::Widget) {
        let control = state.control(&row.name);
        let (min, max, step, unit, value) = match control {
            Some(control) => (
                control.info.min,
                control.info.max,
                control.info.step,
                control.unit,
                control.value,
            ),
            None => (0, 100, 1, row.unit, row.value),
        };

        let guard = Rc::new(Cell::new(false));
        let value_label = gtk::Label::new(None);
        value_label.add_css_class("fader-value");
        value_label.set_xalign(1.0);
        value_label.set_width_request(VALUE_WIDTH);

        let mut switch = None;
        let mut scale = None;
        let mut spin = None;
        let widget: gtk::Widget = match editor_kind(unit, min, max) {
            EditorKind::Switch => {
                let toggle = gtk::Switch::new();
                toggle.set_halign(gtk::Align::Start);
                toggle.set_valign(gtk::Align::Center);
                let handler_guard = guard.clone();
                let emit = on_event.clone();
                let control_name = row.name.clone();
                toggle.connect_state_set(move |_, active| {
                    if !handler_guard.get() {
                        emit(AdvancedEvent::Write {
                            name: control_name.clone(),
                            value: i64::from(active),
                        });
                    }
                    Propagation::Proceed
                });
                switch = Some(toggle.clone());
                let holder = gtk::Box::new(Orientation::Horizontal, 0);
                holder.set_width_request(EDITOR_WIDTH);
                holder.append(&toggle);
                holder.upcast()
            }
            EditorKind::Scale => {
                let start = value.known().unwrap_or(min).clamp(min, max);
                let adjustment = gtk::Adjustment::new(
                    start as f64,
                    min as f64,
                    max as f64,
                    step.max(1) as f64,
                    0.0,
                    0.0,
                );
                let slider = gtk::Scale::new(Orientation::Horizontal, Some(&adjustment));
                slider.set_draw_value(false);
                slider.set_digits(0);
                slider.set_width_request(EDITOR_WIDTH);
                slider.add_css_class("advanced-scale");
                let handler_guard = guard.clone();
                let emit = on_event.clone();
                let control_name = row.name.clone();
                slider.connect_value_changed(move |slider| {
                    if handler_guard.get() {
                        return;
                    }
                    emit(AdvancedEvent::Write {
                        name: control_name.clone(),
                        value: slider.value().round() as i64,
                    });
                });
                scale = Some(slider.clone());
                slider.upcast()
            }
            EditorKind::Number => {
                let start = value.known().unwrap_or(min).clamp(min, max);
                let entry = gtk::SpinButton::with_range(min as f64, max as f64, step.max(1) as f64);
                entry.set_digits(0);
                entry.set_width_request(EDITOR_WIDTH);
                entry.add_css_class("advanced-number");
                entry.set_tooltip_text(Some(
                    "Entrez la valeur du registre : la plage est trop large pour un curseur.",
                ));
                entry.set_value(start as f64);
                let handler_guard = guard.clone();
                let emit = on_event.clone();
                let control_name = row.name.clone();
                entry.connect_value_changed(move |entry| {
                    if handler_guard.get() {
                        return;
                    }
                    emit(AdvancedEvent::Write {
                        name: control_name.clone(),
                        value: entry.value().round() as i64,
                    });
                });
                spin = Some(entry.clone());
                entry.upcast()
            }
        };

        let editor = ControlEditor {
            name: row.name.clone(),
            unit,
            switch,
            scale,
            spin,
            value_label,
            guard,
            writable: row.writable,
        };
        editor.set(value);
        (editor, widget)
    }

    /// Installs a value the device reported, or its failure.
    fn set(&self, value: ControlValue) {
        self.guard.set(true);
        match value {
            ControlValue::Known(value) => {
                if let Some(switch) = &self.switch {
                    switch.set_active(value != 0);
                    switch.remove_css_class("unknown-value");
                }
                if let Some(scale) = &self.scale {
                    scale.set_value(value as f64);
                }
                if let Some(spin) = &self.spin {
                    spin.set_value(value as f64);
                }
                self.value_label.set_text(&format_value(self.unit, value));
                self.value_label.remove_css_class("unknown");
            }
            ControlValue::Unknown => {
                if let Some(switch) = &self.switch {
                    switch.add_css_class("unknown-value");
                }
                self.value_label.set_text(UNKNOWN_TEXT);
                self.value_label.add_css_class("unknown");
            }
        }
        if let Some(switch) = &self.switch {
            switch.set_sensitive(self.writable);
        }
        if let Some(scale) = &self.scale {
            scale.set_sensitive(self.writable);
        }
        if let Some(spin) = &self.spin {
            spin.set_sensitive(self.writable);
        }
        self.guard.set(false);
    }
}

/// One row of the table and its editor.
struct RowWidgets {
    /// The editor that shows and writes the value.
    editor: ControlEditor,
}

/// One row of the visible table, for the search and section filters.
struct FilterEntry {
    /// The ALSA name of the control.
    name: String,
    /// The console section of the row.
    group: String,
    /// The row to hide or show.
    row: gtk::ListBoxRow,
}

/// The search and section filters.
///
/// The filters only hide rows: the model is never rewritten, so clearing a
/// filter brings every row back with the value it already held.
struct FilterBar {
    /// The model the search runs on.
    model: AdvancedModel,
    /// Every row of the table.
    entries: Vec<FilterEntry>,
    /// The sections of the drop-down, without the `Tous` entry.
    groups: Vec<String>,
    /// The number of visible controls.
    count: gtk::Label,
    /// The message shown when no row matches.
    empty: gtk::Label,
}

impl FilterBar {
    /// Shows the rows that match `query` and the selected section.
    fn apply(&self, query: &str, selected: u32) {
        let matched: HashSet<&str> = self
            .model
            .filter(query)
            .iter()
            .map(|row| row.name.as_str())
            .collect();
        let group = if selected == 0 {
            None
        } else {
            self.groups.get(selected as usize - 1).map(String::as_str)
        };

        let mut shown = 0;
        for entry in &self.entries {
            let visible = matched.contains(entry.name.as_str())
                && group.is_none_or(|group| entry.group == group);
            entry.row.set_visible(visible);
            if visible {
                shown += 1;
            }
        }

        self.count.set_text(&format!(
            "{shown} contrôle{}",
            if shown == 1 { "" } else { "s" }
        ));
        self.empty.set_visible(shown == 0);
    }
}

/// The Avancé screen.
pub(crate) struct AdvancedScreen {
    root: gtk::Box,
    rows: Vec<RowWidgets>,
}

impl AdvancedScreen {
    /// Builds the screen from the model and the current control values.
    pub(crate) fn new(
        model: AdvancedModel,
        state: &AppState,
        on_event: impl Fn(AdvancedEvent) + 'static,
    ) -> Self {
        let on_event: Rc<dyn Fn(AdvancedEvent)> = Rc::new(on_event);

        let root = gtk::Box::new(Orientation::Vertical, 14);
        root.set_margin_top(24);
        root.set_margin_bottom(24);
        root.set_margin_start(24);
        root.set_margin_end(24);

        let title = gtk::Label::new(Some("Avancé"));
        title.add_css_class("page-title");
        title.set_xalign(0.0);
        root.append(&title);

        let subtitle = gtk::Label::new(Some(
            "Tous les contrôles ALSA que Mixage, Micro et la table curatée ne couvrent pas.",
        ));
        subtitle.add_css_class("page-subtitle");
        subtitle.set_xalign(0.0);
        root.append(&subtitle);

        let count = gtk::Label::new(None);
        count.add_css_class("advanced-count");
        count.set_xalign(1.0);

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher un contrôle"));
        search.set_hexpand(true);
        search.set_tooltip_text(Some("Filtrer les contrôles par nom"));

        let groups = groups_of(&model);
        let mut labels = vec!["Tous les groupes".to_string()];
        labels.extend(groups.iter().cloned());
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let group_filter = gtk::DropDown::from_strings(&label_refs);
        group_filter.set_tooltip_text(Some("Filtrer par section de la console"));

        let toolbar = gtk::Box::new(Orientation::Horizontal, 10);
        toolbar.add_css_class("advanced-toolbar");
        toolbar.append(&search);
        toolbar.append(&group_filter);
        toolbar.append(&count);
        root.append(&toolbar);

        let empty = gtk::Label::new(Some("Aucun contrôle ne correspond à ce filtre."));
        empty.add_css_class("advanced-empty");
        empty.set_visible(false);

        let panel = gtk::Box::new(Orientation::Vertical, 0);
        panel.add_css_class("panel");
        panel.add_css_class("advanced-panel");
        panel.append(&build_header());

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("advanced-list");

        let mut rows = Vec::new();
        let mut entries = Vec::new();
        for row in &model.rows {
            let (editor, widget) = ControlEditor::new(row, state, &on_event);
            let list_row = build_row(row, widget, &editor.value_label);
            list.append(&list_row);
            entries.push(FilterEntry {
                name: row.name.clone(),
                group: row.group.clone(),
                row: list_row,
            });
            rows.push(RowWidgets { editor });
        }

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&list));
        panel.append(&scroll);
        panel.append(&empty);
        root.append(&panel);

        let filter = Rc::new(FilterBar {
            model,
            entries,
            groups,
            count,
            empty,
        });

        {
            let filter = filter.clone();
            let group_filter = group_filter.clone();
            search.connect_search_changed(move |search| {
                filter.apply(search.text().as_str(), group_filter.selected());
            });
        }
        {
            let filter = filter.clone();
            let search = search.clone();
            group_filter.connect_selected_notify(move |group_filter| {
                filter.apply(search.text().as_str(), group_filter.selected());
            });
        }
        filter.apply("", 0);

        AdvancedScreen { root, rows }
    }

    /// The widget to install in the content stack.
    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Installs a value the device reported, or its failure.
    pub(crate) fn set_control(&self, name: &str, value: ControlValue) {
        for row in &self.rows {
            if row.editor.name == name {
                row.editor.set(value);
            }
        }
    }
}

/// The column header of the table.
fn build_header() -> gtk::Box {
    let header = gtk::Box::new(Orientation::Horizontal, 10);
    header.add_css_class("advanced-header");

    let spacer = gtk::Box::new(Orientation::Horizontal, 0);
    spacer.set_width_request(BADGE_WIDTH);
    header.append(&spacer);

    for (label, width) in [
        ("Section", GROUP_WIDTH),
        ("Contrôle", NAME_WIDTH),
        ("Éditeur", EDITOR_WIDTH),
        ("Valeur", VALUE_WIDTH),
        ("Unité", UNIT_WIDTH),
        ("Type", TYPE_WIDTH),
    ] {
        let column = gtk::Label::new(Some(label));
        column.add_css_class("advanced-column");
        column.set_xalign(0.0);
        column.set_width_request(width);
        if label == "Contrôle" {
            column.set_hexpand(true);
        }
        if label == "Valeur" {
            column.set_xalign(1.0);
        }
        header.append(&column);
    }

    header
}

/// Builds one table row around its editor.
fn build_row(row: &AdvancedRow, editor: gtk::Widget, value_label: &gtk::Label) -> gtk::ListBoxRow {
    let list_row = gtk::ListBoxRow::new();
    list_row.set_activatable(false);
    list_row.set_selectable(false);
    list_row.add_css_class("advanced-row");
    if row.raw {
        list_row.add_css_class("raw-row");
    }

    let line = gtk::Box::new(Orientation::Horizontal, 10);
    line.add_css_class("advanced-line");

    let badge = gtk::Box::new(Orientation::Horizontal, 0);
    badge.set_width_request(BADGE_WIDTH);
    if row.raw {
        let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        icon.add_css_class("raw-badge");
        icon.set_tooltip_text(Some(RAW_TOOLTIP));
        badge.append(&icon);
    }
    line.append(&badge);

    let group = gtk::Label::new(Some(&row.group));
    group.add_css_class("advanced-group");
    group.set_xalign(0.0);
    group.set_width_request(GROUP_WIDTH);
    line.append(&group);

    let name = gtk::Label::new(Some(&row.name));
    name.add_css_class("advanced-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_width_request(NAME_WIDTH);
    name.set_ellipsize(pango::EllipsizeMode::End);
    name.set_tooltip_text(Some(&row.name));
    if !row.writable {
        name.add_css_class("read-only");
    }
    line.append(&name);

    line.append(&editor);
    line.append(value_label);

    let unit = gtk::Label::new(Some(unit_label(row.unit)));
    unit.add_css_class("advanced-unit");
    unit.set_xalign(0.0);
    unit.set_width_request(UNIT_WIDTH);
    line.append(&unit);

    let kind = gtk::Label::new(Some(type_label(row.unit)));
    kind.add_css_class("advanced-type");
    kind.set_xalign(0.0);
    kind.set_width_request(TYPE_WIDTH);
    line.append(&kind);

    list_row.set_child(Some(&line));

    if !row.writable {
        list_row.set_tooltip_text(Some(
            "Lecture seule : le pilote refuse les écritures de ce contrôle.",
        ));
    }

    list_row
}

/// The distinct sections of the table, sorted.
fn groups_of(model: &AdvancedModel) -> Vec<String> {
    let mut groups: Vec<String> = model.rows.iter().map(|row| row.group.clone()).collect();
    groups.sort();
    groups.dedup();
    groups
}

/// The unit column of a row.
fn unit_label(unit: Unit) -> &'static str {
    match unit {
        Unit::Db => "dB",
        Unit::Hz => "Hz",
        Unit::Q => "Q",
        Unit::Steps => "pas",
        Unit::Percent => "%",
        Unit::Bool => "on/off",
        Unit::Raw => "brut",
    }
}

/// The type column of a row.
fn type_label(unit: Unit) -> &'static str {
    match unit {
        Unit::Db => "gain",
        Unit::Hz => "fréquence",
        Unit::Q => "Q",
        Unit::Percent => "pourcentage",
        Unit::Steps | Unit::Raw => "entier",
        Unit::Bool => "interrupteur",
    }
}

#[cfg(test)]
mod tests {
    use super::{editor_kind, type_label, unit_label, EditorKind};
    use zg01_core::model::Unit;

    #[test]
    fn the_unit_and_type_columns_name_every_unit() {
        assert_eq!(unit_label(Unit::Db), "dB");
        assert_eq!(unit_label(Unit::Steps), "pas");
        assert_eq!(type_label(Unit::Bool), "interrupteur");
        assert_eq!(type_label(Unit::Hz), "fréquence");
        assert_eq!(type_label(Unit::Q), "Q");
    }

    #[test]
    fn a_boolean_gets_a_switch_and_a_wide_range_a_number_entry() {
        assert_eq!(editor_kind(Unit::Bool, 0, 1), EditorKind::Switch);
        assert_eq!(editor_kind(Unit::Db, -180, 180), EditorKind::Scale);
        assert_eq!(editor_kind(Unit::Steps, 0, 127), EditorKind::Scale);
        assert_eq!(editor_kind(Unit::Steps, 0, 2097151), EditorKind::Number);
        assert_eq!(editor_kind(Unit::Raw, 0, 2097151), EditorKind::Number);
    }
}
