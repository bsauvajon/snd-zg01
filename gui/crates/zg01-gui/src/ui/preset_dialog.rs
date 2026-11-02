//! The preset loader: chooser, preview and apply report.
//!
//! `Charger` opens a `.zgpreset`-filtered chooser. The file is parsed on a
//! worker; a successful parse opens a modal preview: block, name, every
//! applied and skipped parameter with its address, value and reason, and
//! the `Inclure les paramètres bruts` opt-in. The dialog only returns a
//! [`PresetPlan`]; the application applies it through the writer, and Save
//! stays the separate header action. The final report lists every applied,
//! skipped and failed row in a scrollable window: it is never truncated.

use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

use gtk4 as gtk;
use gtk4::glib::ControlFlow;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use zg01_core::model::{format_value, Control};
use zg01_core::preset::{
    control_map_entry, load_preset, plan_preset, Apply, ApplyReport, Preset, PresetError,
    PresetPlan, Skip, SkipReason,
};

use crate::i18n::{t, tf};

/// What the preview hands to the application when the user applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PresetChoice {
    /// The block of the preset.
    pub block: String,
    /// Its display name.
    pub name: String,
    /// The values to write, and the parameters left alone.
    pub plan: PresetPlan,
}

/// Opens the `.zgpreset` chooser, then the preview of the parsed file.
///
/// `controls` is the card's control surface, used to plan against what the
/// card actually publishes. `on_apply` runs once, when the user confirms the
/// preview, with the plan for the current raw opt-in.
pub(crate) fn present_preset_dialog(
    parent: &adw::ApplicationWindow,
    controls: Vec<Control>,
    on_apply: impl Fn(PresetChoice) + 'static,
) {
    let dialog = gtk::FileDialog::new();
    dialog.set_title(&t("Load a ZG01 preset"));
    dialog.set_accept_label(Some(&t("Open")));

    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&t("ZG01 presets (*.zgpreset)")));
    filter.add_pattern("*.zgpreset");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));

    let parent = parent.clone();
    let controls = Rc::new(controls);
    let on_apply: Rc<dyn Fn(PresetChoice)> = Rc::new(on_apply);
    let dialog_parent = parent.clone();
    dialog.open(
        Some(&dialog_parent),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let path = match result {
                Ok(file) => file.path(),
                Err(error) => {
                    if !error.matches(gtk::gio::IOErrorEnum::Cancelled) {
                        show_error(
                            &parent,
                            &tf(
                                "The file could not be chosen: {error}",
                                &[("{error}", &error.to_string())],
                            ),
                        );
                    }
                    return;
                }
            };
            let Some(path) = path else {
                return;
            };
            // A `.zg01` backup is not a preset: reject it by extension,
            // before the parser ever sees it.
            if is_backup_path(&path) {
                show_error(
                    &parent,
                    &preset_error_text(&PresetError::BackupNotSupported),
                );
                return;
            }
            parse_preset(&parent, path, Rc::clone(&controls), Rc::clone(&on_apply));
        },
    );
}

/// Whether the chosen path is a `.zg01` backup rather than a preset.
fn is_backup_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zg01"))
}

/// Parses a preset on a worker and opens its preview on the GTK thread.
///
/// The parser is fast on a normal file but it is still file I/O and C
/// allocation; the file-chooser callback must not run it, so the main loop
/// only polls the result channel.
fn parse_preset(
    parent: &adw::ApplicationWindow,
    path: std::path::PathBuf,
    controls: Rc<Vec<Control>>,
    on_apply: Rc<dyn Fn(PresetChoice)>,
) {
    let (results, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("zg01-preset-parse".to_string())
        .spawn(move || {
            let _ = results.send(load_preset(&path));
        });
    if let Err(error) = spawned {
        show_error(
            parent,
            &tf(
                "The preset could not be read: {error}",
                &[("{error}", &error.to_string())],
            ),
        );
        return;
    }

    let parent = parent.clone();
    gtk::glib::timeout_add_local(Duration::from_millis(25), move || {
        match receiver.try_recv() {
            Ok(Ok(preset)) => {
                show_preview(&parent, preset, Rc::clone(&controls), Rc::clone(&on_apply));
                ControlFlow::Break
            }
            Ok(Err(error)) => {
                show_error(&parent, &preset_error_text(&error));
                ControlFlow::Break
            }
            Err(TryRecvError::Empty) => ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                show_error(&parent, &t("The preset parser stopped unexpectedly"));
                ControlFlow::Break
            }
        }
    });
}

