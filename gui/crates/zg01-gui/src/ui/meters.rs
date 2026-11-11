//! The card meter page: the two microphone levels and the four
//! eight-channel banks, laid out as a dashboard.
//!
//! The page is fed by the same [`MeterSnapshot`](zg01_core::meters::MeterSnapshot)
//! as the Mixage and Micro screens. A channel that cannot be read is drawn
//! dashed; a valid frame at the floor is a solid empty bar. The channel
//! value is the card control's 0–127 step, never an invented dB readout.

use gtk4::prelude::*;
use gtk4::{self as gtk, Orientation};

use zg01_core::mixage::MeterId;
use zg01_core::model::AppState;

use crate::i18n::t;
use crate::widgets::{ChannelMeter, Meter, MeterSpec};

/// The channel labels of a surround bank, in the vendor QML order.
const CHANNELS: [&str; 8] = ["L", "R", "C", "LFE", "SBL", "SBR", "SL", "SR"];

/// The full scale of the card meter controls, in device steps.
const METER_FULL_SCALE: f64 = 127.0;

/// The two kinds of meter the page uses.
enum MeterKind {
    /// A tall microphone meter.
    Vertical(Meter),
    /// One horizontal channel row.
    Horizontal(ChannelMeter),
}

/// One meter widget, the channel it shows and its value label.
struct Metered {
    id: MeterId,
    channel: usize,
    meter: MeterKind,
    value: gtk::Label,
}

/// The Meters page.
pub(crate) struct MetersScreen {
    root: gtk::Box,
    content: gtk::Box,
    legacy: gtk::Label,
    meters: Vec<Metered>,
}

impl MetersScreen {
    /// Builds the page. The first snapshot fills the readings.
    pub(crate) fn new(_state: &AppState) -> Self {
        let root = gtk::Box::new(Orientation::Vertical, 14);
        root.set_margin_top(24);
        root.set_margin_bottom(24);
        root.set_margin_start(24);
        root.set_margin_end(24);

        let title = gtk::Label::new(Some(&t("Card meters")));
        title.add_css_class("page-title");
        title.set_xalign(0.0);
        root.append(&title);

        let subtitle = gtk::Label::new(Some(&t("Card levels, 0–127 steps.")));
        subtitle.add_css_class("page-subtitle");
        subtitle.set_xalign(0.0);
        root.append(&subtitle);

        let legacy = gtk::Label::new(Some(&t(
            "Legacy driver: the card meters use the four-meter view.",
        )));
        legacy.add_css_class("meter-label");
        legacy.set_xalign(0.0);
        legacy.set_visible(false);
        root.append(&legacy);

        let content = gtk::Box::new(Orientation::Horizontal, 14);
        content.set_hexpand(true);
        content.set_valign(gtk::Align::Start);
        let mut meters = Vec::new();

        let banks = gtk::Grid::new();
        banks.set_column_spacing(14);
        banks.set_row_spacing(14);
        banks.set_column_homogeneous(true);
        banks.set_hexpand(true);
        banks.set_valign(gtk::Align::Start);
        for (index, (id, title)) in [
            (MeterId::PcIn, "PC IN"),
            (MeterId::HdmiIn, "HDMI IN"),
            (MeterId::GameMix, "GAME MIX"),
            (MeterId::StreamMix, "STREAM MIX"),
        ]
        .into_iter()
        .enumerate()
        {
            let card = build_bank(id, title, &mut meters);
            banks.attach(&card, (index % 2) as i32, (index / 2) as i32, 1, 1);
        }
        content.append(&banks);
        content.append(&build_mic_card(&mut meters));

        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&content)
            .build();
        root.append(&scrolled);

        MetersScreen {
            root,
            content,
            legacy,
            meters,
        }
    }

    /// The widget to put in the page stack.
    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Shows or hides the legacy notice and the multichannel banks.
    pub(crate) fn set_legacy(&self, legacy: bool) {
        self.legacy.set_visible(legacy);
        self.content.set_visible(!legacy);
    }

    /// Installs one group's channel readings.
    pub(crate) fn set_meter(&self, id: MeterId, levels: &[Option<i64>]) {
        for metered in &self.meters {
            if metered.id != id {
                continue;
            }
            let step = levels.get(metered.channel).copied().flatten();
            let level = step.map(|steps| (steps as f64 / METER_FULL_SCALE).clamp(0.0, 1.0));
            match &metered.meter {
                MeterKind::Vertical(meter) => meter.set_card_mono(level),
                MeterKind::Horizontal(meter) => meter.set_level(level),
            }
            match step {
                Some(step) => {
                    metered.value.set_text(&step.to_string());
                    metered.value.remove_css_class("unknown");
                }
                None => {
                    metered.value.set_text("—");
                    metered.value.add_css_class("unknown");
                }
            }
        }
    }
}

