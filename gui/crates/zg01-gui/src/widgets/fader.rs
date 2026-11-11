//! The vertical fader.
//!
//! A fader shows one integer control: a value label above, a narrow
//! vertical track with tick marks in the middle and the control label
//! below. The track is drawn with Cairo because `GtkScale` cannot be
//! styled down to a 3 px line with a 15 x 7 px handle.
//!
//! The pointer jumps to the clicked position and then drags; the scroll
//! wheel and the arrow keys step the value; `Home` and `End` go to the
//! ends; a double click returns to the neutral value; holding Shift makes
//! every pointer move finer. The fader never reads the device: it reports
//! a change through the callback and is corrected with [`Fader::set_value`]
//! when the write lands (or fails).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::glib::Propagation;
use gtk4::prelude::*;
use gtk4::{self as gtk, AccessibleProperty, AccessibleRole, Orientation};
use zg01_core::geometry::{fraction_to_value, snap_to_step, value_to_fraction};
use zg01_core::model::{format_value, ControlValue, Unit};

use super::{rounded_rect, theme};
use crate::i18n::tf;

/// What the fader shows while the device value is unknown.
const UNKNOWN_TEXT: &str = "—";

/// The width of the drawing area, in logical pixels.
const AREA_WIDTH: i32 = 48;

/// The default height of the drawing area, in logical pixels.
const AREA_HEIGHT: i32 = 150;

/// The width of the track, in logical pixels.
const TRACK_WIDTH: f64 = 3.0;

/// The width of the handle, in logical pixels.
const HANDLE_WIDTH: f64 = 15.0;

/// The height of the handle, in logical pixels.
const HANDLE_HEIGHT: f64 = 7.0;

/// The length of a tick mark, in logical pixels.
const TICK_LENGTH: f64 = 5.0;

/// The vertical space between the track and the drawing area edge.
const VERTICAL_PADDING: f64 = 10.0;

/// How much a pointer move counts while Shift is held.
const FINE_FACTOR: f64 = 0.25;

/// One control shown as a fader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FaderSpec {
    /// The ALSA name of the control.
    pub(crate) name: String,
    /// The label shown under the track.
    pub(crate) label: String,
    /// The unit the value is shown in.
    pub(crate) unit: Unit,
    /// The lowest value of the control.
    pub(crate) min: i64,
    /// The highest value of the control.
    pub(crate) max: i64,
    /// The increment of the control; zero means one.
    pub(crate) step: i64,
    /// The value the fader starts with.
    pub(crate) value: ControlValue,
}

/// A value the user picked on a fader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FaderOutput {
    /// The ALSA name of the control.
    pub(crate) name: String,
    /// The new value.
    pub(crate) value: i64,
}

/// The state shared by the fader's widgets and its signal handlers.
struct FaderInner {
    spec: FaderSpec,
    value: RefCell<ControlValue>,
    on_change: Box<dyn Fn(FaderOutput)>,
}

impl FaderInner {
    /// The value the fader currently shows.
    fn value(&self) -> ControlValue {
        *self.value.borrow()
    }

    /// Installs `value`, redraws, and reports the change when `emit` is set.
    fn set_value(
        &self,
        area: &gtk::DrawingArea,
        label: &gtk::Label,
        value: ControlValue,
        emit: bool,
    ) {
        if self.value() == value {
            return;
        }
        *self.value.borrow_mut() = value;
        refresh(area, label, &self.spec, value);
        area.queue_draw();
        if emit {
            if let Some(value) = value.known() {
                (self.on_change)(FaderOutput {
                    name: self.spec.name.clone(),
                    value,
                });
            }
        }
    }

    /// Moves the value by `notches` increments.
    ///
    /// A value the GUI no longer knows is not a reason to refuse the
    /// keyboard: the first key press starts from a defined end of the
    /// range (the bottom for an increase, the top for a decrease), which
    /// makes that one value known again.
    fn adjust(&self, area: &gtk::DrawingArea, label: &gtk::Label, notches: i64) {
        let value = adjusted_value(
            self.value().known(),
            notches,
            self.spec.min,
            self.spec.max,
            self.spec.step,
        );
        self.set_value(area, label, ControlValue::Known(value), true);
    }

    /// Moves the value to one end of its range.
    fn set_boundary(&self, area: &gtk::DrawingArea, label: &gtk::Label, value: i64) {
        self.set_value(area, label, ControlValue::Known(value), true);
    }

