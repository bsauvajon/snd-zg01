//! The level meters.
//!
//! A meter draws one or two vertical bars: the card meter, which is the
//! solid one, and optionally the thinner Linux (PipeWire) meter beside it,
//! tagged `PW` so the source is never ambiguous. The fill is a fixed
//! green → yellow → red gradient aligned with the track, so a given
//! height always has the same colour. The peak-hold line stays for one
//! second after the last higher peak, and the top of a bar turns red when
//! the signal clips.
//!
//! A meter with no reading (`None`) draws an outlined, greyed track: an
//! unreadable meter is not a meter that reads zero.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::cairo::{self, LinearGradient};
use gtk4::prelude::*;
use gtk4::{self as gtk, AccessibleProperty, AccessibleRole, Orientation};

use super::{rounded_rect, theme};

/// The default height of the drawing area, in logical pixels.
const AREA_HEIGHT: i32 = 150;

/// The width of the card meter, in logical pixels.
const CARD_WIDTH: f64 = 7.0;

/// The width of the Linux meter, in logical pixels.
const LINUX_WIDTH: f64 = 3.0;

/// The gap between the card and Linux bars, in logical pixels.
const METER_GAP: f64 = 4.0;

/// The margin on each side of the bars, in logical pixels.
const SIDE_MARGIN: f64 = 6.0;

/// The space above the bars, in logical pixels.
const TOP_MARGIN: f64 = 4.0;

/// The height of the `PW` badge above a Linux bar, in logical pixels.
const BADGE_HEIGHT: f64 = 14.0;

/// The space below the bars, in logical pixels.
const BOTTOM_MARGIN: f64 = 4.0;

/// The height of the peak-hold line, in logical pixels.
const PEAK_HEIGHT: f64 = 2.0;

/// The height of the clip marker, in logical pixels.
const CLIP_HEIGHT: f64 = 3.0;

/// The level at which the clip marker lights up.
const CLIP_LEVEL: f64 = 0.98;

/// How long the peak-hold line stays after the last higher peak.
const PEAK_HOLD: Duration = Duration::from_secs(1);

/// One meter shown in a strip or beside the mic chain.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeterSpec {
    /// The label shown under the bars, e.g. `MIC`.
    pub(crate) label: String,
    /// The level of the card meter, normalised to `0.0..=1.0`.
    pub(crate) card: Option<f64>,
    /// The level of the Linux meter, normalised to `0.0..=1.0`.
    pub(crate) linux: Option<f64>,
    /// The peak of the current readings, normalised to `0.0..=1.0`.
    pub(crate) peak: f64,
    /// Whether the Linux source exists, independently of its reading.
    ///
    /// A Linux source that exists but cannot be read still has a bar: it
    /// is drawn as an unknown (dashed) bar, not removed. Set this to
    /// `true` even when `linux` is `None`.
    pub(crate) linux_present: bool,
}

/// How one bar of a meter is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
enum BarState {
    /// The source does not exist: there is no bar at all.
    Absent,
    /// The source exists but has no reading: a dashed outline.
    Unknown,
    /// The source has a reading, normalised to `0.0..=1.0`.
    Level(f64),
}

/// The state of one bar: presence is separate from the reading.
fn bar_state(present: bool, reading: Option<f64>) -> BarState {
    match (present, normalise(reading)) {
        (false, _) => BarState::Absent,
        (true, Some(level)) => BarState::Level(level),
        (true, None) => BarState::Unknown,
    }
}

/// The state a meter keeps between two readings.
struct MeterInner {
    spec: RefCell<MeterSpec>,
    held_peak: Cell<f64>,
    peak_at: Cell<Option<Instant>>,
    timer_pending: Cell<bool>,
}

/// A card and Linux level meter.
pub(crate) struct Meter {
    root: gtk::Box,
    area: gtk::DrawingArea,
    inner: Rc<MeterInner>,
}

