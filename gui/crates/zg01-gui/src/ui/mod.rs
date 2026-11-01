//! The screens of the main area: Mixage, Micro and Avancé.
//!
//! Task 6 builds Mixage, Task 7 builds Micro. Avancé is still the shell
//! placeholder until its own task replaces it.

mod mic;
mod mic_model;
mod mixage;
mod mixage_model;

pub(crate) use mic::{MicEvent, MicScreen};
pub(crate) use mixage::{MixageEvent, MixageScreen};