    /// Moves the value to the position `fraction` of the track.
    fn set_fraction(&self, area: &gtk::DrawingArea, label: &gtk::Label, fraction: f64) {
        let value = fraction_to_value(fraction, self.spec.min, self.spec.max, self.spec.step);
        self.set_value(area, label, ControlValue::Known(value), true);
    }

    /// The neutral value: the bottom of the track, or 0 dB for a gain.
    fn default_value(&self) -> i64 {
        default_value(&self.spec)
    }

    /// The fraction of the current value, or the bottom when unknown.
    fn fraction(&self) -> f64 {
        self.value()
            .known()
            .map(|value| value_to_fraction(value, self.spec.min, self.spec.max))
            .unwrap_or(0.0)
    }
}

/// A vertical fader.
pub(crate) struct Fader {
    root: gtk::Box,
    area: gtk::DrawingArea,
    value_label: gtk::Label,
    inner: Rc<FaderInner>,
}

impl Fader {
    /// Builds a fader for `spec`; `on_change` receives every user change.
    pub(crate) fn new(spec: FaderSpec, on_change: impl Fn(FaderOutput) + 'static) -> Self {
        let root = gtk::Box::new(Orientation::Vertical, 4);
        root.add_css_class("fader-root");
        root.set_halign(gtk::Align::Center);

        let value_label = gtk::Label::new(None);
        value_label.add_css_class("fader-value");
        value_label.set_xalign(0.5);
        value_label.set_width_chars(8);
        root.append(&value_label);

        let area = gtk::DrawingArea::new();
        area.set_content_width(AREA_WIDTH);
        area.set_content_height(AREA_HEIGHT);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_focus_on_click(true);
        area.set_accessible_role(AccessibleRole::Slider);
        area.add_css_class("fader");
        root.append(&area);

        let name_label = gtk::Label::new(Some(&spec.label));
        name_label.add_css_class("fader-label");
        name_label.set_xalign(0.5);
        name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name_label.set_max_width_chars(12);
        name_label.set_tooltip_text(Some(&spec.label));
        root.append(&name_label);

        let inner = Rc::new(FaderInner {
            value: RefCell::new(spec.value),
            spec,
            on_change: Box::new(on_change),
        });
        refresh(&area, &value_label, &inner.spec, inner.value());

        let draw_inner = inner.clone();
        area.set_draw_func(move |area, context, width, height| {
            draw(area, context, width, height, &draw_inner);
        });

        area.connect_has_focus_notify(|area| area.queue_draw());

        install_pointer(&area, &value_label, &inner);
        install_scroll(&area, &value_label, &inner);
        install_keys(&area, &value_label, &inner);

        Fader {
            root,
            area,
            value_label,
            inner,
        }
    }

    /// The widget to put in a layout.
    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The value the fader currently shows.
    pub(crate) fn value(&self) -> ControlValue {
        self.inner.value()
    }

    /// Installs a value read from the device, without reporting a change.
    pub(crate) fn set_value(&self, value: ControlValue) {
        self.inner
            .set_value(&self.area, &self.value_label, value, false);
    }

    /// Enables or disables the fader.
    pub(crate) fn set_sensitive(&self, sensitive: bool) {
        self.root.set_sensitive(sensitive);
    }
}

