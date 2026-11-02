//! The preset loader: chooser, preview and apply report.
//!
//! `Charger` opens a `.zgpreset`-filtered chooser. A successful parse opens a
//! modal preview: block, name, how many parameters apply, every skip and its
//! reason, and the `Inclure les paramètres bruts` opt-in. The dialog only
//! returns a [`PresetPlan`]; the application applies it through the writer,
//! and Save stays the separate header action.

use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use zg01_core::model::Control;
use zg01_core::preset::{
    control_map_entry, load_preset, plan_preset, ApplyReport, Preset, PresetPlan, Skip, SkipReason,
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
            match load_preset(&path) {
                Ok(preset) => {
                    show_preview(&parent, preset, Rc::clone(&controls), Rc::clone(&on_apply))
                }
                Err(error) => show_error(&parent, &error.to_string()),
            }
        },
    );
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
        .default_width(600)
        .default_height(460)
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

    let skips = gtk::ListBox::new();
    skips.set_selection_mode(gtk::SelectionMode::None);
    skips.add_css_class("preset-skips");
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_child(Some(&skips));
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
        let skips = skips.clone();
        let apply = apply.clone();
        Rc::new(move || {
            let plan = plan_preset(&preset, &controls, allow_raw.is_active());
            summary.set_text(&summary_text(&plan));
            while let Some(child) = skips.first_child() {
                skips.remove(&child);
            }
            for skip in &plan.skipped {
                skips.append(&skip_row(skip));
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

/// One row of the skip list.
fn skip_row(skip: &Skip) -> gtk::ListBoxRow {
    let label = gtk::Label::new(Some(&skip_text(skip)));
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

/// The one-line summary of a plan.
fn summary_text(plan: &PresetPlan) -> String {
    let applied = match plan.applies.len() {
        0 => t("No parameter applied"),
        1 => t("1 parameter applied"),
        count => tf(
            "{count} parameters applied",
            &[("{count}", &count.to_string())],
        ),
    };
    let skipped = match plan.skipped.len() {
        0 => t("none skipped"),
        1 => t("1 skipped"),
        count => tf("{count} skipped", &[("{count}", &count.to_string())]),
    };
    tf(
        "{applied}, {skipped}.",
        &[("{applied}", &applied), ("{skipped}", &skipped)],
    )
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

/// Shows the report of an applied plan.
///
/// The report names what landed, what the plan skipped and every driver
/// rejection. It also reminds the user that nothing persists until Save.
pub(crate) fn present_preset_report(
    parent: &adw::ApplicationWindow,
    label: &str,
    report: &ApplyReport,
) {
    let title = if report.failed.is_empty() {
        t("Preset applied")
    } else {
        t("Preset partly applied")
    };

    let mut body = format!("{label}\n\n");
    body.push_str(&tf(
        "{applied} applied, {skipped} skipped, {failed} failed.\n\n",
        &[
            ("{applied}", &report.applied.len().to_string()),
            ("{skipped}", &report.skipped.len().to_string()),
            ("{failed}", &report.failed.len().to_string()),
        ],
    ));
    if !report.applied.is_empty() {
        body.push_str(&t("Applied:\n"));
        for name in report.applied.iter().take(12) {
            body.push_str(&format!("• {name}\n"));
        }
        if report.applied.len() > 12 {
            body.push_str("• …\n");
        }
        body.push('\n');
    }
    if !report.skipped.is_empty() {
        body.push_str(&t("Skipped:\n"));
        for skip in report.skipped.iter().take(12) {
            body.push_str(&format!("• {}\n", skip_text(skip)));
        }
        if report.skipped.len() > 12 {
            body.push_str("• …\n");
        }
        body.push('\n');
    }
    if !report.failed.is_empty() {
        body.push_str(&t("Failed:\n"));
        for (name, error) in report.failed.iter().take(12) {
            body.push_str(&format!("• {name} : {error}\n"));
        }
        if report.failed.len() > 12 {
            body.push_str("• …\n");
        }
        body.push('\n');
    }
    body.push_str(&t(
        "Nothing is saved on the ZG01: use “Save” to keep these settings.",
    ));

    let dialog = adw::AlertDialog::new(Some(&title), Some(&body));
    dialog.add_response("close", &t("Close"));
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");
    dialog.present(Some(parent));
}
