//! Loading a `.zgpreset` file and planning the values it applies.
//!
//! A `.zgpreset` file holds raw device values: the parser of
//! `tools/zgpreset.c` reports one address, width and value per parameter and
//! knows nothing about the ALSA controls or the user units. This module
//! joins the two: it resolves every address through the generated control
//! map (`tools/vendor/control-map-zg01.json`), converts the raw value to the
//! unit the control uses and plans what can be applied.
//!
//! A parameter is **skipped**, never guessed:
//!
//! - its address is not in the control map, or the card does not publish
//!   the control ([`SkipReason::Missing`]);
//! - the control is read-only ([`SkipReason::ReadOnly`]);
//! - the row is a raw vendor-model exception and the user did not opt in
//!   ([`SkipReason::Raw`]); gating follows the `provenance` field, not the
//!   control name: many vendor-model rows end in `Steps` and do apply;
//! - the converted user value is outside the ALSA control's own
//!   `info.min`/`info.max`, or the conversion had to clamp the raw value
//!   ([`SkipReason::OutOfRange`]); it is never clamped into range.
//!
//! Applying a plan writes the values through the coalescing writer. It does
//! **not** call Save: the values only persist when the user presses the Save
//! action.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use thiserror::Error;

use crate::model::Control;
use crate::units::{raw_to_user, user_to_raw, Conv};
use crate::writer::ControlWriter;

/// `ENOTSUP`, the parser's answer to a `.zg01` backup.
const ENOTSUP: c_int = 95;
/// `EINVAL`, the parser's answer to malformed input.
const EINVAL: c_int = 22;

/// One addressable parameter, as the C parser reports it.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct RawParam {
    address: u16,
    width: u8,
    value: u32,
}

/// The parser's view of a preset, mirroring `struct zgpreset` of
/// `tools/zgpreset.h`.
#[repr(C)]
struct RawPreset {
    block: [c_char; 32],
    name: *mut c_char,
    params: *mut RawParam,
    count: usize,
}

extern "C" {
    fn zgpreset_open(path: *const c_char, out: *mut *mut RawPreset) -> c_int;
    fn zgpreset_block(preset: *const RawPreset) -> *const c_char;
    fn zgpreset_name(preset: *const RawPreset) -> *const c_char;
    fn zgpreset_close(preset: *mut RawPreset);
}

/// Closes the parser's allocation whatever happens after `zgpreset_open`.
struct RawPresetGuard(*mut RawPreset);

impl Drop for RawPresetGuard {
    fn drop(&mut self) {
        // SAFETY: the guard owns the pointer and only drops it once.
        unsafe { zgpreset_close(self.0) };
    }
}

/// One parameter of a preset file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetParam {
    /// The device address of the parameter.
    pub address: u16,
    /// The width of the value in the file, in bytes.
    pub width: u8,
    /// The raw device value.
    pub value: u32,
}

/// A parsed `.zgpreset` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    /// The block the preset belongs to, e.g. `MIC INPUT`.
    pub block: String,
    /// The display name, empty when the file carries none.
    pub name: String,
    /// Every parameter, in file order.
    pub params: Vec<PresetParam>,
}