/// Installs the click, double-click and drag handlers.
fn install_pointer(area: &gtk::DrawingArea, label: &gtk::Label, inner: &Rc<FaderInner>) {
    let dragging = Rc::new(Cell::new(false));
    let anchor = Rc::new(Cell::new(0.0));

    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let press_area = area.clone();
    let press_label = label.clone();
    let press_inner = inner.clone();
    let press_dragging = dragging.clone();
    let press_anchor = anchor.clone();
    click.connect_pressed(move |gesture, n_press, _x, y| {
        press_area.grab_focus();
        let fine = gesture
            .current_event_state()
            .contains(gdk::ModifierType::SHIFT_MASK);
        if n_press >= 2 {
            let value = press_inner.default_value();
            press_inner.set_value(&press_area, &press_label, ControlValue::Known(value), true);
        } else if !fine {
            let (top, bottom) = track_bounds(press_area.height());
            press_inner.set_fraction(&press_area, &press_label, fraction_from_y(y, top, bottom));
        }
        press_anchor.set(press_inner.fraction());
        press_dragging.set(true);
    });
    let released = dragging.clone();
    click.connect_released(move |_, _, _, _| released.set(false));
    area.add_controller(click);

    let drag = gtk::GestureDrag::new();
    drag.set_button(gdk::BUTTON_PRIMARY);
    let drag_area = area.clone();
    let drag_label = label.clone();
    let drag_inner = inner.clone();
    let drag_dragging = dragging.clone();
    let drag_anchor = anchor.clone();
    drag.connect_drag_update(move |gesture, _offset_x, offset_y| {
        if !drag_dragging.get() {
            return;
        }
        let (top, bottom) = track_bounds(drag_area.height());
        let length = (bottom - top).max(1.0);
        let fine = gesture
            .current_event_state()
            .contains(gdk::ModifierType::SHIFT_MASK);
        let scale = if fine { FINE_FACTOR } else { 1.0 };
        let fraction = (drag_anchor.get() - offset_y / length * scale).clamp(0.0, 1.0);
        drag_inner.set_fraction(&drag_area, &drag_label, fraction);
    });
    let end_dragging = dragging.clone();
    drag.connect_drag_end(move |_, _, _| end_dragging.set(false));
    area.add_controller(drag);
}

/// Installs the scroll-wheel handler.
fn install_scroll(area: &gtk::DrawingArea, label: &gtk::Label, inner: &Rc<FaderInner>) {
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let scroll_area = area.clone();
    let scroll_label = label.clone();
    let scroll_inner = inner.clone();
    scroll.connect_scroll(move |_controller, _dx, dy| {
        if dy == 0.0 {
            return Propagation::Proceed;
        }
        let notches = if dy < 0.0 { 1 } else { -1 };
        scroll_inner.adjust(&scroll_area, &scroll_label, notches);
        Propagation::Stop
    });
    area.add_controller(scroll);
}

/// Installs the keyboard handler.
fn install_keys(area: &gtk::DrawingArea, label: &gtk::Label, inner: &Rc<FaderInner>) {
    let key = gtk::EventControllerKey::new();
    let key_area = area.clone();
    let key_label = label.clone();
    let key_inner = inner.clone();
    key.connect_key_pressed(move |_, key, _, _state| {
        match key {
            gdk::Key::Up | gdk::Key::Right => {
                key_inner.adjust(&key_area, &key_label, 1);
            }
            gdk::Key::Down | gdk::Key::Left => {
                key_inner.adjust(&key_area, &key_label, -1);
            }
            gdk::Key::Page_Up => {
                key_inner.adjust(&key_area, &key_label, 10);
            }
            gdk::Key::Page_Down => {
                key_inner.adjust(&key_area, &key_label, -10);
            }
            gdk::Key::Home => {
                key_inner.set_boundary(&key_area, &key_label, key_inner.spec.min);
            }
            gdk::Key::End => {
                key_inner.set_boundary(&key_area, &key_label, key_inner.spec.max);
            }
            _ => return Propagation::Proceed,
        }
        Propagation::Stop
    });
    area.add_controller(key);
}

/// Updates the label, the tooltip and the accessible properties.
fn refresh(area: &gtk::DrawingArea, label: &gtk::Label, spec: &FaderSpec, value: ControlValue) {
    let text = match value.known() {
        Some(value) => format_value(spec.unit, value),
        None => UNKNOWN_TEXT.to_string(),
    };
    let tooltip = match value.known() {
        Some(value) => tf(
            "{label}: {value}",
            &[
                ("{label}", &spec.label),
                ("{value}", &format_value(spec.unit, value)),
            ],
        ),
        None => tf("{label}: unknown value", &[("{label}", &spec.label)]),
    };

    if value.known().is_some() {
        label.remove_css_class("unknown");
    } else {
        label.add_css_class("unknown");
    }
    label.set_text(&text);
    area.set_tooltip_text(Some(&tooltip));

    let description = tf("Control {name}", &[("{name}", &spec.name)]);
    let mut properties = vec![
        gtk::accessible::Property::Label(&spec.label),
        gtk::accessible::Property::Description(&description),
        gtk::accessible::Property::Orientation(Orientation::Vertical),
        gtk::accessible::Property::ValueMin(spec.min as f64),
        gtk::accessible::Property::ValueMax(spec.max as f64),
        gtk::accessible::Property::ValueText(&text),
    ];
    if let Some(value) = value.known() {
        properties.push(gtk::accessible::Property::ValueNow(value as f64));
    } else {
        area.reset_property(AccessibleProperty::ValueNow);
    }
    area.update_property(&properties);
}

