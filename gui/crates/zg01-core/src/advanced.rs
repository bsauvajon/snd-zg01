//! The Avancé screen model: every control the curated screens leave over.
//!
//! The model is pure data, like [`crate::model::AppState`]: it enumerates
//! the controls the driver publishes and keeps the ones no curated screen
//! displays. It is the escape hatch for the whole ALSA surface, not a
//! replacement for Mixage or Micro.
//!
//! Three rules follow the device:
//!
//! * a control claimed by the layout table, by [`crate::mixage::MixageModel`]
//!   or by [`crate::mic::MicModel`] never appears here: a control is shown
//!   once;
//! * the raw marker is the generated control map's `provenance`, not the
//!   control name: a vendor-model row ending in `Steps` is a normal control,
//!   and several raw rows do not end in `Steps`;
//! * a value the GUI does not know stays [`ControlValue::Unknown`], so the
//!   screen shows it as unknown instead of a fake zero. The four meter
//!   controls are the usual case: they return `-ENODATA` while the device
//!   does not stream, and they are listed read-only.
//!
//! `Save to ZG01` and `Reset to ZG01` are not rows: they are write-only
//! device actions, not settings.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::layout::Layout;
use crate::mic::MicModel;
use crate::mixage::MixageModel;
use crate::model::{Control, ControlValue, Unit};
use crate::preset::control_map;

/// One control of the Avancé table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvancedRow {
    /// The ALSA name of the control.
    pub name: String,
    /// The value the GUI last read, or [`ControlValue::Unknown`].
    pub value: ControlValue,
    /// The unit the value is expressed in.
    pub unit: Unit,
    /// Whether the driver accepts writes.
    pub writable: bool,
    /// Whether the generated control map marks this address as raw.
    pub raw: bool,
    /// The console section the control belongs to, derived from its name.
    pub group: String,
}

/// The whole Avancé screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvancedModel {
    /// Every control the curated screens do not display, in driver order.
    pub rows: Vec<AdvancedRow>,
}

impl AdvancedModel {
    /// Builds the table from the controls the driver publishes.
    ///
    /// `layout` is the curated table; `mixage` and `mic` are the models of
    /// the two curated screens. A control any of them displays is skipped.
    pub fn from_controls(
        controls: &[Control],
        layout: &Layout,
        mixage: &MixageModel,
        mic: &MicModel,
    ) -> Self {
        let claimed = claimed_controls(layout, mixage, mic);
        let raw = raw_names();

        let rows = controls
            .iter()
            .filter(|control| !claimed.contains(control.name()))
            .filter(|control| !control.is_action())
            .map(|control| AdvancedRow {
                name: control.name().to_string(),
                value: control.value,
                unit: control.unit,
                writable: control.is_writable(),
                raw: raw.contains(control.name()),
                group: group_of(control.name()),
            })
            .collect();

        AdvancedModel { rows }
    }

    /// The rows whose name contains `query`, case-insensitively.
    ///
    /// An empty query (or one that is only whitespace) matches every row.
    pub fn filter(&self, query: &str) -> Vec<&AdvancedRow> {
        let query = query.trim().to_lowercase();
        self.rows
            .iter()
            .filter(|row| row.name.to_lowercase().contains(&query))
            .collect()
    }
}

/// Every control name a curated screen displays.
fn claimed_controls<'a>(
    layout: &'a Layout,
    mixage: &'a MixageModel,
    mic: &'a MicModel,
) -> HashSet<&'a str> {
    let mut claimed = HashSet::new();

    for screen in &layout.screens {
        for group in &screen.groups {
            for widget in &group.controls {
                claimed.insert(widget.name.as_str());
            }
        }
    }

    for strip in &mixage.strips {
        for name in [
            strip.level.as_deref(),
            strip.mute.as_deref(),
            strip.fx.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            claimed.insert(name);
        }
    }
    for name in [
        mixage.output.usb.as_deref(),
        mixage.output.hdmi.as_deref(),
        mixage.output.monitor.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        claimed.insert(name);
    }

    for block in [&mic.gate, &mic.comp, &mic.limiter] {
        for name in [block.toggle.as_deref(), block.value.as_deref()]
            .into_iter()
            .flatten()
        {
            claimed.insert(name);
        }
        for name in &block.advanced {
            claimed.insert(name.as_str());
        }
    }
    if let Some(name) = &mic.eq.toggle {
        claimed.insert(name.as_str());
    }
    for band in &mic.eq.bands {
        claimed.insert(band.gain.as_str());
        claimed.insert(band.frequency.as_str());
        claimed.insert(band.q.as_str());
        if let Some(shape) = &band.shape {
            claimed.insert(shape.as_str());
        }
    }

    claimed
}

