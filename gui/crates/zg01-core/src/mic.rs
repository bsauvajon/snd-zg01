//! The Micro screen model: the three dynamics blocks and the four-band EQ.
//!
//! The model is pure data, like [`crate::mixage::MixageModel`]: it names
//! the ALSA controls the screen shows and never touches GTK. A control the
//! driver does not publish is `None`, which hides its widget; a block keeps
//! the sub-parameters that do exist, in driver order.
//!
//! Three rules follow the hardware:
//!
//! * the dynamics chain is three faders, not four: `Mic Level Status` is
//!   read-only and absent, and the per-block out gains are advanced
//!   sub-parameters, not faders;
//! * the EQ has four bands, each a gain/frequency/Q triple; the shelf
//!   shapes are the existing `EQ Low Shape` (band 1) and `EQ High Shape`
//!   (band 4);
//! * the advanced list only holds names the device actually publishes.

use crate::model::Control;

/// One dynamics block: GATE, COMP or LIMIT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockModel {
    /// The ALSA name of the enable switch, when the driver publishes it.
    pub toggle: Option<String>,
    /// The ALSA name of the level fader, when the driver publishes it.
    pub value: Option<String>,
    /// The ALSA names of the sub-parameters, in the order they are shown.
    ///
    /// Only controls that exist are listed; a missing one is hidden.
    pub advanced: Vec<String>,
}

/// One band of the four-band EQ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqBandModel {
    /// The ALSA name of the gain fader.
    pub gain: String,
    /// The ALSA name of the frequency editor.
    pub frequency: String,
    /// The ALSA name of the Q editor.
    pub q: String,
    /// The ALSA name of the shelf-shape switch, on the outer bands.
    pub shape: Option<String>,
}

/// The four-band EQ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqModel {
    /// The ALSA name of the EQ enable switch.
    pub toggle: Option<String>,
    /// The bands that exist, in band order (1 to 4).
    pub bands: Vec<EqBandModel>,
}

/// The whole Micro screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicModel {
    /// The gate block.
    pub gate: BlockModel,
    /// The compressor block.
    pub comp: BlockModel,
    /// The limiter block.
    pub limiter: BlockModel,
    /// The four-band EQ.
    pub eq: EqModel,
}

impl MicModel {
    /// Builds the screen from the controls the driver publishes.
    ///
    /// A missing control is `None`, never an invented name. An EQ band is
    /// built only when its gain, frequency and Q all exist; the shapes
    /// belong to bands 1 and 4 and are hidden when their control is absent.
    pub fn from_controls(controls: &[Control]) -> Self {
        MicModel {
            gate: block(
                controls,
                "Gate Capture Switch",
                "Gate",
                &[
                    "Gate Threshold",
                    "Gate Range",
                    "Gate Attack",
                    "Gate Hold",
                    "Gate Decay",
                ],
            ),
            comp: block(
                controls,
                "Compressor Capture Switch",
                "Compressor",
                &[
                    "Compressor Threshold",
                    "Compressor Ratio",
                    "Compressor Attack",
                    "Compressor Release",
                    "Compressor Knee",
                    "Compressor Out Gain",
                ],
            ),
            limiter: block(
                controls,
                "Limiter Capture Switch",
                "Limiter",
                &[
                    "Limiter Threshold",
                    "Limiter Attack",
                    "Limiter Release",
                    "Limiter Knee",
                    "Limiter Out Gain",
                ],
            ),
            eq: EqModel {
                toggle: find(controls, "EQ Capture Switch"),
                bands: (1..=4)
                    .filter_map(|band| band_model(controls, band))
                    .collect(),
            },
        }
    }
}

/// Builds one dynamics block from the controls that exist.
fn block(controls: &[Control], toggle: &str, value: &str, advanced: &[&str]) -> BlockModel {
    BlockModel {
        toggle: find(controls, toggle),
        value: find(controls, value),
        advanced: advanced
            .iter()
            .filter(|name| exists(controls, name))
            .map(|name| (*name).to_string())
            .collect(),
    }
}

/// Builds one EQ band, or `None` when one of its three values is missing.
fn band_model(controls: &[Control], band: u8) -> Option<EqBandModel> {
    let gain = format!("EQ Band {band} Gain");
    let frequency = format!("EQ Band {band} Frequency");
    let q = format!("EQ Band {band} Q");
    if !exists(controls, &gain) || !exists(controls, &frequency) || !exists(controls, &q) {
        return None;
    }

    let shape = match band {
        1 => find(controls, "EQ Low Shape"),
        4 => find(controls, "EQ High Shape"),
        _ => None,
    };

    Some(EqBandModel {
        gain,
        frequency,
        q,
        shape,
    })
}

/// The name of `control` when the driver publishes it.
fn find(controls: &[Control], name: &str) -> Option<String> {
    exists(controls, name).then(|| name.to_string())
}

/// Whether the driver publishes a control named `name`.
fn exists(controls: &[Control], name: &str) -> bool {
    controls.iter().any(|control| control.name() == name)
}

#[cfg(test)]
mod tests {
    use super::MicModel;
    use crate::alsa::{ControlInfo, ControlKind};
    use crate::model::{Control, ControlValue, Unit};

    fn control(name: &str) -> Control {
        Control {
            info: ControlInfo {
                name: name.to_string(),
                kind: ControlKind::Integer,
                min: 0,
                max: 100,
                step: 0,
                count: 1,
                writable: true,
            },
            value: ControlValue::Known(0),
            unit: Unit::Steps,
        }
    }

    #[test]
    fn an_empty_surface_builds_no_invented_controls() {
        let model = MicModel::from_controls(&[]);
        assert_eq!(model.gate.toggle, None);
        assert_eq!(model.comp.value, None);
        assert!(model.gate.advanced.is_empty());
        assert_eq!(model.eq.toggle, None);
        assert!(model.eq.bands.is_empty());
    }

    #[test]
    fn a_band_without_its_q_is_not_built() {
        let controls = [
            "EQ Band 1 Gain",
            "EQ Band 1 Frequency",
            "EQ Band 2 Gain",
            "EQ Band 2 Frequency",
            "EQ Band 2 Q",
        ]
        .map(control);
        let model = MicModel::from_controls(&controls);
        assert_eq!(model.eq.bands.len(), 1);
        assert_eq!(model.eq.bands[0].gain, "EQ Band 2 Gain");
    }
}