impl Meter {
    /// Builds a meter for `spec`.
    pub(crate) fn new(mut spec: MeterSpec) -> Self {
        // A reading proves that the source exists, even when the caller
        // forgot to set `linux_present`.
        spec.card = normalise(spec.card);
        spec.linux = normalise(spec.linux);
        spec.peak = normalise(Some(spec.peak)).unwrap_or(0.0);
        spec.linux_present |= spec.linux.is_some();

        let root = gtk::Box::new(Orientation::Vertical, 4);
        root.add_css_class("meter-root");
        root.set_halign(gtk::Align::Center);

        let area = gtk::DrawingArea::new();
        let width = meter_width(spec.linux_present);
        area.set_content_width(width);
        area.set_content_height(AREA_HEIGHT);
        area.set_vexpand(true);
        area.set_accessible_role(AccessibleRole::Meter);
        area.add_css_class("meter");
        root.append(&area);

        let label = gtk::Label::new(Some(&spec.label));
        label.add_css_class("meter-label");
        label.set_xalign(0.5);
        label.set_max_width_chars(10);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        root.append(&label);

        let inner = Rc::new(MeterInner {
            held_peak: Cell::new(initial_peak(&spec)),
            peak_at: Cell::new(None),
            timer_pending: Cell::new(false),
            spec: RefCell::new(spec),
        });
        if inner.held_peak.get() > 0.0 {
            inner.peak_at.set(Some(Instant::now()));
            schedule_peak_clear(&inner, &area);
        }

        refresh_accessible(&area, &inner.spec.borrow());
        let draw_inner = inner.clone();
        area.set_draw_func(move |_, context, width, height| {
            draw(context, width, height, &draw_inner);
        });

        Meter { root, area, inner }
    }

    /// The widget to put in a layout.
    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Installs new readings.
    ///
    /// `card` and `linux` are normalised levels; `None` means the meter
    /// could not be read. `peak` is the peak of the readings and updates
    /// the one-second hold.
    pub(crate) fn set_levels(&self, card: Option<f64>, linux: Option<f64>, peak: f64) {
        let card = normalise(card);
        let linux = normalise(linux);
        let peak = normalise(Some(peak)).unwrap_or(0.0);
        let had_linux = self.inner.spec.borrow().linux_present;

        {
            let mut spec = self.inner.spec.borrow_mut();
            spec.card = card;
            spec.linux = linux;
            spec.peak = peak;
            // A reading proves presence; a missing reading never removes
            // a source that exists, so its bar stays visible as unknown.
            spec.linux_present |= linux.is_some();
        }
        if !had_linux && linux.is_some() {
            self.area.set_content_width(meter_width(true));
        }
        refresh_accessible(&self.area, &self.inner.spec.borrow());
        self.area.queue_draw();

        let current = card.into_iter().chain(linux).fold(0.0, f64::max);
        let candidate = peak.max(current);
        if candidate > self.inner.held_peak.get() {
            self.inner.held_peak.set(candidate);
            self.inner.peak_at.set(Some(Instant::now()));
            schedule_peak_clear(&self.inner, &self.area);
        }
    }

    /// Enables or dims the meter.
    #[allow(dead_code)] // TODO(task-7): the Micro screen dims its meters.
    pub(crate) fn set_sensitive(&self, sensitive: bool) {
        self.root.set_sensitive(sensitive);
    }
}

/// The peak a fresh meter shows: the spec peak, or the current level.
fn initial_peak(spec: &MeterSpec) -> f64 {
    let peak = normalise(Some(spec.peak)).unwrap_or(0.0);
    let level = spec
        .card
        .into_iter()
        .chain(spec.linux)
        .filter_map(|level| normalise(Some(level)))
        .fold(0.0, f64::max);
    peak.max(level)
}

/// The width of a meter drawing area.
fn meter_width(has_linux: bool) -> i32 {
    let bars = CARD_WIDTH
        + if has_linux {
            METER_GAP + LINUX_WIDTH
        } else {
            0.0
        };
    (bars + 2.0 * SIDE_MARGIN).round() as i32
}

/// Clamps a level to `0.0..=1.0`; a non-finite level is not a reading.
fn normalise(level: Option<f64>) -> Option<f64> {
    level
        .filter(|level| level.is_finite())
        .map(|level| level.clamp(0.0, 1.0))
}

