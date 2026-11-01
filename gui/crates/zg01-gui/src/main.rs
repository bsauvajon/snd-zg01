//! GTK4/libadwaita control GUI for the Yamaha ZG01.
//!
//! Task 0 only provides the application skeleton: a single window with a
//! placeholder body. The ALSA control surface, the ZG01 model screens and the
//! `.zgpreset` handling are added by later tasks.

use gtk4::prelude::*;
use relm4::prelude::*;

/// Application ID, used for the desktop entry and the D-Bus name.
const APP_ID: &str = "io.github.bsauvajon.Zg01Control";

/// Title of the main window.
const APP_TITLE: &str = "ZG01 Control";

/// Default width of the main window, in logical pixels.
const WINDOW_WIDTH: i32 = 1100;

/// Default height of the main window, in logical pixels.
const WINDOW_HEIGHT: i32 = 680;

/// Application model. Empty until the control surface lands.
#[derive(Debug)]
struct Model;

#[relm4::component]
impl SimpleComponent for Model {
    type Init = ();
    type Input = ();
    type Output = ();

    view! {
        adw::ApplicationWindow {
            set_title: Some(APP_TITLE),
            set_default_width: WINDOW_WIDTH,
            set_default_height: WINDOW_HEIGHT,
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_vexpand: true,
                gtk::Label {
                    set_label: APP_TITLE,
                    set_vexpand: true,
                }
            }
        }
    }

    fn init(
        _init: Self::Init,
        _root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = Model;

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("starting {APP_TITLE}");

    let app = RelmApp::new(APP_ID);
    app.run::<Model>(());
}
