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
use crate::i18n::{t, tf};

/// The default height of the drawing area, in logical pixels.
const AREA_HEIGHT: i32 = 150;

/// The width of the card meter, in logical pixels.
const CARD_WIDTH: f64 = 7.0;

/// The width of the Linux meter, in logical pixels.
const LINUX_WIDTH: f64 = 3.0;

/// The gap between two card bars of a bank, in logical pixels.
const CARD_GAP: f64 = 2.0;

/// The gap between the card and Linux bars, in logical pixels.
const METER_GAP: f64 = 4.0;

/// The margin on each side of the bars, in logical pixels.
const SIDE_MARGIN: f64 = 6.0;

/// The space above the bars, in logical pixels.
const TOP_MARGIN: f64 = 4.0;

/// The space below the bars, in logical pixels.
const BOTTOM_MARGIN: f64 = 4.0;

/// The height of the peak-hold line, in logical pixels.
const PEAK_HEIGHT: f64 = 2.0;

/// The height of the clip marker, in logical pixels.
const CLIP_HEIGHT: f64 = 3.0;

/// The level at which the clip marker lights up.
const CLIP_LEVEL: f64 = 0.98;

/// The height of a horizontal channel meter, in logical pixels.
const HORIZONTAL_HEIGHT: i32 = 12;

/// The height of the filled bar inside a horizontal channel meter.
const HORIZONTAL_BAR_HEIGHT: f64 = 8.0;

/// How long the peak-hold line stays after the last higher peak.
const PEAK_HOLD: Duration = Duration::from_secs(1);

/// One meter shown in a strip or beside the mic chain.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeterSpec {
    /// The card label shown under the bars, e.g. `MIC` or `CARD`.
    pub(crate) label: String,
    /// The Linux (PipeWire) label shown beside the card one, e.g. `PW`.
    pub(crate) linux_label: String,
    /// The channel levels of the card meter, normalised to `0.0..=1.0`.
    pub(crate) card: [Option<f64>; 8],
    /// How many of [`MeterSpec::card`] are shown (1 for a strip, 8 for a bank).
    pub(crate) card_count: usize,
    /// The width of one card bar, in logical pixels.
    pub(crate) bar_width: f64,
    /// Whether the meter has a card source at all.
    pub(crate) card_present: bool,
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