/// The value one key press or scroll notch produces.
///
/// A known value moves by `notches`, clamped and snapped to the grid. An
/// unknown value starts at the bottom of the range for an increase and at
/// the top for a decrease, so the keyboard can recover it after a reset.
fn adjusted_value(current: Option<i64>, notches: i64, min: i64, max: i64, step: i64) -> i64 {
    let Some(current) = current else {
        return if notches > 0 { min } else { max };
    };
    let delta = notches.saturating_mul(step.max(1));
    let target = current.saturating_add(delta).clamp(min, max);
    // A device correction can leave the value off the published grid; a
    // key press puts it back on the nearest step.
    snap_to_step(target, min, max, step)
}

/// The neutral value: the bottom of the track, or 0 dB for a gain.
fn default_value(spec: &FaderSpec) -> i64 {
    fraction_to_value(
        value_to_fraction(0, spec.min, spec.max),
        spec.min,
        spec.max,
        spec.step,
    )
}

/// The top and bottom of the track for a drawing area of `height`.
fn track_bounds(height: i32) -> (f64, f64) {
    let height = height.max(1) as f64;
    let top = VERTICAL_PADDING.min(height / 2.0);
    let bottom = (height - top).max(top);
    (top, bottom)
}

/// The fraction of a pointer at `y` on the track.
fn fraction_from_y(y: f64, top: f64, bottom: f64) -> f64 {
    if bottom <= top {
        return 0.0;
    }
    ((bottom - y) / (bottom - top)).clamp(0.0, 1.0)
}

/// Draws the track, the ticks, the fill and the handle.
fn draw(
    area: &gtk::DrawingArea,
    context: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    inner: &FaderInner,
) {
    let (top, bottom) = track_bounds(height);
    let length = bottom - top;
    let centre = f64::from(width) / 2.0;
    let track_x = centre - TRACK_WIDTH / 2.0;

    draw_ticks(context, centre, top, bottom);

    rounded_rect(
        context,
        track_x,
        top,
        TRACK_WIDTH,
        length,
        TRACK_WIDTH / 2.0,
    );
    theme::TRACK.set_source(context, 1.0);
    let _ = context.fill();

    match inner.value() {
        ControlValue::Known(value) => {
            let fraction = value_to_fraction(value, inner.spec.min, inner.spec.max);
            let handle_y = bottom - fraction * length;

            rounded_rect(
                context,
                track_x,
                handle_y,
                TRACK_WIDTH,
                (bottom - handle_y).max(0.0),
                TRACK_WIDTH / 2.0,
            );
            theme::ACCENT_STRONG.set_source(context, 1.0);
            let _ = context.fill();

            let handle_x = centre - HANDLE_WIDTH / 2.0;
            let handle_y = handle_y - HANDLE_HEIGHT / 2.0;
            rounded_rect(
                context,
                handle_x,
                handle_y,
                HANDLE_WIDTH,
                HANDLE_HEIGHT,
                HANDLE_HEIGHT / 2.0,
            );
            theme::ACCENT.set_source(context, 1.0);
            let _ = context.fill_preserve();
            theme::ACCENT_STRONG.set_source(context, 0.9);
            context.set_line_width(1.0);
            let _ = context.stroke();

            if area.has_focus() {
                rounded_rect(
                    context,
                    handle_x - 3.0,
                    handle_y - 3.0,
                    HANDLE_WIDTH + 6.0,
                    HANDLE_HEIGHT + 6.0,
                    HANDLE_HEIGHT / 2.0 + 3.0,
                );
                theme::ACCENT_STRONG.set_source(context, 0.85);
                context.set_line_width(1.5);
                let _ = context.stroke();
            }
        }
        ControlValue::Unknown => {
            // An unknown value is not zero: no fill and no handle, only a
            // dashed outline at the place the track would be.
            rounded_rect(
                context,
                track_x,
                top,
                TRACK_WIDTH,
                length,
                TRACK_WIDTH / 2.0,
            );
            theme::MUTED.set_source(context, 0.55);
            context.set_line_width(1.0);
            context.set_dash(&[3.0, 3.0], 0.0);
            let _ = context.stroke();
            context.set_dash(&[], 0.0);
        }
    }
}