/// Builds the microphone card: two tall meters, IN and OUT.
fn build_mic_card(meters: &mut Vec<Metered>) -> gtk::Box {
    let card = gtk::Box::new(Orientation::Vertical, 12);
    card.add_css_class("panel");
    card.add_css_class("meter-card");
    card.set_width_request(220);
    card.set_hexpand(false);
    card.set_valign(gtk::Align::Start);

    let heading = gtk::Label::new(Some(&t("MIC")));
    heading.add_css_class("meter-card-title");
    heading.set_xalign(0.0);
    card.append(&heading);

    let column = gtk::Box::new(Orientation::Vertical, 16);
    column.set_halign(gtk::Align::Center);
    column.set_hexpand(true);
    for (id, label) in [(MeterId::MicIn, "IN"), (MeterId::MicOut, "OUT")] {
        let mut spec = MeterSpec::mono(label, None, true, None, false);
        spec.bar_width = 22.0;
        let meter = Meter::new(spec);
        let value = channel_value();
        value.add_css_class("mic-value");

        let block = gtk::Box::new(Orientation::Vertical, 4);
        block.set_halign(gtk::Align::Center);
        block.append(meter.widget());
        block.append(&value);
        column.append(&block);

        meters.push(Metered {
            id,
            channel: 0,
            meter: MeterKind::Vertical(meter),
            value,
        });
    }
    card.append(&column);
    card
}

/// Builds one eight-channel bank card.
fn build_bank(id: MeterId, title: &str, meters: &mut Vec<Metered>) -> gtk::Box {
    let card = gtk::Box::new(Orientation::Vertical, 10);
    card.add_css_class("panel");
    card.add_css_class("meter-card");
    card.set_width_request(260);
    card.set_hexpand(true);

    let heading = gtk::Label::new(Some(&t(title)));
    heading.add_css_class("meter-card-title");
    heading.set_xalign(0.0);
    card.append(&heading);

    let grid = gtk::Grid::new();
    grid.set_column_spacing(10);
    grid.set_row_spacing(4);
    grid.set_hexpand(true);
    for (channel, channel_label) in CHANNELS.iter().enumerate() {
        let label = gtk::Label::new(Some(channel_label));
        label.add_css_class("channel-label");
        label.set_xalign(0.0);
        label.set_width_chars(3);

        let accessible = format!("{title} {channel_label}");
        let meter = ChannelMeter::new(&accessible, None);
        let value = channel_value();

        grid.attach(&label, 0, channel as i32, 1, 1);
        grid.attach(meter.widget(), 1, channel as i32, 1, 1);
        grid.attach(&value, 2, channel as i32, 1, 1);

        meters.push(Metered {
            id,
            channel,
            meter: MeterKind::Horizontal(meter),
            value,
        });
    }
    card.append(&grid);
    card
}

/// A fixed-width value label, showing an em dash when unreadable.
fn channel_value() -> gtk::Label {
    let label = gtk::Label::new(Some("—"));
    label.add_css_class("channel-value");
    label.set_xalign(0.5);
    label.set_width_chars(4);
    label
}

#[cfg(test)]
mod tests {
    use gtk4 as gtk;
    use gtk4::prelude::*;

    use zg01_core::layout::Layout;
    use zg01_core::model::{AppState, DeviceState};

    use super::MetersScreen;

    #[test]
    #[ignore = "needs a GTK display; run under Xvfb with --ignored"]
    fn the_page_fits_a_default_window() {
        gtk::init().expect("a GTK display, e.g. Xvfb");
        let state = AppState::new(
            DeviceState::Disconnected,
            Vec::new(),
            Layout {
                screens: Vec::new(),
            },
        );
        let screen = MetersScreen::new(&state);
        let root = screen.widget();
        let (_, natural) = root.preferred_size();
        assert!(
            natural.height() <= 680,
            "the Meters page must scroll: natural height {}",
            natural.height()
        );
    }
}
