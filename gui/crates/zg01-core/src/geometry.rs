//! The geometry shared by the custom widgets.
//!
//! A fader maps a control value to a position on its vertical track and
//! maps a pointer position back to a value; a meter maps a normalised
//! level to a height. Both directions of the fader are pure arithmetic, so
//! they live here, away from GTK, and are tested without a display.

/// The position of `value` on a track that spans `min`..=`max`.
///
/// The result is in `0.0..=1.0`; a value outside the range is clamped. A
/// degenerate range (`max <= min`) has no position and returns `0.0`.
pub fn value_to_fraction(value: i64, min: i64, max: i64) -> f64 {
    if max <= min {
        return 0.0;
    }
    let value = value.clamp(min, max);
    (value as f64 - min as f64) / (max as f64 - min as f64)
}

/// The control value at `fraction` on a track that spans `min`..=`max`.
///
/// The fraction is clamped to `0.0..=1.0`, a non-finite fraction is
/// treated as the bottom of the track, and the result is snapped to the
/// step grid anchored on `min` and clamped to `max`. A step of zero or
/// below is treated as one: a control with no published step still moves
/// one value at a time.
pub fn fraction_to_value(fraction: f64, min: i64, max: i64, step: i64) -> i64 {
    if max <= min {
        return min;
    }
    let fraction = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    };
    let step = step.max(1);
    let raw = min as f64 + fraction * (max as f64 - min as f64);
    // Keep the arithmetic in f64: an extreme i64 range must not overflow
    // while it is snapped to the step grid.
    let steps = ((raw - min as f64) / step as f64).round();
    let value = min as f64 + steps * step as f64;
    value.clamp(min as f64, max as f64) as i64
}

/// The value on the step grid closest to `value`.
///
/// The grid is anchored on `min`, like [`fraction_to_value`]: an editor
/// that emits `snap_to_step(raw, min, max, step)` only ever writes
/// `min + n * step`. A step of zero or below is treated as one, and the
/// result is clamped to the range.
pub fn snap_to_step(value: i64, min: i64, max: i64, step: i64) -> i64 {
    if max <= min {
        return min;
    }
    let step = step.max(1);
    let value = value.clamp(min, max);
    let steps = ((value as f64 - min as f64) / step as f64).round();
    let snapped = min as f64 + steps * step as f64;
    snapped.clamp(min as f64, max as f64) as i64
}

#[cfg(test)]
mod tests {
    use super::{fraction_to_value, snap_to_step, value_to_fraction};

    #[test]
    fn a_value_maps_to_its_place_on_the_track() {
        assert_eq!(value_to_fraction(0, 0, 127), 0.0);
        assert_eq!(value_to_fraction(127, 0, 127), 1.0);
        assert!((value_to_fraction(64, 0, 127) - 64.0 / 127.0).abs() < 1e-12);
    }

    #[test]
    fn the_round_trip_keeps_a_value_on_its_step() {
        for value in [0, 12, 52, 100] {
            let fraction = value_to_fraction(value, 0, 100);
            assert_eq!(fraction_to_value(fraction, 0, 100, 4), value);
        }
    }

    #[test]
    fn a_scale_or_number_entry_snaps_an_off_grid_value_to_the_grid() {
        assert_eq!(snap_to_step(50, 0, 100, 5), 50);
        assert_eq!(snap_to_step(63, 0, 100, 5), 65);
        assert_eq!(snap_to_step(62, 0, 100, 5), 60);
        assert_eq!(snap_to_step(100, 0, 100, 5), 100);
        assert_eq!(snap_to_step(101, 0, 100, 5), 100);
        assert_eq!(snap_to_step(-1, 0, 100, 5), 0);
        assert_eq!(snap_to_step(-185, -180, 180, 1), -180);
        assert_eq!(snap_to_step(42, 0, 100, 0), 42, "a zero step is one");
        assert_eq!(
            snap_to_step(-173, -180, 180, 10),
            -170,
            "the grid is anchored on the minimum, not on zero"
        );
        assert_eq!(
            snap_to_step(7, 0, 2_097_151, 1),
            7,
            "a one-step register is already on its grid"
        );
    }
}
