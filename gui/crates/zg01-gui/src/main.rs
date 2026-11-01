//! GTK4/libadwaita control GUI for the Yamaha ZG01.
//!
//! This is the shell: card discovery, the state model, the header, the
//! sidebar, the footer and the connection view. The Mixage, Micro and
//! Avancé bodies, the widgets and the `.zgpreset` handling are added by
//! later tasks.

mod app;
mod widgets;
mod window;

use relm4::prelude::*;

/// Application ID, used for the desktop entry and the D-Bus name.
const APP_ID: &str = "io.github.bsauvajon.Zg01Control";

fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("starting ZG01 Control");

    let app = RelmApp::new(APP_ID);

    // v1 is dark-only: the light token set is a follow-up.
    relm4::set_global_css(include_str!("style.css"));
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);

    app.run::<app::App>(());
}