/// Draws the five tick marks, the ends longer than the quarters.
fn draw_ticks(context: &gtk4::cairo::Context, centre: f64, top: f64, bottom: f64) {
    for (index, fraction) in [0.0, 0.25, 0.5, 0.75, 1.0].into_iter().enumerate() {
        let y = bottom - fraction * (bottom - top);
        let length = if index == 0 || index == 4 {
            TICK_LENGTH + 2.0
        } else {
            TICK_LENGTH
        };
        let left = centre - TRACK_WIDTH / 2.0 - 3.0 - length;
        let right = centre + TRACK_WIDTH / 2.0 + 3.0;
        theme::MUTED.set_source(context, 0.40);
        context.rectangle(left, y - 0.5, length, 1.0);
        context.rectangle(right, y - 0.5, length, 1.0);
    }
    let _ = context.fill();
}

#[cfg(test)]
mod tests {
    use super::{adjusted_value, default_value, fraction_from_y, track_bounds, FaderSpec};
    use zg01_core::model::{ControlValue, Unit};

    fn spec() -> FaderSpec {
        FaderSpec {
            name: "Streaming Mic Level Steps".to_string(),
            label: "MIC".to_string(),
            unit: Unit::Steps,
            min: 0,
            max: 127,
            step: 1,
            value: ControlValue::Known(100),
        }
    }

    #[test]
    fn the_track_keeps_a_margin_at_both_ends() {
        let (top, bottom) = track_bounds(150);
        assert_eq!(top, 10.0);
        assert_eq!(bottom, 140.0);
    }

    #[test]
    fn a_tiny_track_stays_sane() {
        let (top, bottom) = track_bounds(3);
        assert!(top <= bottom);
        assert!(bottom <= 3.0);
    }

    #[test]
    fn the_top_of_the_track_is_the_high_value() {
        let (top, bottom) = track_bounds(150);
        assert_eq!(fraction_from_y(top, top, bottom), 1.0);
        assert_eq!(fraction_from_y(bottom, top, bottom), 0.0);
        assert!((fraction_from_y((top + bottom) / 2.0, top, bottom) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn the_spec_carries_the_alsa_name_and_the_show_label() {
        let spec = spec();
        assert_eq!(spec.name, "Streaming Mic Level Steps");
        assert_eq!(spec.label, "MIC");
        assert_eq!(spec.value, ControlValue::Known(100));
    }

    #[test]
    fn the_double_click_resets_to_the_neutral_value() {
        // Levels and percentages go to the bottom of the track.
        assert_eq!(default_value(&spec()), 0);
        // A gain goes to 0 dB, the middle of -18.0 dB .. +18.0 dB.
        let gain = FaderSpec {
            name: "EQ Band 1 Gain".to_string(),
            label: "Gain".to_string(),
            unit: Unit::Db,
            min: -180,
            max: 180,
            step: 1,
            value: ControlValue::Known(60),
        };
        assert_eq!(default_value(&gain), 0);
        // A range that does not contain zero goes to its lowest value.
        let shifted = FaderSpec {
            min: 10,
            max: 20,
            ..gain
        };
        assert_eq!(
            default_value(&shifted),
            10,
            "the grid starts on the minimum"
        );
    }

    #[test]
    fn the_keyboard_recovers_an_unknown_value() {
        assert_eq!(
            adjusted_value(None, 1, 0, 127, 1),
            0,
            "an increase starts from the bottom"
        );
        assert_eq!(
            adjusted_value(None, -1, 0, 127, 1),
            127,
            "a decrease starts from the top"
        );
        assert_eq!(adjusted_value(Some(10), 1, 0, 127, 1), 11);
        assert_eq!(adjusted_value(Some(10), -1, 0, 127, 1), 9);
        assert_eq!(adjusted_value(Some(126), 10, 0, 127, 1), 127);
    }

    #[test]
    fn a_key_press_snaps_an_off_grid_value_back_to_the_grid() {
        // The grid itself is tested in `zg01_core::geometry`; the fader
        // only has to use it for a key press.
        assert_eq!(
            zg01_core::geometry::snap_to_step(63, 0, 100, 5),
            65,
            "the fader reports the snapped value, not the raw one"
        );
    }
}