/// Why a `.zgpreset` file could not be read.
#[derive(Debug, Error)]
pub enum PresetError {
    /// The file is a `.zg01` backup, which the parser does not support.
    #[error(".zg01 backups are not supported; load a .zgpreset file instead")]
    BackupNotSupported,
    /// The file is not a valid preset.
    #[error("the file is not a valid .zgpreset file")]
    Malformed,
    /// The file could not be read.
    #[error("the preset could not be read: {0}")]
    Io(#[from] std::io::Error),
    /// The path cannot be passed to the C parser.
    #[error("the preset path contains a NUL byte")]
    InvalidPath,
    /// The parser failed with a code that is not an I/O error.
    #[error("the preset parser failed with code {0}")]
    Parser(c_int),
}

/// Reads a `.zgpreset` file.
///
/// A `.zg01` backup is rejected with [`PresetError::BackupNotSupported`], a
/// malformed file with [`PresetError::Malformed`] and an unreadable one with
/// the I/O error. The display name is never truncated.
pub fn load_preset(path: &Path) -> Result<Preset, PresetError> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| PresetError::InvalidPath)?;
    let mut raw: *mut RawPreset = ptr::null_mut();

    // SAFETY: the path is a valid NUL-terminated string and `raw` points to
    // a writable pointer the parser initialises.
    let status = unsafe { zgpreset_open(path.as_ptr(), &mut raw) };
    if status != 0 {
        return Err(preset_error(status));
    }
    let preset = RawPresetGuard(raw);

    // SAFETY: a successful `zgpreset_open` returned a live preset; the two
    // accessors return a NUL-terminated string, empty when there is none.
    let block = unsafe { CStr::from_ptr(zgpreset_block(preset.0)) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: as above.
    let name = unsafe { CStr::from_ptr(zgpreset_name(preset.0)) }
        .to_string_lossy()
        .into_owned();

    // SAFETY: the parser sets `count` to the length of the `params` array,
    // which is only null when the count is zero (and a valid preset has at
    // least one parameter).
    let params = unsafe {
        let parsed = &*preset.0;
        if parsed.count == 0 || parsed.params.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(parsed.params, parsed.count)
                .iter()
                .map(|param| PresetParam {
                    address: param.address,
                    width: param.width,
                    value: param.value,
                })
                .collect()
        }
    };

    Ok(Preset {
        block,
        name,
        params,
    })
}

/// Turns the parser's status into the error the UI reports.
fn preset_error(status: c_int) -> PresetError {
    if status == -ENOTSUP {
        PresetError::BackupNotSupported
    } else if status == -EINVAL {
        PresetError::Malformed
    } else if status < 0 {
        PresetError::Io(std::io::Error::from_raw_os_error(-status))
    } else {
        PresetError::Parser(status)
    }
}

/// The generated control map, parsed once.
const CONTROL_MAP_JSON: &str = include_str!("../../../../tools/vendor/control-map-zg01.json");

/// One row of the generated control map.
#[derive(Debug, Clone, Deserialize)]
pub struct ControlMapEntry {
    /// The device address the preset stores.
    pub address: u16,
    /// The ALSA control the address maps to.
    pub control_name: String,
    /// The conversion between the raw device value and the user value.
    pub conv: Conv,
    /// Whether the generated map says the control accepts writes.
    pub writable: bool,
    /// `existing`, `vendor-model`, `raw` or `read-only`.
    pub provenance: String,
    /// The lowest user value of the control.
    pub min: i64,
    /// The highest user value of the control.
    pub max: i64,
}

static CONTROL_MAP: OnceLock<Vec<ControlMapEntry>> = OnceLock::new();

/// The generated control map, parsed from `tools/vendor/control-map-zg01.json`.
pub fn control_map() -> &'static [ControlMapEntry] {
    CONTROL_MAP.get_or_init(|| {
        serde_json::from_str(CONTROL_MAP_JSON).expect("the generated control map is valid JSON")
    })
}

/// The map row of one address.
pub fn control_map_entry(address: u16) -> Option<&'static ControlMapEntry> {
    control_map().iter().find(|entry| entry.address == address)
}

/// Why one preset parameter is not applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The address is unknown, or the card does not publish its control.
    Missing,
    /// The control does not accept writes.
    ReadOnly,
    /// A raw row, and the user did not opt in.
    Raw,
    /// The converted user value is outside the ALSA control's range.
    OutOfRange,
}

/// One value the plan applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apply {
    /// The device address of the parameter.
    pub address: u16,
    /// The ALSA name of the control.
    pub name: String,
    /// The user value to write.
    pub value: i64,
}

/// One value that reached the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The device address of the parameter.
    pub address: u16,
    /// The ALSA name of the control.
    pub name: String,
    /// The user value that was written.
    pub value: i64,
}

/// One value the driver refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    /// The device address of the parameter.
    pub address: u16,
    /// The ALSA name of the control.
    pub name: String,
    /// What the driver answered.
    pub error: String,
}

/// One preset parameter the plan does not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    /// The device address of the parameter.
    pub address: u16,
    /// Why it is not applied.
    pub reason: SkipReason,
}

/// What a preset would do to the card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetPlan {
    /// The values to write, in preset order.
    pub applies: Vec<Apply>,
    /// The parameters left alone, with their reason.
    pub skipped: Vec<Skip>,
}

