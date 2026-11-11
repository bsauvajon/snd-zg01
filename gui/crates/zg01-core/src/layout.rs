//! The curated layout table: which control goes on which screen.
//!
//! The table is `gui/data/layout.toml`, compiled into the binary
//! ([`LAYOUT_TOML`]). It says where a control is shown, in which group,
//! with which label and in which unit, and it is the reason a control
//! never shows up twice: a control a curated screen claims does not
//! appear in the advanced screen.
//!
//! The table is written by hand and reviewed, it is never generated from
//! the vendor model at runtime.
//!
//! Two rules make it survive a driver that does not publish every
//! control:
//!
//! * a control the device does not publish is a warning, not an error:
//!   its widget is hidden;
//! * a control claimed twice is an error: the table is wrong.

use std::collections::HashMap;

use serde::Deserialize;
use thiserror::Error;

use crate::alsa::ControlInfo;
use crate::model::Unit;

/// The layout table, compiled into the binary.
pub const LAYOUT_TOML: &str = include_str!("../../../data/layout.toml");

/// The widget a control is shown as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetKind {
    /// A slider.
    Fader,
    /// A switch.
    Toggle,
    /// A number entry.
    Number,
}

/// One control of a group.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WidgetSpec {
    /// The ALSA name of the control.
    pub name: String,
    /// The widget the control is shown as.
    pub widget: WidgetKind,
    /// The label to show, when the driver name is not the one to show.
    #[serde(default)]
    pub label: Option<String>,
    /// The unit to show the value in.
    #[serde(default)]
    pub unit: Option<Unit>,
}

/// A group of controls of a screen.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Group {
    /// The identifier of the group, unique in its screen.
    #[serde(default)]
    pub id: String,
    /// The title of the group.
    pub title: String,
    /// The controls of the group, in the order they are shown.
    #[serde(default)]
    pub controls: Vec<WidgetSpec>,
}

/// A screen of the GUI.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Screen {
    /// The identifier of the screen, unique in the table.
    #[serde(default)]
    pub id: String,
    /// The title of the screen.
    pub title: String,
    /// The groups of the screen, in the order they are shown.
    #[serde(rename = "group", default)]
    pub groups: Vec<Group>,
}

/// The whole layout table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Layout {
    /// The screens of the table, in the order they are shown.
    #[serde(rename = "screen", default)]
    pub screens: Vec<Screen>,
}

/// What can go wrong while reading the layout table.
#[derive(Debug, Error)]
pub enum LayoutError {
    /// The table is not valid TOML, or a field is missing or unknown.
    #[error("the layout table is not valid: {0}")]
    Parse(#[from] toml::de::Error),
    /// A screen or a group has no identifier.
    #[error("the layout table has a {kind} without an identifier")]
    MissingId {
        /// What the identifier is missing from.
        kind: &'static str,
    },
    /// Two screens or two groups share an identifier.
    #[error("the layout table has two {kind} with the identifier {id:?}")]
    DuplicateId {
        /// What the identifier is shared by.
        kind: &'static str,
        /// The shared identifier.
        id: String,
    },
}

/// How bad a finding of the validation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The GUI can carry on: the widget of the control is hidden.
    Warning,
    /// The layout table is wrong and has to be fixed.
    Error,
}

/// What the validation of a layout table found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutWarning {
    /// The table claims a control the device does not publish.
    ///
    /// The widget of the control is hidden; the GUI carries on.
    MissingControl {
        /// The name of the control.
        name: String,
        /// The screen that claims it.
        screen: String,
        /// The group that claims it.
        group: String,
    },
    /// The table claims the same control twice.
    ///
    /// The table is wrong: a control belongs to one place only.
    DuplicateControl {
        /// The name of the control.
        name: String,
        /// The screen that claims it again.
        screen: String,
        /// The group that claims it again.
        group: String,
        /// The group that claimed it first.
        first_group: String,
    },
}

impl LayoutWarning {
    /// How bad the finding is.
    pub fn severity(&self) -> Severity {
        match self {
            LayoutWarning::MissingControl { .. } => Severity::Warning,
            LayoutWarning::DuplicateControl { .. } => Severity::Error,
        }
    }

    /// Whether the finding is an error.
    pub fn is_error(&self) -> bool {
        self.severity() == Severity::Error
    }

    /// The finding as a sentence, for the log.
    pub fn message(&self) -> String {
        match self {
            LayoutWarning::MissingControl { name, screen, group } => {
                format!("the control {name:?} of the group {group:?} of the screen {screen:?} does not exist")
            }
            LayoutWarning::DuplicateControl {
                name,
                screen,
                group,
                first_group,
            } => format!(
                "the control {name:?} is claimed twice, by the group {first_group:?} and by the group {group:?} of the screen {screen:?}"
            ),
        }
    }
}

