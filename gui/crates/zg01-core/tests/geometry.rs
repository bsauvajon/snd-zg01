//! The value/fraction geometry the fader and the meters draw with.
//!
//! A fader shows a value as a position on a vertical track; a pointer or a
//! scroll event travels the other way. Both directions have to clamp, snap
//! to the control step and survive degenerate ranges, because a driver may
//! publish a single-valued control.

use zg01_core::geometry::{fraction_to_value, value_to_fraction};

#[test]
fn maps_values_to_fractions() {
    assert_eq!(value_to_fraction(0, 0, 100), 0.0);
    assert_eq!(value_to_fraction(50, 0, 100), 0.5);
    assert_eq!(value_to_fraction(100, 0, 100), 1.0);
    assert_eq!(fraction_to_value(0.5, 0, 127, 1), 64);
}

#[test]
fn clamps_values_outside_the_range() {
    assert_eq!(value_to_fraction(-10, 0, 100), 0.0);
    assert_eq!(value_to_fraction(150, 0, 100), 1.0);
    assert_eq!(fraction_to_value(-0.5, 0, 100, 1), 0);
    assert_eq!(fraction_to_value(1.5, 0, 100, 1), 100);
}

#[test]
fn a_negative_range_maps_from_the_bottom() {
    assert_eq!(value_to_fraction(-180, -180, 180), 0.0);
    assert_eq!(value_to_fraction(0, -180, 180), 0.5);
    assert_eq!(value_to_fraction(180, -180, 180), 1.0);
    assert_eq!(fraction_to_value(0.5, -180, 180, 1), 0);
}

#[test]
fn the_value_is_snapped_to_the_step_grid() {
    // The grid is anchored on the minimum, so a 4-step control on 0..100
    // cannot land on 50.
    assert_eq!(fraction_to_value(0.0, 0, 100, 4), 0);
    assert_eq!(fraction_to_value(0.5, 0, 100, 4), 52);
    assert_eq!(fraction_to_value(0.13, 0, 100, 4), 12);
    assert_eq!(fraction_to_value(0.99, 0, 100, 4), 100);
}

#[test]
fn a_zero_step_is_treated_as_one() {
    assert_eq!(fraction_to_value(0.5, 0, 127, 0), 64);
}

#[test]
fn a_degenerate_range_is_safe() {
    assert_eq!(value_to_fraction(5, 10, 10), 0.0);
    assert_eq!(fraction_to_value(0.5, 10, 10, 1), 10);
    assert_eq!(value_to_fraction(5, 10, 0), 0.0);
    assert_eq!(fraction_to_value(0.5, 10, 0, 1), 10);
}

#[test]
fn a_non_finite_fraction_is_safe() {
    assert_eq!(fraction_to_value(f64::NAN, 0, 100, 1), 0);
    assert_eq!(fraction_to_value(f64::INFINITY, 0, 100, 1), 100);
    assert_eq!(fraction_to_value(f64::NEG_INFINITY, 0, 100, 1), 0);
}

#[test]
fn an_extreme_range_does_not_overflow() {
    assert_eq!(value_to_fraction(i64::MAX, i64::MIN, i64::MAX), 1.0);
    assert_eq!(fraction_to_value(1.0, i64::MIN, i64::MAX, 1), i64::MAX);
    assert_eq!(fraction_to_value(0.0, i64::MIN, i64::MAX, 1), i64::MIN);
}