/// The outcome of applying a plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// The values that reached the device.
    pub applied: Vec<Applied>,
    /// The parameters that were not applied, as planned.
    pub skipped: Vec<Skip>,
    /// The controls whose write failed, with the driver's error.
    pub failed: Vec<Failed>,
    /// Whether the user cancelled the operation before every value landed.
    pub cancelled: bool,
}

/// A token that stops a preset application between two writes.
///
/// The check happens before every write, so cancelling never interrupts a
/// write that is already in flight. Reset and disconnect cancel the token
/// before they act, and the operation reports itself as cancelled instead
/// of claiming success.
#[derive(Debug, Clone, Default)]
pub struct ApplyCancellation {
    cancelled: Arc<AtomicBool>,
}

impl ApplyCancellation {
    /// A token that is not cancelled yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the application to stop before its next write.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether the operation was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Resolves every preset parameter through the control map.
///
/// `controls` is the card's control surface: a map row whose control the
/// card does not publish is skipped, never applied by address alone. Raw
/// rows are skipped unless `allow_raw` is set. A value the conversion clamps
/// is out of range, not silently clamped.
pub fn plan_preset(preset: &Preset, controls: &[Control], allow_raw: bool) -> PresetPlan {
    let mut plan = PresetPlan::default();

    for param in &preset.params {
        let Some(entry) = control_map_entry(param.address) else {
            plan.skipped.push(Skip {
                address: param.address,
                reason: SkipReason::Missing,
            });
            continue;
        };
        let Some(control) = controls
            .iter()
            .find(|control| control.name() == entry.control_name)
        else {
            plan.skipped.push(Skip {
                address: param.address,
                reason: SkipReason::Missing,
            });
            continue;
        };
        if !entry.writable || !control.is_writable() {
            plan.skipped.push(Skip {
                address: param.address,
                reason: SkipReason::ReadOnly,
            });
            continue;
        }
        if entry.provenance == "raw" && !allow_raw {
            plan.skipped.push(Skip {
                address: param.address,
                reason: SkipReason::Raw,
            });
            continue;
        }

        let raw = i64::from(param.value);
        let user = raw_to_user(entry.conv, raw);
        // The ALSA range is in user units. The round trip catches a raw
        // value the conversion had to clamp (a Q above the last device
        // step), which the range check alone would miss.
        if user < control.info.min
            || user > control.info.max
            || user_to_raw(entry.conv, user) != raw
        {
            plan.skipped.push(Skip {
                address: param.address,
                reason: SkipReason::OutOfRange,
            });
            continue;
        }

        plan.applies.push(Apply {
            address: param.address,
            name: entry.control_name.clone(),
            value: user,
        });
    }

    plan
}

/// Writes every value of `plan`, one awaited write per control.
///
/// The report lists what was written, what the plan skipped and what the
/// driver refused. Save is not called: a preset only persists when the user
/// presses the header action.
pub fn apply_plan(plan: &PresetPlan, writer: &ControlWriter) -> ApplyReport {
    apply_plan_cancellable(plan, writer, &ApplyCancellation::new(), |_| {})
}

/// Writes every value of `plan`, stopping between writes when cancelled.
///
/// `progress` receives the report after every write, so the UI can show the
/// applied, skipped and failed counters while the operation runs. A
/// cancelled operation stops before its next write, marks the report
/// `cancelled`, and never claims the remaining values were applied.
pub fn apply_plan_cancellable(
    plan: &PresetPlan,
    writer: &ControlWriter,
    cancellation: &ApplyCancellation,
    mut progress: impl FnMut(&ApplyReport),
) -> ApplyReport {
    let mut report = ApplyReport {
        applied: Vec::new(),
        skipped: plan.skipped.clone(),
        failed: Vec::new(),
        cancelled: false,
    };

    for apply in &plan.applies {
        if cancellation.is_cancelled() {
            report.cancelled = true;
            break;
        }
        match writer.set_and_wait(&apply.name, apply.value) {
            Ok(()) => report.applied.push(Applied {
                address: apply.address,
                name: apply.name.clone(),
                value: apply.value,
            }),
            Err(error) => report.failed.push(Failed {
                address: apply.address,
                name: apply.name.clone(),
                error,
            }),
        }
        progress(&report);
    }

    report
}