/// The control names the generated map marks as raw.
///
/// The map is parsed once; the set is then a cheap lookup for every row.
fn raw_names() -> &'static HashSet<&'static str> {
    static RAW: OnceLock<HashSet<&'static str>> = OnceLock::new();
    RAW.get_or_init(|| {
        control_map()
            .iter()
            .filter(|entry| entry.provenance == "raw")
            .map(|entry| entry.control_name.as_str())
            .collect()
    })
}

/// The console section of a control, derived from its ALSA name.
///
/// The generated map does not carry a section, and the curated screens own
/// the only hand-written groups, so the table groups its own rows. The
/// mapping is presentational: it never filters a control out.
fn group_of(name: &str) -> String {
    let group = if name.starts_with("P48V")
        || name.starts_with("Mute ")
        || name.starts_with("Gate")
        || name.starts_with("Compressor")
        || name.starts_with("Limiter")
        || name.starts_with("Mic ")
        || name.starts_with("EQ ")
    {
        "MIC"
    } else if name.starts_with("Streaming") {
        "STREAMING"
    } else if name.starts_with("Monitor") {
        "MONITOR"
    } else if name.starts_with("Game") || name.starts_with("Current Game") {
        "GAME"
    } else if name.starts_with("Voice") {
        "VOICE"
    } else if name.starts_with("Binaural") {
        "BINAURAL"
    } else if name.starts_with("Sound Effect") || name.starts_with("Echo") {
        "EFFETS"
    } else if name.starts_with("HDMI") {
        "HDMI"
    } else if name.starts_with("LED") {
        "SYSTÈME"
    } else if name.starts_with("Preset") {
        "PRESET"
    } else if name.starts_with("Dirty") {
        "ÉTAT"
    } else if name.starts_with("To PC") {
        "PC"
    } else {
        return name
            .split_whitespace()
            .next()
            .unwrap_or(name)
            .to_uppercase();
    };
    group.to_string()
}

#[cfg(test)]
mod tests {
    use super::{group_of, AdvancedModel};
    use crate::alsa::{ControlInfo, ControlKind};
    use crate::layout::Layout;
    use crate::mic::MicModel;
    use crate::mixage::MixageModel;
    use crate::model::{Control, ControlValue, Unit};

    fn control(name: &str, writable: bool) -> Control {
        Control {
            info: ControlInfo {
                name: name.to_string(),
                kind: ControlKind::Integer,
                min: 0,
                max: 100,
                step: 0,
                writable,
            },
            value: ControlValue::Known(1),
            unit: Unit::Steps,
        }
    }

    fn empty_layout() -> Layout {
        Layout {
            screens: Vec::new(),
        }
    }

    #[test]
    fn an_empty_surface_builds_no_rows() {
        let model = AdvancedModel::from_controls(
            &[],
            &empty_layout(),
            &MixageModel::from_controls(&[]),
            &MicModel::from_controls(&[]),
        );
        assert!(model.rows.is_empty());
        assert!(model.filter("").is_empty());
    }

    #[test]
    fn a_control_the_layout_claims_is_skipped() {
        let controls = [control("Gate", true), control("P48V Capture Switch", true)];
        let layout = crate::layout::load_layout_str(crate::layout::LAYOUT_TOML)
            .expect("the compiled table parses");
        let model = AdvancedModel::from_controls(
            &controls,
            &layout,
            &MixageModel::from_controls(&[]),
            &MicModel::from_controls(&[]),
        );
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].name, "P48V Capture Switch");
    }

    #[test]
    fn the_group_is_derived_from_the_name() {
        assert_eq!(group_of("P48V Capture Switch"), "MIC");
        assert_eq!(group_of("Streaming Mic Link Steps"), "STREAMING");
        assert_eq!(group_of("Game EQ Band 1 Gain"), "GAME");
        assert_eq!(group_of("Monitor Select Steps"), "MONITOR");
        assert_eq!(group_of("Binaural Voice Switch Steps"), "BINAURAL");
        assert_eq!(group_of("Unknown Thing"), "UNKNOWN");
    }

    #[test]
    fn the_filter_trims_the_query() {
        let controls = [
            control("P48V Capture Switch", true),
            control("Mute Capture Switch", true),
        ];
        let model = AdvancedModel::from_controls(
            &controls,
            &empty_layout(),
            &MixageModel::from_controls(&[]),
            &MicModel::from_controls(&[]),
        );
        assert_eq!(model.filter("  p48v  ").len(), 1);
        assert_eq!(model.filter("capture").len(), 2);
    }
}
