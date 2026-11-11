//! The Mixage model of the screen.
//!
//! The model is GTK-free and lives in `zg01-core`, so the integration
//! tests in `zg01-core/tests/mixage.rs` build it without a display. This
//! module is the single import path the screen uses, and the place the
//! later screens (Avancé) take the model from.

pub(crate) use zg01_core::mixage::{MeterId, MixageModel, StreamOutputModel, StripId, StripModel};