/// The remaining peak hold after `elapsed`, or `None` when it is over.
///
/// A newer peak moves the deadline; a timer armed for an older peak must
/// wait only for the remainder, never a fresh `PEAK_HOLD`.
fn peak_clear_delay(elapsed: Duration) -> Option<Duration> {
    let remaining = PEAK_HOLD.saturating_sub(elapsed);
    if remaining.is_zero() {
        None
    } else {
        Some(remaining)
    }
}

/// Schedules the end of the peak hold, if no timer is pending.
///
/// A newer, higher peak moves the deadline; the timer that fires early
/// notices it and waits for the remainder of that deadline instead of
/// clearing the line or starting another full second.
fn schedule_peak_clear(inner: &Rc<MeterInner>, area: &gtk::DrawingArea) {
    if inner.timer_pending.replace(true) {
        return;
    }
    schedule_peak_timer(inner, area);
}

/// Arms one timer for the remaining hold, or clears an expired peak.
fn schedule_peak_timer(inner: &Rc<MeterInner>, area: &gtk::DrawingArea) {
    let Some(at) = inner.peak_at.get() else {
        clear_peak(inner, area);
        return;
    };
    let Some(delay) = peak_clear_delay(at.elapsed()) else {
        clear_peak(inner, area);
        return;
    };

    let inner = inner.clone();
    let area = area.clone();
    let _ = gtk4::glib::timeout_add_local_once(delay, move || {
        // A newer peak may have moved the deadline while we waited.
        let still_held = inner
            .peak_at
            .get()
            .and_then(|at| peak_clear_delay(at.elapsed()))
            .is_some();
        if still_held {
            schedule_peak_timer(&inner, &area);
        } else {
            clear_peak(&inner, &area);
        }
    });
}

/// Drops the held peak and repaints.
fn clear_peak(inner: &Rc<MeterInner>, area: &gtk::DrawingArea) {
    inner.timer_pending.set(false);
    inner.held_peak.set(0.0);
    inner.peak_at.set(None);
    area.queue_draw();
}

/// Updates the accessible label and value of the meter.
fn refresh_accessible(area: &gtk::DrawingArea, spec: &MeterSpec) {
    let level = spec.card.into_iter().chain(spec.linux).fold(0.0, f64::max);
    let any = spec.card.is_some() || spec.linux.is_some();
    let text = if any {
        format!("{} %", (level * 100.0).round() as i32)
    } else {
        "inconnu".to_string()
    };
    let description = format!("Mètre {}", spec.label);

    let mut properties = vec![
        gtk::accessible::Property::Label(&spec.label),
        gtk::accessible::Property::Description(&description),
        gtk::accessible::Property::Orientation(Orientation::Vertical),
        gtk::accessible::Property::ValueMin(0.0),
        gtk::accessible::Property::ValueMax(100.0),
        gtk::accessible::Property::ValueText(&text),
    ];
    if any {
        properties.push(gtk::accessible::Property::ValueNow(level * 100.0));
    } else {
        area.reset_property(AccessibleProperty::ValueNow);
    }
    area.update_property(&properties);
}

