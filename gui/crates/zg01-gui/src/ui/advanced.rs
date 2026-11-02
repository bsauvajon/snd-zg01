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
use zg01_core::geometry::snap_to_step;
use zg01_core::model::{format_value, AppState, ControlValue, Unit};

use super::advanced_model::{AdvancedModel, AdvancedRow};
use crate::i18n::{t, tnf};

/// What the editor shows while the device value is unknown.
const UNKNOWN_TEXT: &str = "—";

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

/// The value an editor emits for the raw value a user picked.
///
/// A scale drag or a typed number can land between two device steps; the
/// driver only accepts the `min + n * step` grid, so every emitted value is
/// snapped there, exactly like the shared [`crate::widgets`] fader.
fn snapped_edit(raw: i64, min: i64, max: i64, step: i64) -> i64 {
    snap_to_step(raw, min, max, step)
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
    /// The last value the editor showed or emitted, for de-duplication.
    last: Rc<Cell<Option<i64>>>,
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
        let last = Rc::new(Cell::new(value.known()));
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
                let handler_last = last.clone();
                let emit = on_event.clone();
                let control_name = row.name.clone();
                slider.connect_value_changed(move |slider| {
                    if handler_guard.get() {
                        return;
                    }
                    let raw = slider.value().round() as i64;
                    let value = snapped_edit(raw, min, max, step);
                    if value != raw {
                        // Put the handle back on the grid without treating
                        // the correction as a second user change.
                        handler_guard.set(true);
                        slider.set_value(value as f64);
                        handler_guard.set(false);
                    }
                    if handler_last.get() == Some(value) {
                        return;
                    }
                    handler_last.set(Some(value));
                    emit(AdvancedEvent::Write {
                        name: control_name.clone(),
                        value,
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
                entry.set_tooltip_text(Some(&t(
                    "Enter the register value: the range is too wide for a slider.",
                )));
                entry.set_value(start as f64);
                let handler_guard = guard.clone();
                let handler_last = last.clone();
                let emit = on_event.clone();
                let control_name = row.name.clone();
                entry.connect_value_changed(move |entry| {
                    if handler_guard.get() {
                        return;
                    }
                    let raw = entry.value().round() as i64;
                    let value = snapped_edit(raw, min, max, step);
                    if value != raw {
                        handler_guard.set(true);
                        entry.set_value(value as f64);
                        handler_guard.set(false);
                    }
                    if handler_last.get() == Some(value) {
                        return;
                    }
                    handler_last.set(Some(value));
                    emit(AdvancedEvent::Write {
                        name: control_name.clone(),
                        value,
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
            last,
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
                self.last.set(Some(value));
            }
            ControlValue::Unknown => {
                if let Some(switch) = &self.switch {
                    switch.add_css_class("unknown-value");
                }
                self.value_label.set_text(UNKNOWN_TEXT);
                self.value_label.add_css_class("unknown");
                self.last.set(None);
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

        self.count.set_text(&tnf(
            "{count} control",
            "{count} controls",
            shown,
            &[("{count}", &shown.to_string())],
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

        let title = gtk::Label::new(Some(&t("Advanced")));
        title.add_css_class("page-title");
        title.set_xalign(0.0);
        root.append(&title);

        let subtitle = gtk::Label::new(Some(&t(
            "Every ALSA control that Mixage, Micro and the curated table do not cover.",
        )));
        subtitle.add_css_class("page-subtitle");
        subtitle.set_xalign(0.0);
        root.append(&subtitle);

        let count = gtk::Label::new(None);
        count.add_css_class("advanced-count");
        count.set_xalign(1.0);

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(&t("Search for a control")));
        search.set_hexpand(true);
        search.set_tooltip_text(Some(&t("Filter controls by name")));

        let groups = groups_of(&model);
        let mut labels = vec![t("All groups")];
        labels.extend(groups.iter().cloned());
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let group_filter = gtk::DropDown::from_strings(&label_refs);
        group_filter.set_tooltip_text(Some(&t("Filter by console section")));

        let toolbar = gtk::Box::new(Orientation::Horizontal, 10);
        toolbar.add_css_class("advanced-toolbar");
        toolbar.append(&search);
        toolbar.append(&group_filter);
        toolbar.append(&count);
        root.append(&toolbar);

        let empty = gtk::Label::new(Some(&t("No control matches this filter.")));
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

        connect_filters(&search, &group_filter, filter.clone());
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

/// Wires the search and section filters to the table.
///
/// Each handler holds the other widget weakly. A strong capture on both
/// sides would form a reference cycle between the search entry and the
/// drop-down and keep the whole row tree alive after a reload replaced the
/// screen. The [`FilterBar`] is held strongly by the two handlers and is
/// released with them.
fn connect_filters(search: &gtk::SearchEntry, group_filter: &gtk::DropDown, filter: Rc<FilterBar>) {
    let group_weak = group_filter.downgrade();
    {
        let filter = filter.clone();
        search.connect_search_changed(move |search| {
            let Some(group_filter) = group_weak.upgrade() else {
                return;
            };
            filter.apply(search.text().as_str(), group_filter.selected());
        });
    }

    let search_weak = search.downgrade();
    group_filter.connect_selected_notify(move |group_filter| {
        let Some(search) = search_weak.upgrade() else {
            return;
        };
        filter.apply(search.text().as_str(), group_filter.selected());
    });
}

/// The column header of the table.
fn build_header() -> gtk::Box {
    let header = gtk::Box::new(Orientation::Horizontal, 10);
    header.add_css_class("advanced-header");

    let spacer = gtk::Box::new(Orientation::Horizontal, 0);
    spacer.set_width_request(BADGE_WIDTH);
    header.append(&spacer);

    for (key, label, width) in [
        ("Section", t("Section"), GROUP_WIDTH),
        ("Control", t("Control"), NAME_WIDTH),
        ("Editor", t("Editor"), EDITOR_WIDTH),
        ("Value", t("Value"), VALUE_WIDTH),
        ("Unit", t("Unit"), UNIT_WIDTH),
        ("Type", t("Type"), TYPE_WIDTH),
    ] {
        let column = gtk::Label::new(Some(&label));
        column.add_css_class("advanced-column");
        column.set_xalign(0.0);
        column.set_width_request(width);
        if key == "Control" {
            column.set_hexpand(true);
        }
        if key == "Value" {
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
        icon.set_tooltip_text(Some(&t(
            "Raw parameter: the meaning of this control is not decoded. Check the value before writing it.",
        )));
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

    let unit = gtk::Label::new(Some(&unit_label(row.unit)));
    unit.add_css_class("advanced-unit");
    unit.set_xalign(0.0);
    unit.set_width_request(UNIT_WIDTH);
    line.append(&unit);

    let kind = gtk::Label::new(Some(&type_label(row.unit)));
    kind.add_css_class("advanced-type");
    kind.set_xalign(0.0);
    kind.set_width_request(TYPE_WIDTH);
    line.append(&kind);

    list_row.set_child(Some(&line));

    if !row.writable {
        list_row.set_tooltip_text(Some(&t(
            "Read-only: the driver refuses writes to this control.",
        )));
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
fn unit_label(unit: Unit) -> String {
    match unit {
        Unit::Db => t("dB"),
        Unit::Hz => t("Hz"),
        Unit::Q => t("Q"),
        Unit::Steps => t("steps"),
        Unit::Percent => t("%"),
        Unit::Bool => t("on/off"),
        Unit::Raw => t("raw"),
    }
}

/// The type column of a row.
fn type_label(unit: Unit) -> String {
    match unit {
        Unit::Db => t("gain"),
        Unit::Hz => t("frequency"),
        Unit::Q => t("Q"),
        Unit::Percent => t("percentage"),
        Unit::Steps | Unit::Raw => t("integer"),
        Unit::Bool => t("switch"),
    }
}

#[cfg(test)]
mod tests {
    use super::{connect_filters, editor_kind, snapped_edit, type_label, unit_label, EditorKind};
    use crate::ui::advanced_model::AdvancedModel;
    use gtk4 as gtk;
    use gtk4::prelude::*;
    use std::rc::Rc;
    use zg01_core::model::Unit;

    #[test]
    fn the_unit_and_type_columns_name_every_unit() {
        assert_eq!(unit_label(Unit::Db), "dB");
        assert_eq!(unit_label(Unit::Steps), "steps");
        assert_eq!(type_label(Unit::Bool), "switch");
        assert_eq!(type_label(Unit::Hz), "frequency");
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

    #[test]
    fn an_off_grid_scale_or_spin_value_snaps_to_the_control_grid() {
        assert_eq!(snapped_edit(63, 0, 100, 5), 65);
        assert_eq!(snapped_edit(62, 0, 100, 5), 60);
        assert_eq!(
            snapped_edit(-173, -180, 180, 10),
            -170,
            "the grid is anchored on the control minimum"
        );
        assert_eq!(
            snapped_edit(7, 0, 2_097_151, 1),
            7,
            "a one-step register is already on its grid"
        );
        assert_eq!(
            snapped_edit(42, 0, 100, 0),
            42,
            "a control with no published step moves one value at a time"
        );
    }

    /// The search and section handlers must not form a reference cycle.
    ///
    /// Each handler used to capture the other widget strongly, which kept
    /// both, the `FilterBar` and every `ListBoxRow` alive after a reload
    /// replaced the screen. The weak captures release them.
    #[test]
    #[ignore = "needs a GTK display; run under Xvfb with --ignored"]
    fn the_filter_handlers_do_not_keep_the_widgets_alive() {
        gtk::init().expect("a GTK display, e.g. Xvfb");

        let search = gtk::SearchEntry::new();
        let group_filter = gtk::DropDown::from_strings(&["All groups", "MIC"]);
        let filter = Rc::new(super::FilterBar {
            model: AdvancedModel { rows: Vec::new() },
            entries: Vec::new(),
            groups: vec!["MIC".to_string()],
            count: gtk::Label::new(None),
            empty: gtk::Label::new(None),
        });
        connect_filters(&search, &group_filter, filter);

        let search_weak = search.downgrade();
        let group_weak = group_filter.downgrade();
        drop(search);
        drop(group_filter);

        assert!(
            search_weak.upgrade().is_none(),
            "the drop-down handler must not keep the search entry alive"
        );
        assert!(
            group_weak.upgrade().is_none(),
            "the search handler must not keep the drop-down alive"
        );
    }
}
