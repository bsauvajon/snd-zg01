//! Conversions between the raw device values and the user-facing units.
//!
//! The driver exposes its parameters in the units `zgctl` documents: the
//! gains in 0.1 dB, the frequencies in Hz, the Q in 0.01 and everything
//! else in raw steps. The raw value of a parameter is on another scale
//! again, and the conversions between the two are the ones of the driver:
//! they live in `src/zg01_codec.h` and are ported here without change, so
//! the GUI never carries a second conversion table.
//!
//! | Conversion | Device value | User value |
//! |---|---|---|
//! | [`Conv::Id`] | the value itself | the value itself |
//! | [`Conv::Gain`] | `user + 180` (`0 dB` is `180`) | `device - 180` |
//! | [`Conv::Freq`] | the value in Hz | the value in Hz |
//! | [`Conv::Q`] | `12 * log2(Q / 0.5)` | `0.5 * 2^(device / 12)` |
//!
//! The gain range is -18.0 dB .. +18.0 dB, the frequency range 20 Hz ..
//! 20 kHz and the Q range 0.50 .. 16.0 (the device steps 0 .. 60).
//!
//! The Q conversion is the delicate one: the device holds 12 steps per
//! octave, so several user values map to one device step. The driver
//! reports the *physical* value of a step when it converts back to the
//! same step, and the closest value of the step otherwise, so that a read
//! followed by a write leaves the setting where it was. The port keeps
//! that behaviour, including the two binary searches it costs.

use serde::Deserialize;

/// The conversion a control applies to its raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Conv {
    /// The raw value is the user value: switches, the 0..100 levels, the
    /// shapes.
    Id,
    /// A gain, in 0.1 dB.
    Gain,
    /// A frequency, in Hz.
    Freq,
    /// A Q, in 0.01.
    Q,
}

/// The user gain range: -18.0 dB .. +18.0 dB.
const GAIN_USER_MIN: i64 = -180;
/// The user gain range: -18.0 dB .. +18.0 dB.
const GAIN_USER_MAX: i64 = 180;
/// The device gain of 0 dB.
const GAIN_DEV_ZERO: i64 = 180;

/// The user frequency range: 20 Hz .. 20 kHz.
const FREQ_DEV_MIN: i64 = 20;
/// The user frequency range: 20 Hz .. 20 kHz.
const FREQ_DEV_MAX: i64 = 20000;

/// The user Q range: 0.50 .. 16.0.
const Q_USER_MIN: i64 = 50;
/// The user Q range: 0.50 .. 16.0.
const Q_USER_MAX: i64 = 1600;
/// The device Q range: 0 .. 60.
const Q_DEV_MAX: i64 = 60;

/// The user value of a raw device value.
///
/// Port of `zg01_dev_to_user` of `src/zg01_control.c`, which calls the
/// codec of `src/zg01_codec.h`.
pub fn raw_to_user(conv: Conv, raw: i64) -> i64 {
    match conv {
        Conv::Id => raw,
        Conv::Gain => gain_dev_to_user(raw),
        Conv::Freq => freq_dev_to_user(raw),
        Conv::Q => q_dev_to_user(raw),
    }
}

/// The raw device value of a user value.
///
/// Port of `zg01_user_to_dev` of `src/zg01_control.c`, which calls the
/// codec of `src/zg01_codec.h`.
pub fn user_to_raw(conv: Conv, user: i64) -> i64 {
    match conv {
        Conv::Id => user,
        Conv::Gain => gain_user_to_dev(user),
        Conv::Freq => freq_user_to_dev(user),
        Conv::Q => q_user_to_dev(user),
    }
}

/// The user gain of a raw gain value.
///
/// Port of `zg01_gain_dev_to_user`.
fn gain_dev_to_user(dev: i64) -> i64 {
    dev - GAIN_DEV_ZERO
}

/// The raw gain value of a user gain.
///
/// Port of `zg01_gain_user_to_dev`: the user gain is clamped to the
/// published range.
fn gain_user_to_dev(user: i64) -> i64 {
    user.clamp(GAIN_USER_MIN, GAIN_USER_MAX) + GAIN_DEV_ZERO
}