/// Builds and presents the preview of one parsed preset.
fn show_preview(
    parent: &adw::ApplicationWindow,
    preset: Preset,
    controls: Rc<Vec<Control>>,
    on_apply: Rc<dyn Fn(PresetChoice)>,
) {
    let window = gtk::Window::builder()
        .title(t("Load a ZG01 preset"))
        .modal(true)
        .transient_for(parent)
        .default_width(720)
        .default_height(560)
        .build();
    window.set_titlebar(Some(&gtk::HeaderBar::new()));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);

    let block = gtk::Label::new(Some(&tf("Block: {block}", &[("{block}", &preset.block)])));
    block.add_css_class("preset-block");
    block.set_xalign(0.0);
    root.append(&block);

    let name_text = if preset.name.is_empty() {
        t("Unnamed")
    } else {
        tf("Name: {name}", &[("{name}", &preset.name)])
    };
    let name = gtk::Label::new(Some(&name_text));
    name.add_css_class("preset-preview-name");
    name.set_xalign(0.0);
    root.append(&name);

    let summary = gtk::Label::new(None);
    summary.add_css_class("preset-summary");
    summary.set_xalign(0.0);
    root.append(&summary);

    let rows = gtk::ListBox::new();
    rows.set_selection_mode(gtk::SelectionMode::None);
    rows.add_css_class("preset-skips");
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_child(Some(&rows));
    scrolled.set_vexpand(true);
    scrolled.set_has_frame(true);
    root.append(&scrolled);

    let allow_raw = gtk::CheckButton::with_label(&t("Include raw parameters"));
    allow_raw.set_tooltip_text(Some(&t(
        "Raw parameters have no vendor model: their address and conversion are not verified by Yamaha.",
    )));
    root.append(&allow_raw);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label(&t("Cancel"));
    let apply = gtk::Button::with_label(&t("Apply"));
    apply.add_css_class("suggested-action");
    actions.append(&cancel);
    actions.append(&apply);
    root.append(&actions);

    let preset = Rc::new(preset);

    let refresh: Rc<dyn Fn()> = {
        let preset = Rc::clone(&preset);
        let controls = Rc::clone(&controls);
        let allow_raw = allow_raw.clone();
        let summary = summary.clone();
        let rows = rows.clone();
        let apply = apply.clone();
        Rc::new(move || {
            let plan = plan_preset(&preset, &controls, allow_raw.is_active());
            summary.set_text(&summary_text(&plan));
            while let Some(child) = rows.first_child() {
                rows.remove(&child);
            }
            for line in plan_lines(&plan, &controls) {
                rows.append(&text_row(&line));
            }
            apply.set_sensitive(!plan.applies.is_empty());
        })
    };
    allow_raw.connect_toggled({
        let refresh = Rc::clone(&refresh);
        move |_| refresh()
    });
    refresh();

    let preview = window.clone();
    let cancel_window = window.clone();
    cancel.connect_clicked(move |_| cancel_window.close());
    apply.connect_clicked(move |_| {
        let plan = plan_preset(&preset, &controls, allow_raw.is_active());
        on_apply(PresetChoice {
            block: preset.block.clone(),
            name: preset.name.clone(),
            plan,
        });
        preview.close();
    });

    window.set_child(Some(&root));
    window.present();
}

/// One row of a preview or report.
fn text_row(text: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("preset-skip");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    row.set_child(Some(&label));
    row
}

/// Every element of a plan, in application order then skip order.
///
/// The preview used to list only the skipped rows; the address, name and
/// value of every applied parameter are part of the before/after
/// accounting, and no row is dropped.
fn plan_lines(plan: &PresetPlan, controls: &[Control]) -> Vec<String> {
    let mut lines = Vec::with_capacity(plan.applies.len() + plan.skipped.len());
    for apply in &plan.applies {
        lines.push(apply_text(apply, controls));
    }
    for skip in &plan.skipped {
        lines.push(skip_text(skip));
    }
    lines
}

/// The description of an applied parameter.
fn apply_text(apply: &Apply, controls: &[Control]) -> String {
    let value = controls
        .iter()
        .find(|control| control.name() == apply.name)
        .map(|control| format_value(control.unit, apply.value))
        .unwrap_or_else(|| apply.value.to_string());
    tf(
        "{address} · {name} → {value}",
        &[
            ("{address}", &format!("0x{:03x}", apply.address)),
            ("{name}", &apply.name),
            ("{value}", &value),
        ],
    )
}

/// The one-line summary of a plan.
///
/// The counts travel in one translated message with placeholders, so the
/// two values are not assembled from separately translated fragments.
fn summary_text(plan: &PresetPlan) -> String {
    tf(
        "{applied} applied, {skipped} skipped.",
        &[
            ("{applied}", &plan.applies.len().to_string()),
            ("{skipped}", &plan.skipped.len().to_string()),
        ],
    )
}

