//! The custom widgets of the GUI: the vertical fader and the meters.
//!
//! The widgets are `gtk::DrawingArea`s drawn with Cairo rather than
//! libadwaita controls, because neither a `GtkScale` nor a `GtkLevelBar`
//! has the geometry the design asks for (a narrow vertical track, a
//! 15 x 7 px handle, a peak-hold line and a Linux meter drawn beside the
//! card meter).
//!
//! The widgets are pure presentation: they hold the value the parent gives
//! them, never read the device, and report a user change through the
//! callback the parent passes to [`Fader::new`]. The Mixage and Micro
//! screens build them.

mod fader;
mod meter;

// Re-exported for the screens.
#[allow(unused_imports)]
pub(crate) use fader::{Fader, FaderOutput, FaderSpec};
#[allow(unused_imports)]
pub(crate) use meter::{Meter, MeterSpec};

use gtk4::cairo;

/// The dark theme C tokens, mirroring `style.css`.
///
/// Cairo cannot read `@define-color` tokens, so the values live here too.
/// `the_cairo_tokens_match_the_css` keeps the two tables in step.
pub(crate) mod theme {
    /// The colour of a design token, without alpha.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Rgb {
        red: f64,
        green: f64,
        blue: f64,
    }

    impl Rgb {
        /// The token from its `0xRRGGBB` value.
        pub(crate) const fn hex(hex: u32) -> Self {
            Rgb {
                red: ((hex >> 16) & 0xff) as f64 / 255.0,
                green: ((hex >> 8) & 0xff) as f64 / 255.0,
                blue: (hex & 0xff) as f64 / 255.0,
            }
        }

        /// Sets this token as the Cairo source, with `alpha` in `0.0..=1.0`.
        pub(crate) fn set_source(self, context: &gtk4::cairo::Context, alpha: f64) {
            context.set_source_rgba(self.red, self.green, self.blue, alpha);
        }

        /// The components, for the Cairo gradients.
        pub(crate) fn components(self) -> (f64, f64, f64) {
            (self.red, self.green, self.blue)
        }
    }

    /// The fader and meter track (`#2a2f3a`).
    pub(crate) const TRACK: Rgb = Rgb::hex(0x2a_2f_3a);
    /// The primary text (`#eef1f6`).
    pub(crate) const TEXT: Rgb = Rgb::hex(0xee_f1_f6);
    /// The secondary text and the unknown state (`#8f96a3`).
    pub(crate) const MUTED: Rgb = Rgb::hex(0x8f_96_a3);
    /// The fader handle (`#c9b8ff`).
    pub(crate) const ACCENT: Rgb = Rgb::hex(0xc9_b8_ff);
    /// The fader fill and the focus ring (`#8b6ff0`).
    pub(crate) const ACCENT_STRONG: Rgb = Rgb::hex(0x8b_6f_f0);
    /// The bottom of the meter gradient (`#34c759`).
    pub(crate) const SUCCESS: Rgb = Rgb::hex(0x34_c7_59);
    /// The middle of the meter gradient (`#f0b429`).
    pub(crate) const WARNING: Rgb = Rgb::hex(0xf0_b4_29);
    /// The top of the meter gradient and the clip marker (`#f0523b`).
    pub(crate) const ERROR: Rgb = Rgb::hex(0xf0_52_3b);
}

/// Adds a rounded rectangle to the current path.
///
/// Cairo has no rounded-rectangle primitive; the four corners are arcs of
/// `radius`, clamped so a small rectangle never inverts.
fn rounded_rect(context: &cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let width = width.max(0.0);
    let height = height.max(0.0);
    let radius = radius.min(width / 2.0).min(height / 2.0).max(0.0);
    let right = x + width;
    let bottom = y + height;

    context.new_sub_path();
    context.arc(
        right - radius,
        y + radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    context.arc(
        right - radius,
        bottom - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    context.arc(
        x + radius,
        bottom - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    context.arc(
        x + radius,
        y + radius,
        radius,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    context.close_path();
}

#[cfg(test)]
mod tests {
    use super::theme;

    /// The CSS the binary installs at startup.
    const STYLE: &str = include_str!("../style.css");

    fn css_colour(colour: theme::Rgb) -> String {
        let (red, green, blue) = colour.components();
        format!(
            "#{:02x}{:02x}{:02x}",
            (red * 255.0).round() as u8,
            (green * 255.0).round() as u8,
            (blue * 255.0).round() as u8
        )
    }

    #[test]
    fn the_cairo_tokens_match_the_css() {
        for colour in [
            theme::TRACK,
            theme::TEXT,
            theme::MUTED,
            theme::ACCENT,
            theme::ACCENT_STRONG,
            theme::SUCCESS,
            theme::WARNING,
            theme::ERROR,
        ] {
            let hex = css_colour(colour);
            assert!(
                STYLE.contains(&hex),
                "the token {hex} of the Cairo table is missing from style.css"
            );
        }
    }
}
