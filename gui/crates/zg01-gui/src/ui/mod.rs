//! The screens of the main area: Mixage, Micro and Avancé.
//!
//! Task 6 builds Mixage. Micro and Avancé are still the shell
//! placeholders until their own tasks replace them.

mod mixage;
mod mixage_model;

pub(crate) use mixage::{MixageEvent, MixageScreen};