/// Reads a layout table.
///
/// The table is checked for the identifiers it must carry: a screen or a
/// group without an identifier, or two of them sharing one, is an error.
pub fn load_layout_str(text: &str) -> Result<Layout, LayoutError> {
    let layout: Layout = toml::from_str(text)?;

    let mut screen_ids = HashMap::new();
    for screen in &layout.screens {
        if screen.id.is_empty() {
            return Err(LayoutError::MissingId { kind: "screen" });
        }
        if screen_ids.insert(screen.id.clone(), ()).is_some() {
            return Err(LayoutError::DuplicateId {
                kind: "screen",
                id: screen.id.clone(),
            });
        }

        let mut group_ids = HashMap::new();
        for group in &screen.groups {
            if group.id.is_empty() {
                return Err(LayoutError::MissingId { kind: "group" });
            }
            if group_ids.insert(group.id.clone(), ()).is_some() {
                return Err(LayoutError::DuplicateId {
                    kind: "group",
                    id: group.id.clone(),
                });
            }
        }
    }

    Ok(layout)
}

/// Checks a layout table against the controls the device publishes.
///
/// A control the device does not publish is a warning: the GUI hides its
/// widget. A control the table claims twice is an error: the table is
/// wrong.
pub fn validate_layout(layout: &Layout, controls: &[ControlInfo]) -> Vec<LayoutWarning> {
    let mut warnings = Vec::new();
    let mut claimed: HashMap<&str, &str> = HashMap::new();

    for screen in &layout.screens {
        for group in &screen.groups {
            for widget in &group.controls {
                if let Some(first_group) = claimed.get(widget.name.as_str()) {
                    warnings.push(LayoutWarning::DuplicateControl {
                        name: widget.name.clone(),
                        screen: screen.id.clone(),
                        group: group.id.clone(),
                        first_group: (*first_group).to_string(),
                    });
                    continue;
                }
                claimed.insert(widget.name.as_str(), group.id.as_str());

                if !controls.iter().any(|control| control.name == widget.name) {
                    warnings.push(LayoutWarning::MissingControl {
                        name: widget.name.clone(),
                        screen: screen.id.clone(),
                        group: group.id.clone(),
                    });
                }
            }
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::{load_layout_str, validate_layout, LayoutWarning, Severity, LAYOUT_TOML};
    use crate::alsa::{ControlInfo, ControlKind};

    fn info(name: &str) -> ControlInfo {
        ControlInfo {
            name: name.to_string(),
            kind: ControlKind::Integer,
            min: 0,
            max: 100,
            step: 0,
            writable: true,
        }
    }

    #[test]
    fn the_compiled_table_is_valid() {
        let layout = load_layout_str(LAYOUT_TOML).expect("the compiled table parses");
        assert!(!layout.screens.is_empty());
    }

    #[test]
    fn a_screen_without_an_identifier_is_an_error() {
        let error = load_layout_str(
            r#"
[[screen]]
title = "Micro"
"#,
        )
        .expect_err("the table is rejected");
        assert!(matches!(
            error,
            super::LayoutError::MissingId { kind: "screen" }
        ));
    }

    #[test]
    fn two_screens_with_the_same_identifier_are_an_error() {
        let error = load_layout_str(
            r#"
[[screen]]
id = "mic"
title = "Micro"

[[screen]]
id = "mic"
title = "Micro again"
"#,
        )
        .expect_err("the table is rejected");
        assert!(matches!(
            error,
            super::LayoutError::DuplicateId { kind: "screen", .. }
        ));
    }

    #[test]
    fn a_missing_control_is_a_warning_and_a_duplicate_is_an_error() {
        let layout = load_layout_str(
            r#"
[[screen]]
id = "mic"
title = "Micro"

[[screen.group]]
id = "gate"
title = "GATE"
controls = [
  { name = "Gate", widget = "fader" },
  { name = "Gate", widget = "number" },
  { name = "Nope", widget = "toggle" },
]
"#,
        )
        .expect("the table parses");
        let warnings = validate_layout(&layout, &[info("Gate")]);
        assert_eq!(warnings.len(), 2);
        assert_eq!(
            warnings
                .iter()
                .filter(|warning| warning.severity() == Severity::Warning)
                .count(),
            1
        );
        assert_eq!(
            warnings
                .iter()
                .filter(|warning| warning.severity() == Severity::Error)
                .count(),
            1
        );
        assert!(matches!(
            &warnings[0],
            LayoutWarning::DuplicateControl { name, .. } if name == "Gate"
        ));
        assert!(matches!(
            &warnings[1],
            LayoutWarning::MissingControl { name, .. } if name == "Nope"
        ));
    }
}