/// Draws the tracks, the fills, the peak line, the clip markers and `PW`.
fn draw(context: &cairo::Context, width: i32, height: i32, inner: &MeterInner) {
    let spec = inner.spec.borrow();
    let has_linux = spec.linux_present;
    let top = if has_linux {
        TOP_MARGIN + BADGE_HEIGHT
    } else {
        TOP_MARGIN
    };
    let bottom = (f64::from(height) - BOTTOM_MARGIN).max(top);
    let length = bottom - top;

    let bars = CARD_WIDTH
        + if has_linux {
            METER_GAP + LINUX_WIDTH
        } else {
            0.0
        };
    let start = (f64::from(width) - bars) / 2.0;
    let card_x = start;
    let linux_x = start + CARD_WIDTH + METER_GAP;

    // The card meter always exists; only its reading can be unknown. The
    // Linux source may be absent, in which case it has no bar at all.
    let card_state = bar_state(true, spec.card);
    let linux_state = bar_state(has_linux, spec.linux);

    draw_track(context, card_x, CARD_WIDTH, top, bottom);
    if has_linux {
        draw_track(context, linux_x, LINUX_WIDTH, top, bottom);
    }

    match card_state {
        BarState::Unknown => draw_unknown(context, card_x, CARD_WIDTH, top, bottom),
        BarState::Level(level) => {
            draw_fill(context, card_x, CARD_WIDTH, level, top, bottom, 1.0);
            draw_clip(context, card_x, CARD_WIDTH, level, top);
        }
        BarState::Absent => {}
    }
    match linux_state {
        BarState::Unknown => draw_unknown(context, linux_x, LINUX_WIDTH, top, bottom),
        BarState::Level(level) => {
            draw_fill(context, linux_x, LINUX_WIDTH, level, top, bottom, 0.65);
            draw_clip(context, linux_x, LINUX_WIDTH, level, top);
        }
        BarState::Absent => {}
    }

    let held = inner.held_peak.get();
    if held > 0.0 {
        let y = bottom - held * length;
        let right = if has_linux {
            linux_x + LINUX_WIDTH
        } else {
            card_x + CARD_WIDTH
        };
        rounded_rect(
            context,
            card_x - 1.0,
            y - PEAK_HEIGHT / 2.0,
            right - card_x + 2.0,
            PEAK_HEIGHT,
            PEAK_HEIGHT / 2.0,
        );
        theme::TEXT.set_source(context, 0.9);
        let _ = context.fill();
    }

    if has_linux {
        draw_badge(context, width);
    }
}

/// Draws the empty track of one bar.
fn draw_track(context: &cairo::Context, x: f64, width: f64, top: f64, bottom: f64) {
    rounded_rect(context, x, top, width, bottom - top, width / 2.0);
    theme::TRACK.set_source(context, 1.0);
    let _ = context.fill();
}

/// Outlines a bar whose meter could not be read.
///
/// A silent meter is a solid empty track; an unreadable one is dashed, so
/// the two states never look the same.
fn draw_unknown(context: &cairo::Context, x: f64, width: f64, top: f64, bottom: f64) {
    rounded_rect(context, x, top, width, bottom - top, width / 2.0);
    theme::MUTED.set_source(context, 0.45);
    context.set_line_width(1.0);
    context.set_dash(&[3.0, 3.0], 0.0);
    let _ = context.stroke();
    context.set_dash(&[], 0.0);
}

/// Draws the filled part of one bar with the fixed level gradient.
fn draw_fill(
    context: &cairo::Context,
    x: f64,
    width: f64,
    level: f64,
    top: f64,
    bottom: f64,
    alpha: f64,
) {
    let height = (bottom - top).max(0.0);
    let filled = level.clamp(0.0, 1.0) * height;
    if filled <= 0.0 {
        return;
    }
    let gradient = level_gradient(bottom, top, alpha);
    let _ = context.set_source(&gradient);
    rounded_rect(context, x, bottom - filled, width, filled, width / 2.0);
    let _ = context.fill();
}

/// The green → yellow → red gradient of the meter, aligned with the track.
fn level_gradient(bottom: f64, top: f64, alpha: f64) -> LinearGradient {
    let gradient = LinearGradient::new(0.0, bottom, 0.0, top);
    for (offset, colour) in [
        (0.0, theme::SUCCESS),
        (0.55, theme::SUCCESS),
        (0.78, theme::WARNING),
        (1.0, theme::ERROR),
    ] {
        let (red, green, blue) = colour.components();
        gradient.add_color_stop_rgba(offset, red, green, blue, alpha);
    }
    gradient
}

/// Draws the red clip marker at the top of a clipping bar.
///
/// The marker is wider than the bar so it stays visible when the fill
/// itself is already red at full scale.
fn draw_clip(context: &cairo::Context, x: f64, width: f64, level: f64, top: f64) {
    if !is_clipping(level) {
        return;
    }
    rounded_rect(
        context,
        x - 2.0,
        top,
        width + 4.0,
        CLIP_HEIGHT,
        CLIP_HEIGHT / 2.0,
    );
    theme::ERROR.set_source(context, 1.0);
    let _ = context.fill();
}

/// Whether a level is close enough to full scale to show the clip marker.
fn is_clipping(level: f64) -> bool {
    level >= CLIP_LEVEL
}