/// The user frequency of a raw frequency.
///
/// Port of `zg01_freq_dev_to_user`.
fn freq_dev_to_user(dev: i64) -> i64 {
    dev
}

/// The raw frequency of a user frequency.
///
/// Port of `zg01_freq_user_to_dev`: the frequency is clamped to the
/// published range.
fn freq_user_to_dev(hz: i64) -> i64 {
    hz.clamp(FREQ_DEV_MIN, FREQ_DEV_MAX)
}

/// The raw Q value of a user Q.
///
/// Port of `zg01_q_user_to_dev`: the user Q is clamped to the published
/// range and the device value is 12 steps per octave above the 0.5 floor.
fn q_user_to_dev(q01: i64) -> i64 {
    let q01 = q01.clamp(Q_USER_MIN, Q_USER_MAX) as u64;
    let l = log2_fp(q01);
    let l50 = log2_fp(Q_USER_MIN as u64);
    i64::from((12 * (l - l50)) / 256)
}

/// The user Q of a raw Q value.
///
/// Port of `zg01_q_dev_to_user`.
fn q_dev_to_user(dev: i64) -> i64 {
    let dev = dev.clamp(0, Q_DEV_MAX);

    // The user values that map to a device step form a contiguous
    // interval: the user scale is finer than the device one.
    let lower = q_user_at_least(dev);
    if q_user_to_dev(lower) != dev {
        // No user value reports this step.
        return lower;
    }
    let upper = q_user_at_most(dev);

    // The physical value of the step, when it falls inside the interval.
    let v = ((dev * 256) / 12) as u32;
    let mut physical = (((u64::from(50u32) * u64::from(exp2_fp(v))) + 128) >> 8) as i64;
    physical = physical.clamp(lower, upper);

    physical
}