impl MeterSpec {
    /// A one-bar meter, used by the Mixage strips and the Micro screen.
    pub(crate) fn mono(
        label: &str,
        card: Option<f64>,
        card_present: bool,
        linux: Option<f64>,
        linux_present: bool,
    ) -> Self {
        let mut card_levels = [None; 8];
        card_levels[0] = card;
        MeterSpec {
            label: label.to_string(),
            linux_label: "PW".to_string(),
            card: card_levels,
            card_count: 1,
            bar_width: CARD_WIDTH,
            card_present,
            linux,
            peak: 0.0,
            linux_present,
        }
    }
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

/// Installs card readings without touching the Linux source.
fn apply_card(spec: &mut MeterSpec, levels: &[Option<f64>]) {
    spec.card = [None; 8];
    for (slot, level) in spec.card.iter_mut().zip(levels.iter().take(8)) {
        *slot = normalise(*level);
    }
    spec.card_count = levels.len().clamp(1, 8);
}

/// Installs a Linux source and its reading without touching the card one.
fn apply_linux(spec: &mut MeterSpec, present: bool, level: Option<f64>) {
    spec.linux = if present { normalise(level) } else { None };
    spec.linux_present = present || spec.linux.is_some();
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
    legend: gtk::Box,
    card_legend: gtk::Box,
    linux_legend: gtk::Box,
    card_label: gtk::Label,
    inner: Rc<MeterInner>,
}

/// Builds one coloured legend entry: a dot and its label.
fn build_legend(text: &str, colour_class: &str) -> (gtk::Box, gtk::Label) {
    let item = gtk::Box::new(Orientation::Horizontal, 4);
    item.set_halign(gtk::Align::Center);

    let dot = gtk::Label::new(Some("●"));
    dot.add_css_class("meter-swatch");
    dot.add_css_class(colour_class);

    let label = gtk::Label::new(Some(text));
    label.add_css_class("meter-legend-label");
    label.set_xalign(0.5);

    item.append(&dot);
    item.append(&label);
    (item, label)
}

impl Meter {
    /// Builds a meter for `spec`.
    pub(crate) fn new(mut spec: MeterSpec) -> Self {
        // A reading proves that the source exists, even when the caller
        // forgot to set `linux_present`.
        for level in &mut spec.card {
            *level = normalise(*level);
        }
        spec.card_count = spec.card_count.clamp(1, 8);
        spec.linux = normalise(spec.linux);
        spec.peak = normalise(Some(spec.peak)).unwrap_or(0.0);
        spec.linux_present |= spec.linux.is_some();

        let root = gtk::Box::new(Orientation::Vertical, 4);
        root.add_css_class("meter-root");
        root.set_halign(gtk::Align::Center);

        let area = gtk::DrawingArea::new();
        let width = meter_width(
            spec.card_present,
            spec.card_count,
            spec.linux_present,
            spec.bar_width,
        );
        area.set_content_width(width);
        area.set_content_height(AREA_HEIGHT);
        area.set_vexpand(true);
        area.set_accessible_role(AccessibleRole::Meter);
        area.add_css_class("meter");
        root.append(&area);

        let legend = gtk::Box::new(Orientation::Horizontal, 8);
        legend.add_css_class("meter-legend");
        legend.set_halign(gtk::Align::Center);
        let (card_legend, card_label) = build_legend(&spec.label, "meter-swatch-card");
        let (linux_legend, _linux_label) = build_legend(&spec.linux_label, "meter-swatch-linux");
        legend.append(&card_legend);
        legend.append(&linux_legend);
        card_legend.set_visible(spec.card_present);
        linux_legend.set_visible(spec.linux_present);
        legend.set_visible(spec.card_present || spec.linux_present);
        root.append(&legend);

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

        Meter {
            root,
            area,
            legend,
            card_legend,
            linux_legend,
            card_label,
            inner,
        }
    }

    /// The widget to put in a layout.
    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Installs card readings; the Linux bar is untouched.
    pub(crate) fn set_card(&self, levels: &[Option<f64>]) {
        self.update(|spec| apply_card(spec, levels));
    }

    /// Installs one card reading; the Linux bar is untouched.
    pub(crate) fn set_card_mono(&self, card: Option<f64>) {
        self.set_card(&[card]);
    }

    /// Changes the card label under the bars.
    pub(crate) fn set_label(&self, text: &str) {
        self.card_label.set_text(text);
    }

    /// Shows or hides the card track, keeping the Linux bar.
    pub(crate) fn set_card_present(&self, present: bool) {
        let (count, linux, bar_width) = {
            let mut spec = self.inner.spec.borrow_mut();
            spec.card_present = present;
            (spec.card_count, spec.linux_present, spec.bar_width)
        };
        self.area
            .set_content_width(meter_width(present, count, linux, bar_width));
        self.refresh_legend();
        self.area.queue_draw();
    }

    /// Installs a Linux source and its reading; the card bar is untouched.
    pub(crate) fn set_linux_source(&self, present: bool, level: Option<f64>) {
        self.update(|spec| apply_linux(spec, present, level));
    }

    /// Shows only the legend entries whose source exists.
    fn refresh_legend(&self) {
        let (card, linux) = {
            let spec = self.inner.spec.borrow();
            (spec.card_present, spec.linux_present)
        };
        self.card_legend.set_visible(card);
        self.linux_legend.set_visible(linux);
        self.legend.set_visible(card || linux);
    }