/// The UI message of a preset read failure.
///
/// The error enum lives in `zg01-core`, whose messages are developer-facing
/// English. The dialog maps each variant to a gettext key so a French user
/// sees the reason in French; the wrapped I/O detail stays as the
/// operating-system text.
fn preset_error_text(error: &PresetError) -> String {
    match error {
        PresetError::BackupNotSupported => {
            t(".zg01 backups are not supported; load a .zgpreset file instead")
        }
        PresetError::Malformed => t("The file is not a valid .zgpreset file"),
        PresetError::Io(source) => tf(
            "The preset could not be read: {error}",
            &[("{error}", &source.to_string())],
        ),
        PresetError::InvalidPath => t("The preset path contains a NUL byte"),
        PresetError::Parser(code) => tf(
            "The preset parser failed with code {code}",
            &[("{code}", &code.to_string())],
        ),
    }
}

/// The description of a skipped parameter.
fn skip_text(skip: &Skip) -> String {
    let name = control_map_entry(skip.address)
        .map(|entry| entry.control_name.clone())
        .unwrap_or_else(|| t("unknown address"));
    tf(
        "{address} · {name} — {reason}",
        &[
            ("{address}", &format!("0x{:03x}", skip.address)),
            ("{name}", &name),
            ("{reason}", &skip_reason_text(skip.reason)),
        ],
    )
}

/// The reason of a skip, in the words of the UI.
fn skip_reason_text(reason: SkipReason) -> String {
    match reason {
        SkipReason::Missing => t("control missing or unknown address"),
        SkipReason::ReadOnly => t("read-only"),
        SkipReason::Raw => t("raw parameter (check to include)"),
        SkipReason::OutOfRange => t("value out of range"),
    }
}

/// Shows why a preset could not be read.
fn show_error(parent: &adw::ApplicationWindow, message: &str) {
    let dialog = adw::AlertDialog::new(Some(&t("Unreadable preset")), Some(message));
    dialog.add_response("close", &t("Close"));
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");
    dialog.present(Some(parent));
}

/// The category of one report row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportKind {
    Applied,
    Skipped,
    Failed,
}

/// One row of the final report.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportEntry {
    kind: ReportKind,
    text: String,
}

/// Every row of an apply report, in full.
fn report_entries(report: &ApplyReport) -> Vec<ReportEntry> {
    let mut entries =
        Vec::with_capacity(report.applied.len() + report.skipped.len() + report.failed.len());
    for applied in &report.applied {
        entries.push(ReportEntry {
            kind: ReportKind::Applied,
            text: tf(
                "{address} · {name} → {value}",
                &[
                    ("{address}", &format!("0x{:03x}", applied.address)),
                    ("{name}", &applied.name),
                    ("{value}", &applied.value.to_string()),
                ],
            ),
        });
    }
    for skip in &report.skipped {
        entries.push(ReportEntry {
            kind: ReportKind::Skipped,
            text: skip_text(skip),
        });
    }
    for failed in &report.failed {
        entries.push(ReportEntry {
            kind: ReportKind::Failed,
            text: tf(
                "{address} · {name} — {error}",
                &[
                    ("{address}", &format!("0x{:03x}", failed.address)),
                    ("{name}", &failed.name),
                    ("{error}", &failed.error),
                ],
            ),
        });
    }
    entries
}

/// Shows the report of an applied plan.
///
/// The report is a scrollable window, not a fixed alert body: every
/// applied, skipped and failed row is listed, however many a preset has.
/// It also reminds the user that nothing persists until Save.
pub(crate) fn present_preset_report(
    parent: &adw::ApplicationWindow,
    label: &str,
    report: &ApplyReport,
) {
    let title = if report.cancelled {
        t("Preset cancelled")
    } else if report.failed.is_empty() {
        t("Preset applied")
    } else {
        t("Preset partly applied")
    };

    let window = gtk::Window::builder()
        .title(title.clone())
        .modal(true)
        .transient_for(parent)
        .default_width(760)
        .default_height(560)
        .build();
    window.set_titlebar(Some(&gtk::HeaderBar::new()));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);

    let heading = gtk::Label::new(Some(&title));
    heading.add_css_class("preset-preview-name");
    heading.set_xalign(0.0);
    root.append(&heading);

    let summary = gtk::Label::new(Some(&tf(
        "{label}\n{applied} applied, {skipped} skipped, {failed} failed.",
        &[
            ("{label}", label),
            ("{applied}", &report.applied.len().to_string()),
            ("{skipped}", &report.skipped.len().to_string()),
            ("{failed}", &report.failed.len().to_string()),
        ],
    )));
    summary.add_css_class("preset-summary");
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    root.append(&summary);

    let rows = gtk::ListBox::new();
    rows.set_selection_mode(gtk::SelectionMode::None);
    rows.add_css_class("preset-skips");
    let mut current = None;
    for entry in report_entries(report) {
        if current != Some(entry.kind) {
            current = Some(entry.kind);
            let section = match entry.kind {
                ReportKind::Applied => t("Applied:"),
                ReportKind::Skipped => t("Skipped:"),
                ReportKind::Failed => t("Failed:"),
            };
            let label = gtk::Label::new(Some(&section));
            label.add_css_class("preset-block");
            label.set_xalign(0.0);
            rows.append(&text_row_widget(&label));
        }
        rows.append(&text_row(&entry.text));
    }
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_child(Some(&rows));
    scrolled.set_vexpand(true);
    scrolled.set_has_frame(true);
    root.append(&scrolled);

    let note = gtk::Label::new(Some(&t(
        "Nothing is saved on the ZG01: use “Save” to keep these settings.",
    )));
    note.add_css_class("preset-summary");
    note.set_xalign(0.0);
    note.set_wrap(true);
    root.append(&note);

    let close = gtk::Button::with_label(&t("Close"));
    close.add_css_class("suggested-action");
    close.set_halign(gtk::Align::End);
    let close_window = window.clone();
    close.connect_clicked(move |_| close_window.close());
    root.append(&close);

    window.set_child(Some(&root));
    window.present();
}