/// The smallest user Q whose device value is not below `dev`.
///
/// Port of `zg01_q_user_at_least`.
fn q_user_at_least(dev: i64) -> i64 {
    let mut lo = Q_USER_MIN;
    let mut hi = Q_USER_MAX;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if q_user_to_dev(mid) < dev {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// The largest user Q whose device value is not above `dev`.
///
/// Port of `zg01_q_user_at_most`.
fn q_user_at_most(dev: i64) -> i64 {
    let mut lo = Q_USER_MIN;
    let mut hi = Q_USER_MAX;
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if q_user_to_dev(mid) > dev {
            hi = mid - 1;
        } else {
            lo = mid;
        }
    }
    lo
}

/// `log2(x)` in 1/256 units, for `x >= 1`.
///
/// Port of `zg01_log2_fp`: the mantissa is squared bit by bit, so the
/// eight fractional bits are exact to well below one unit.
fn log2_fp(x: u64) -> u32 {
    if x == 0 {
        return 0;
    }

    let mut e = 0u32;
    let mut y = x;
    while y >= 2 {
        y >>= 1;
        e += 1;
    }
    if e == 0 {
        // x == 1
        return 0;
    }

    // Normalise the mantissa to Q31: the value is in [1, 2).
    let mut m = x << (31 - e);

    // Nine fractional bits, then round to eight.
    let mut frac = 0u32;
    for _ in 0..9 {
        let sq = m * m; // Q62
        frac <<= 1;
        if sq >= (1u64 << 63) {
            // The mantissa squared is 2 or more.
            frac |= 1;
            m = sq >> 32;
        } else {
            m = sq >> 31;
        }
    }
    frac = (frac + 1) >> 1;

    e * 256 + frac
}

/// `2^(v / 256) * 256`, the inverse of [`log2_fp`].
///
/// Port of `zg01_exp2_fp`: the Q31 factors of `2^(1/2^k)` are multiplied
/// in for every set bit of the fraction.
fn exp2_fp(v: u32) -> u32 {
    /// The Q31 factors of `2^(1/2^k)`.
    const SQRT2_Q31: [u64; 8] = [
        3_037_000_500,
        2_553_695_365,
        2_341_987_720,
        2_242_581_548,
        2_194_578_198,
        2_171_188_663,
        2_159_253_276,
        2_153_311_139,
    ];

    let i = v >> 8;
    let f = v & 0xff;
    let mut r: u64 = 1u64 << 31; // 1.0 in Q31
    for (k, factor) in SQRT2_Q31.iter().enumerate() {
        if f & (1u32 << (7 - k as u32)) != 0 {
            r = (r * factor) >> 31;
        }
    }

    // `r >> 23` is `2^(f/256) * 256`; scaling by `2^i` completes the
    // result. The C code truncates the result to 32 bits, which the port
    // does by casting.
    let shifted = if i >= 32 { 0 } else { (r >> 23) << i };
    shifted as u32
}

#[cfg(test)]
mod tests {
    use super::{raw_to_user, user_to_raw, Conv, Q_DEV_MAX};

    #[test]
    fn the_gain_scale_is_the_codec_one() {
        assert_eq!(raw_to_user(Conv::Gain, 180), 0);
        assert_eq!(raw_to_user(Conv::Gain, 240), 60);
        assert_eq!(raw_to_user(Conv::Gain, 0), -180);
        assert_eq!(raw_to_user(Conv::Gain, 360), 180);
        assert_eq!(user_to_raw(Conv::Gain, 0), 180);
        assert_eq!(user_to_raw(Conv::Gain, 60), 240);
        assert_eq!(user_to_raw(Conv::Gain, -180), 0);
        assert_eq!(user_to_raw(Conv::Gain, 180), 360);
    }

    #[test]
    fn the_frequency_is_its_own_value() {
        assert_eq!(raw_to_user(Conv::Freq, 20), 20);
        assert_eq!(raw_to_user(Conv::Freq, 1000), 1000);
        assert_eq!(raw_to_user(Conv::Freq, 20000), 20000);
        assert_eq!(user_to_raw(Conv::Freq, 20), 20);
        assert_eq!(user_to_raw(Conv::Freq, 1000), 1000);
        assert_eq!(user_to_raw(Conv::Freq, 20000), 20000);
    }

    #[test]
    fn the_q_scale_is_the_codec_one() {
        assert_eq!(raw_to_user(Conv::Q, 12), 100);
        assert_eq!(user_to_raw(Conv::Q, 100), 12);
        assert_eq!(user_to_raw(Conv::Q, 1600), 60);
        assert_eq!(raw_to_user(Conv::Q, 0), 50);
        assert_eq!(user_to_raw(Conv::Q, 50), 0);
    }

    #[test]
    fn a_q_value_converts_back_to_its_step() {
        // The driver reports a value that converts back to the same device
        // step, so a read followed by a write leaves the setting where it
        // was.
        for dev in 0..=Q_DEV_MAX {
            let user = raw_to_user(Conv::Q, dev);
            assert_eq!(
                user_to_raw(Conv::Q, user),
                dev,
                "the Q {user} of the device step {dev} does not convert back"
            );
        }
    }

    #[test]
    fn the_identity_conversion_changes_nothing() {
        assert_eq!(raw_to_user(Conv::Id, 42), 42);
        assert_eq!(user_to_raw(Conv::Id, 42), 42);
    }

    #[test]
    fn an_out_of_range_value_stays_sane() {
        // The driver range-checks the user value before it converts; the
        // conversion clamps it instead of producing a wild raw value.
        assert_eq!(user_to_raw(Conv::Gain, 1000), 360);
        assert_eq!(user_to_raw(Conv::Gain, -1000), 0);
        assert_eq!(user_to_raw(Conv::Freq, 0), 20);
        assert_eq!(user_to_raw(Conv::Freq, 100000), 20000);
        assert_eq!(user_to_raw(Conv::Q, 0), 0);
        assert_eq!(user_to_raw(Conv::Q, 100000), 60);
    }
}