    /// Applies one side of the meter, then repaints and updates the peak.
    fn update(&self, apply: impl FnOnce(&mut MeterSpec)) {
        {
            let mut spec = self.inner.spec.borrow_mut();
            apply(&mut spec);
        }
        let spec = self.inner.spec.borrow();
        self.area.set_content_width(meter_width(
            spec.card_present,
            spec.card_count,
            spec.linux_present,
            spec.bar_width,
        ));
        refresh_accessible(&self.area, &spec);
        let current = spec
            .card
            .iter()
            .flatten()
            .copied()
            .chain(spec.linux)
            .fold(0.0, f64::max);
        let candidate = spec.peak.max(current);
        drop(spec);
        self.refresh_legend();
        self.area.queue_draw();

        if candidate > self.inner.held_peak.get() {
            self.inner.held_peak.set(candidate);
            self.inner.peak_at.set(Some(Instant::now()));
            schedule_peak_clear(&self.inner, &self.area);
        }
    }
}

/// The peak a fresh meter shows: the spec peak, or the current level.
fn initial_peak(spec: &MeterSpec) -> f64 {
    let peak = normalise(Some(spec.peak)).unwrap_or(0.0);
    let level = spec
        .card
        .iter()
        .flatten()
        .copied()
        .chain(spec.linux)
        .filter_map(|level| normalise(Some(level)))
        .fold(0.0, f64::max);
    peak.max(level)
}

/// The width of a meter drawing area.
fn meter_width(card_present: bool, card_count: usize, has_linux: bool, bar_width: f64) -> i32 {
    let card_count = if card_present {
        card_count.clamp(1, 8)
    } else {
        0
    };
    let bar_width = bar_width.clamp(1.0, 64.0);
    let mut bars = 0.0;
    if card_count > 0 {
        bars += card_count as f64 * bar_width;
        bars += (card_count - 1) as f64 * CARD_GAP;
    }
    if card_count > 0 && has_linux {
        bars += METER_GAP;
    }
    if has_linux {
        bars += LINUX_WIDTH;
    }
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
    let level = spec
        .card
        .iter()
        .flatten()
        .copied()
        .chain(spec.linux)
        .fold(0.0, f64::max);
    let any = spec.card.iter().any(Option::is_some) || spec.linux.is_some();
    let text = if any {
        tf(
            "{percent} %",
            &[("{percent}", &((level * 100.0).round() as i32).to_string())],
        )
    } else {
        t("unknown")
    };
    let description = tf("Meter {name}", &[("{name}", &spec.label)]);

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
    let top = TOP_MARGIN;
    let bottom = (f64::from(height) - BOTTOM_MARGIN).max(top);
    let length = bottom - top;

    let has_card = spec.card_present;
    let card_bars = if has_card {
        spec.card_count.clamp(1, 8)
    } else {
        0
    };
    let card_width = spec.bar_width.clamp(1.0, 64.0);
    let card_total = if card_bars > 0 {
        card_bars as f64 * card_width + (card_bars - 1) as f64 * CARD_GAP
    } else {
        0.0
    };
    let gap = if card_bars > 0 && has_linux {
        METER_GAP
    } else {
        0.0
    };
    let bars = card_total + gap + if has_linux { LINUX_WIDTH } else { 0.0 };
    let start = (f64::from(width) - bars) / 2.0;
    let linux_x = start + card_total + gap;

    // A card channel that exists but cannot be read is drawn dashed; a
    // valid frame at the floor is a solid empty track.
    for index in 0..card_bars {
        let x = start + index as f64 * (card_width + CARD_GAP);
        draw_track(context, x, card_width, top, bottom);
        match bar_state(true, spec.card[index]) {
            BarState::Unknown => draw_unknown(context, x, card_width, top, bottom),
            BarState::Level(level) => {
                draw_fill(context, x, card_width, level, top, bottom, 1.0);
                draw_clip(context, x, card_width, level, top);
            }
            BarState::Absent => {}
        }
    }

    let linux_state = bar_state(has_linux, spec.linux);
    if has_linux {
        draw_track(context, linux_x, LINUX_WIDTH, top, bottom);
        match linux_state {
            BarState::Unknown => draw_unknown(context, linux_x, LINUX_WIDTH, top, bottom),
            BarState::Level(level) => {
                draw_linux_fill(context, linux_x, LINUX_WIDTH, level, top, bottom);
                draw_clip(context, linux_x, LINUX_WIDTH, level, top);
            }
            BarState::Absent => {}
        }
    }

    let held = inner.held_peak.get();
    if held > 0.0 {
        let y = bottom - held * length;
        let right = if has_linux {
            linux_x + LINUX_WIDTH
        } else {
            start + card_total
        };
        rounded_rect(
            context,
            start - 1.0,
            y - PEAK_HEIGHT / 2.0,
            right - start + 2.0,
            PEAK_HEIGHT,
            PEAK_HEIGHT / 2.0,
        );
        theme::TEXT.set_source(context, 0.9);
        let _ = context.fill();
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

/// Draws the Linux bar in the accent colour, so it reads as the `PW` source.
fn draw_linux_fill(
    context: &cairo::Context,
    x: f64,
    width: f64,
    level: f64,
    top: f64,
    bottom: f64,
) {
    let height = (bottom - top).max(0.0);
    let filled = level.clamp(0.0, 1.0) * height;
    if filled <= 0.0 {
        return;
    }
    theme::PW.set_source(context, 1.0);
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

/// A horizontal channel meter, used by the card meter banks.
///
/// The bar grows from left to right and uses the same green → yellow →
/// red scale as the vertical meters. An unreadable channel is a dashed
/// track, never a zero.
pub(crate) struct ChannelMeter {
    area: gtk::DrawingArea,
    label: String,
    level: Rc<Cell<Option<f64>>>,
}

impl ChannelMeter {
    /// Builds a channel meter for `label`, initially at `level`.
    pub(crate) fn new(label: &str, level: Option<f64>) -> Self {
        let level = Rc::new(Cell::new(normalise(level)));
        let area = gtk::DrawingArea::new();
        area.set_content_height(HORIZONTAL_HEIGHT);
        area.set_hexpand(true);
        area.set_valign(gtk::Align::Center);
        area.set_accessible_role(AccessibleRole::Meter);
        area.add_css_class("channel-meter");

        let drawn_level = level.clone();
        area.set_draw_func(move |_, context, width, height| {
            draw_horizontal(context, width, height, drawn_level.get());
        });

        let meter = ChannelMeter {
            area,
            label: label.to_string(),
            level,
        };
        refresh_horizontal_accessible(&meter.area, &meter.label, meter.level.get());
        meter
    }

    /// The widget to put in a row.
    pub(crate) fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// Installs a new reading, or `None` for an unreadable channel.
    pub(crate) fn set_level(&self, level: Option<f64>) {
        let level = normalise(level);
        self.level.set(level);
        refresh_horizontal_accessible(&self.area, &self.label, level);
        self.area.queue_draw();
    }
}

/// Draws one horizontal channel bar.
fn draw_horizontal(context: &cairo::Context, width: i32, height: i32, level: Option<f64>) {
    let width = f64::from(width).max(1.0);
    let height = f64::from(height);
    let bar_height = HORIZONTAL_BAR_HEIGHT.min(height).max(1.0);
    let top = (height - bar_height) / 2.0;
    let radius = bar_height / 2.0;

    rounded_rect(context, 0.0, top, width, bar_height, radius);
    theme::TRACK.set_source(context, 1.0);
    let _ = context.fill();

    match level {
        None => {
            rounded_rect(context, 0.0, top, width, bar_height, radius);
            theme::MUTED.set_source(context, 0.45);
            context.set_line_width(1.0);
            context.set_dash(&[3.0, 3.0], 0.0);
            let _ = context.stroke();
            context.set_dash(&[], 0.0);
        }
        Some(level) => {
            let filled = level.clamp(0.0, 1.0) * width;
            if filled > 0.0 {
                let gradient = horizontal_gradient(width);
                let _ = context.set_source(&gradient);
                rounded_rect(
                    context,
                    0.0,
                    top,
                    filled,
                    bar_height,
                    radius.min(filled / 2.0),
                );
                let _ = context.fill();
            }
            if is_clipping(level) {
                rounded_rect(
                    context,
                    (width - 3.0).max(0.0),
                    top - 1.0,
                    3.0,
                    bar_height + 2.0,
                    1.5,
                );
                theme::ERROR.set_source(context, 1.0);
                let _ = context.fill();
            }
        }
    }
}

/// The green → yellow → red gradient of a horizontal channel meter.
fn horizontal_gradient(width: f64) -> LinearGradient {
    let gradient = LinearGradient::new(0.0, 0.0, width.max(1.0), 0.0);
    for (offset, colour) in [
        (0.0, theme::SUCCESS),
        (0.55, theme::SUCCESS),
        (0.78, theme::WARNING),
        (1.0, theme::ERROR),
    ] {
        let (red, green, blue) = colour.components();
        gradient.add_color_stop_rgba(offset, red, green, blue, 1.0);
    }
    gradient
}

/// Updates the accessible label and value of a horizontal meter.
fn refresh_horizontal_accessible(area: &gtk::DrawingArea, label: &str, level: Option<f64>) {
    let text = match level {
        Some(level) => tf(
            "{percent} %",
            &[("{percent}", &((level * 100.0).round() as i32).to_string())],
        ),
        None => t("unknown"),
    };
    let description = tf("Meter {name}", &[("{name}", label)]);
    let mut properties = vec![
        gtk::accessible::Property::Label(label),
        gtk::accessible::Property::Description(&description),
        gtk::accessible::Property::Orientation(Orientation::Horizontal),
        gtk::accessible::Property::ValueMin(0.0),
        gtk::accessible::Property::ValueMax(100.0),
        gtk::accessible::Property::ValueText(&text),
    ];
    if let Some(level) = level {
        properties.push(gtk::accessible::Property::ValueNow(level * 100.0));
    } else {
        area.reset_property(AccessibleProperty::ValueNow);
    }
    area.update_property(&properties);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        apply_card, apply_linux, bar_state, initial_peak, is_clipping, meter_width, normalise,
        peak_clear_delay, BarState, MeterSpec, CARD_WIDTH, PEAK_HOLD,
    };

    #[test]
    fn a_bank_width_counts_every_card_bar_and_the_pw_bar() {
        assert_eq!(meter_width(true, 1, false, CARD_WIDTH), 19);
        assert_eq!(meter_width(true, 8, false, CARD_WIDTH), 19 + 7 * 9);
        assert_eq!(meter_width(true, 8, true, CARD_WIDTH), 26 + 7 * 9);
        assert_eq!(meter_width(false, 0, true, CARD_WIDTH), 15);
        assert_eq!(meter_width(false, 0, false, CARD_WIDTH), 12);
    }

    #[test]
    fn a_wide_card_bar_widens_the_meter() {
        assert_eq!(meter_width(true, 1, false, 22.0), 34);
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
        let spec = MeterSpec::mono("MIC", Some(0.5), true, None, false);
        assert_eq!(spec.label, "MIC");
        assert_eq!(spec.linux_label, "PW");
        assert_eq!(spec.card[0], Some(0.5));
        assert_eq!(spec.linux, None);
    }

    #[test]
    fn a_fresh_meter_holds_the_peak_of_its_spec() {
        let mut spec = MeterSpec::mono("MIC", Some(0.4), true, None, false);
        spec.peak = 0.8;
        assert_eq!(initial_peak(&spec), 0.8);
        let mut spec = MeterSpec::mono("MIC", Some(0.9), true, None, false);
        spec.peak = 0.2;
        assert_eq!(initial_peak(&spec), 0.9);
    }

    #[test]
    fn a_card_update_keeps_the_linux_bar() {
        let mut spec = MeterSpec::mono("MIC", Some(0.4), true, Some(0.2), true);
        apply_card(&mut spec, &[Some(0.6)]);
        assert_eq!(spec.card[0], Some(0.6));
        assert_eq!(spec.linux, Some(0.2));
        assert!(spec.linux_present);
    }

    #[test]
    fn eight_card_channels_keep_every_reading() {
        let mut spec = MeterSpec::mono("PC", None, true, None, false);
        apply_card(
            &mut spec,
            &[Some(0.1), Some(0.2), None, None, None, None, None, None],
        );
        assert_eq!(spec.card_count, 8);
        assert_eq!(spec.card[0], Some(0.1));
        assert_eq!(spec.card[1], Some(0.2));
        assert_eq!(spec.card[2], None);
    }

    #[test]
    fn a_linux_update_keeps_the_card_bar_and_controls_presence() {
        let mut spec = MeterSpec::mono("MIC", Some(0.4), true, None, false);
        apply_linux(&mut spec, true, Some(0.2));
        assert_eq!(spec.card[0], Some(0.4));
        assert_eq!(spec.linux, Some(0.2));
        assert!(spec.linux_present);

        apply_linux(&mut spec, false, None);
        assert_eq!(spec.card[0], Some(0.4));
        assert_eq!(spec.linux, None);
        assert!(!spec.linux_present);
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