/// A row wrapping an arbitrary widget, used for the section headings.
fn text_row_widget(widget: &impl IsA<gtk::Widget>) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    row.set_child(Some(widget));
    row
}

#[cfg(test)]
mod tests {
    use super::{is_backup_path, plan_lines, preset_error_text, report_entries, ReportKind};
    use std::path::Path;
    use zg01_core::alsa::{ControlInfo, ControlKind};
    use zg01_core::model::{Control, ControlValue, Unit};
    use zg01_core::preset::{
        Applied, Apply, ApplyReport, Failed, PresetError, PresetPlan, Skip, SkipReason,
    };

    fn control(name: &str) -> Control {
        Control {
            info: ControlInfo {
                name: name.to_string(),
                kind: ControlKind::Integer,
                min: 0,
                max: 100,
                step: 0,
                writable: true,
            },
            value: ControlValue::Known(0),
            unit: Unit::Steps,
        }
    }

    #[test]
    fn preset_errors_have_translatable_ui_messages() {
        assert_eq!(
            preset_error_text(&PresetError::BackupNotSupported),
            ".zg01 backups are not supported; load a .zgpreset file instead"
        );
        assert_eq!(
            preset_error_text(&PresetError::Malformed),
            "The file is not a valid .zgpreset file"
        );
        assert_eq!(
            preset_error_text(&PresetError::InvalidPath),
            "The preset path contains a NUL byte"
        );
    }

    #[test]
    fn a_zg01_backup_is_rejected_by_extension() {
        assert!(is_backup_path(Path::new("backup.zg01")));
        assert!(is_backup_path(Path::new("BACKUP.ZG01")));
        assert!(!is_backup_path(Path::new("studio.zgpreset")));
        assert!(!is_backup_path(Path::new("zg01")));
    }

    #[test]
    fn the_preview_lists_every_applied_and_skipped_row() {
        let plan = PresetPlan {
            applies: (0..15)
                .map(|index| Apply {
                    address: index,
                    name: format!("Control {index}"),
                    value: index as i64,
                })
                .collect(),
            skipped: (0..15)
                .map(|index| Skip {
                    address: 100 + index,
                    reason: SkipReason::Missing,
                })
                .collect(),
        };
        let controls: Vec<Control> = (0..15)
            .map(|index| control(&format!("Control {index}")))
            .collect();

        let lines = plan_lines(&plan, &controls);
        assert_eq!(lines.len(), 30, "no applied or skipped row is truncated");
        assert!(lines.iter().any(|line| line.contains("Control 14")));
        assert!(
            lines.iter().any(|line| line.contains("0x064")),
            "0x100/… addresses are listed"
        );
    }

    #[test]
    fn the_report_lists_every_row() {
        let report = ApplyReport {
            applied: (0..15)
                .map(|index| Applied {
                    address: index,
                    name: format!("Applied {index}"),
                    value: index as i64,
                })
                .collect(),
            skipped: (0..13)
                .map(|index| Skip {
                    address: 100 + index,
                    reason: SkipReason::OutOfRange,
                })
                .collect(),
            failed: (0..11)
                .map(|index| Failed {
                    address: 200 + index,
                    name: format!("Failed {index}"),
                    error: "driver refused".to_string(),
                })
                .collect(),
            cancelled: false,
        };

        let entries = report_entries(&report);
        assert_eq!(entries.len(), 39);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.kind == ReportKind::Applied)
                .count(),
            15
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.kind == ReportKind::Skipped)
                .count(),
            13
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.kind == ReportKind::Failed)
                .count(),
            11
        );
        assert!(entries.iter().any(|entry| entry.text.contains("Failed 10")));
    }
}
