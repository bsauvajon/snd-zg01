//! The Micro model of the screen.
//!
//! The model is GTK-free and lives in `zg01-core`, so the integration
//! tests in `zg01-core/tests/mic.rs` build it without a display. This
//! module is the single import path the screen uses.

// The screen names the types it needs; re-exporting the whole model keeps
// the model's public surface in one import path.
#[allow(unused_imports)]
pub(crate) use zg01_core::mic::{BlockModel, EqBandModel, EqModel, MicModel};