/// Draws the `PW` badge above the Linux bar.
fn draw_badge(context: &cairo::Context, width: i32) {
    context.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    context.set_font_size(11.0);
    if let Ok(extents) = context.text_extents("PW") {
        let x = f64::from(width) - extents.width() - SIDE_MARGIN;
        let y = TOP_MARGIN + extents.height();
        context.move_to(x.max(0.0), y);
        theme::MUTED.set_source(context, 0.9);
        let _ = context.show_text("PW");
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        bar_state, initial_peak, is_clipping, meter_width, normalise, peak_clear_delay, BarState,
        MeterSpec, PEAK_HOLD,
    };

    #[test]
    fn the_card_meter_is_narrower_than_the_pair() {
        assert_eq!(meter_width(false), 19);
        assert_eq!(meter_width(true), 26);
    }

    #[test]
    fn a_level_is_clamped_to_the_unit_interval() {
        assert_eq!(normalise(Some(-0.5)), Some(0.0));
        assert_eq!(normalise(Some(1.5)), Some(1.0));
        assert_eq!(normalise(Some(0.42)), Some(0.42));
        assert_eq!(normalise(None), None);
    }

    #[test]
    fn a_non_finite_level_is_not_a_reading() {
        assert_eq!(normalise(Some(f64::NAN)), None);
        assert_eq!(normalise(Some(f64::INFINITY)), None);
    }

    #[test]
    fn a_meter_spec_carries_the_card_and_linux_readings() {
        let spec = MeterSpec {
            label: "MIC".to_string(),
            card: Some(0.5),
            linux: None,
            peak: 0.7,
            linux_present: false,
        };
        assert_eq!(spec.label, "MIC");
        assert_eq!(spec.card, Some(0.5));
        assert_eq!(spec.linux, None);
    }

    #[test]
    fn a_fresh_meter_holds_the_peak_of_its_spec() {
        let spec = MeterSpec {
            label: "MIC".to_string(),
            card: Some(0.4),
            linux: None,
            peak: 0.8,
            linux_present: false,
        };
        assert_eq!(initial_peak(&spec), 0.8);
        let spec = MeterSpec {
            label: "MIC".to_string(),
            card: Some(0.9),
            linux: None,
            peak: 0.2,
            linux_present: false,
        };
        assert_eq!(initial_peak(&spec), 0.9);
    }

    #[test]
    fn clipping_is_near_full_scale() {
        assert!(is_clipping(0.99));
        assert!(is_clipping(1.0));
        assert!(!is_clipping(0.97));
        assert!(!is_clipping(0.0));
    }

    #[test]
    fn the_peak_hold_never_exceeds_one_second() {
        assert_eq!(peak_clear_delay(Duration::ZERO), Some(PEAK_HOLD));
        assert_eq!(
            peak_clear_delay(Duration::from_millis(200)),
            Some(Duration::from_millis(800))
        );
        assert_eq!(
            peak_clear_delay(Duration::from_millis(900)),
            Some(Duration::from_millis(100))
        );
        assert_eq!(peak_clear_delay(PEAK_HOLD), None);
        assert_eq!(peak_clear_delay(Duration::from_secs(2)), None);

        // A peak that arrives 200 ms before the old deadline makes the
        // early timer wait 800 ms: 200 + 800 is one second from the new
        // peak, not a fresh second plus the old wait.
        let since_new_peak = Duration::from_millis(200);
        let remaining = peak_clear_delay(since_new_peak).expect("the peak is still held");
        assert_eq!(since_new_peak + remaining, PEAK_HOLD);
    }

    #[test]
    fn a_present_but_unreadable_linux_source_is_unknown() {
        assert_eq!(bar_state(true, None), BarState::Unknown);
        assert_eq!(bar_state(true, Some(0.4)), BarState::Level(0.4));
        assert_eq!(bar_state(false, None), BarState::Absent);
        assert_eq!(
            bar_state(false, Some(0.4)),
            BarState::Absent,
            "a reading cannot create a source that does not exist"
        );
        assert_eq!(
            bar_state(true, Some(f64::NAN)),
            BarState::Unknown,
            "a non-finite reading is unreadable, not a zero"
        );
    }
}
