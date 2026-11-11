//! The card meter page: the six groups, with every channel of the four
//! surround banks.
//!
//! The page is fed by the same [`MeterSnapshot`](zg01_core::meters::MeterSnapshot)
//! as the Mixage and Micro screens. A channel that cannot be read is drawn
//! dashed; a valid frame at the floor is a solid empty bar.

use gtk4::prelude::*;
use gtk4::{self as gtk, Orientation};

use zg01_core::mixage::MeterId;
use zg01_core::model::AppState;

use crate::i18n::t;
use crate::widgets::{Meter, MeterSpec};

/// The channel labels of a surround bank, in the vendor QML order.
const CHANNELS: [&str; 8] = ["L", "R", "C", "LFE", "SBL", "SBR", "SL", "SR"];

/// One meter widget and the channel it shows.
struct Metered {
    id: MeterId,
    channel: usize,
    meter: Meter,
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

        let legacy = gtk::Label::new(Some(&t(
            "Legacy driver: the card meters use the four-meter view.",
        )));
        legacy.add_css_class("meter-label");
        legacy.set_xalign(0.0);
        legacy.set_visible(false);
        root.append(&legacy);

        let content = gtk::Box::new(Orientation::Vertical, 14);
        let mut meters = Vec::new();
        for (group, label, channels) in [
            ("MIC IN", "MIC IN", &["IN"][..]),
            ("MIC OUT", "MIC OUT", &["OUT"][..]),
            ("PC IN", "PC IN", &CHANNELS[..]),
            ("HDMI IN", "HDMI IN", &CHANNELS[..]),
            ("GAME MIX", "GAME MIX", &CHANNELS[..]),
            ("STREAM MIX", "STREAM MIX", &CHANNELS[..]),
        ] {
            let id = match group {
                "MIC IN" => MeterId::MicIn,
                "MIC OUT" => MeterId::MicOut,
                "PC IN" => MeterId::PcIn,
                "HDMI IN" => MeterId::HdmiIn,
                "GAME MIX" => MeterId::GameMix,
                _ => MeterId::StreamMix,
            };
            let heading = gtk::Label::new(Some(&t(label)));
            heading.add_css_class("meter-label");
            heading.set_xalign(0.0);
            content.append(&heading);

            let row = gtk::Box::new(Orientation::Horizontal, 8);
            row.set_halign(gtk::Align::Start);
            for (channel, channel_label) in channels.iter().enumerate() {
                let meter = Meter::new(MeterSpec::mono(channel_label, None, true, None, false));
                row.append(meter.widget());
                meters.push(Metered { id, channel, meter });
            }
            content.append(&row);
        }
        root.append(&content);

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
            if metered.id == id {
                let level = levels.get(metered.channel).copied().flatten();
                metered
                    .meter
                    .set_card_mono(level.map(|steps| (steps as f64 / 127.0).clamp(0.0, 1.0)));
            }
        }
    }
}
