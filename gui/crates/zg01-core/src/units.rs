//! Conversions between the raw device values and the user-facing units.
//!
//! The driver exposes its parameters in the units `zgctl` documents: the
//! gains in 0.1 dB, the frequencies in Hz, the Q in 0.01 and everything
//! else in raw steps. The raw value of a parameter is on another scale
//! again, and the conversions between the two are the ones of the driver:
//!
//! * the gains are scaled over the device range (`zg01_user_to_dev` and
//!   `zg01_dev_to_user`);
//! * the frequencies go through the measured table of
//!   `docs/MIC_CONTROL_PROTOCOL.md` (`zg01_freq_to_raw` and
//!   `zg01_raw_to_freq`);
//! * the Q is 12 steps per octave around a 0.5 floor
//!   (`zg01_q100_to_raw` and `zg01_raw_to_q100`).
//!
//! The formulas are copied from the driver without change, so the GUI
//! never carries a second conversion table.
//!
//! # Scale of the ported formulas
//!
//! The port follows the revision of `src/zg01_control.c` the GUI plan was
//! written against, which reads the mixer values on the older scale: the
//! gain zero point is the raw value 308 and the frequencies go through the
//! table below. The driver of this tree encodes the values as base-128
//! digits instead (`src/zg01_codec.h`, since commit `3faadcb`), where the
//! gain zero point is 180 and a frequency is its own value in Hz. The
//! ALSA elements of the current driver already expose the user units, so
//! the GUI does not need these conversions to read or write a control;
//! they matter for the raw values a preset carries. See the task 2
//! report before using them on a raw value.

/// The conversion a control applies to its raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// The measured frequency table of `docs/MIC_CONTROL_PROTOCOL.md`.
///
/// The pairs are `(hz, raw)`.
const FREQ_TABLE: &[(u32, u32)] = &[
    (20, 20),
    (50, 50),
    (100, 100),
    (200, 328),
    (500, 884),
    (1000, 1896),
    (2000, 3920),
    (5000, 9992),
    (10000, 19984),
    (15000, 29976),
    (16383, 32766),
    (16384, 65536),
    (17000, 66664),
    (20000, 72736),
];

/// The user gain range: -18.0 dB .. +18.0 dB.
const GAIN_USER_MIN: i64 = -180;
/// The user gain range: -18.0 dB .. +18.0 dB.
const GAIN_USER_MAX: i64 = 180;
/// The user Q range: 0.50 .. 16.0.
const Q_USER_MIN: i64 = 50;
/// The user Q range: 0.50 .. 16.0.
const Q_USER_MAX: i64 = 1600;

/// The user value of a raw device value.
///
/// Port of `zg01_dev_to_user`.
pub fn raw_to_user(conv: Conv, raw: i64) -> i64 {
    match conv {
        Conv::Id => raw,
        Conv::Gain => gain_dev_to_user(raw),
        Conv::Freq => raw_to_freq(raw),
        Conv::Q => raw_to_q100(raw),
    }
}

/// The raw device value of a user value.
///
/// Port of `zg01_user_to_dev`.
pub fn user_to_raw(conv: Conv, user: i64) -> i64 {
    match conv {
        Conv::Id => user,
        Conv::Gain => gain_user_to_dev(user),
        Conv::Freq => freq_to_raw(user),
        Conv::Q => q100_to_raw(user),
    }
}

/// The user gain of a raw gain value.
///
/// Port of the `CONV_GAIN` case of `zg01_dev_to_user`.
fn gain_dev_to_user(raw: i64) -> i64 {
    let raw = raw.max(0);
    (180 * raw + 154) / 308 - 180
}

/// The raw gain value of a user gain.
///
/// Port of the `CONV_GAIN` case of `zg01_user_to_dev`.
///
/// The user gain is clamped to the published range, as the driver does
/// before it converts, so an out-of-range input cannot produce a raw
/// value that means nothing.
fn gain_user_to_dev(user: i64) -> i64 {
    let user = user.clamp(GAIN_USER_MIN, GAIN_USER_MAX);
    (308 * (user + 180) + 90) / 180
}

/// The user frequency of a raw frequency.
///
/// Port of `zg01_raw_to_freq`: the table is walked and the value is
/// interpolated between the two surrounding entries.
fn raw_to_freq(raw: i64) -> i64 {
    let raw = raw.max(0) as u32;
    let (first_hz, first_raw) = FREQ_TABLE[0];
    if raw <= first_raw {
        return i64::from(first_hz);
    }

    for window in FREQ_TABLE.windows(2) {
        let (h0, r0) = window[0];
        let (h1, r1) = window[1];
        if raw <= r1 {
            let span = u64::from(r1 - r0);
            let step = u64::from(raw - r0) * u64::from(h1 - h0) / span;
            return i64::from(h0) + step as i64;
        }
    }

    i64::from(FREQ_TABLE[FREQ_TABLE.len() - 1].0)
}

/// The raw frequency of a user frequency.
///
/// Port of `zg01_freq_to_raw`: the table is walked and the value is
/// interpolated between the two surrounding entries.
fn freq_to_raw(hz: i64) -> i64 {
    let (first_hz, first_raw) = FREQ_TABLE[0];
    if hz <= i64::from(first_hz) {
        return i64::from(first_raw);
    }

    for window in FREQ_TABLE.windows(2) {
        let (h0, r0) = window[0];
        let (h1, r1) = window[1];
        if hz <= i64::from(h1) {
            let span = u64::from(h1 - h0);
            let step = (hz - i64::from(h0)) as u64 * u64::from(r1 - r0) / span;
            return i64::from(r0) + step as i64;
        }
    }

    i64::from(FREQ_TABLE[FREQ_TABLE.len() - 1].1)
}

/// The user Q of a raw Q value.
///
/// Port of `zg01_raw_to_q100`.
fn raw_to_q100(raw: i64) -> i64 {
    let v = (raw.max(0) as u64 * 256) / 12;
    let v = u32::try_from(v).unwrap_or(u32::MAX);
    ((u64::from(50u32) * u64::from(exp2_fp(v))) >> 8) as i64
}

/// The raw Q value of a user Q.
///
/// Port of `zg01_q100_to_raw`.
///
/// The user Q is clamped to the published range, as the driver does before
/// it converts.
fn q100_to_raw(q100: i64) -> i64 {
    let q100 = q100.clamp(Q_USER_MIN, Q_USER_MAX) as u32;
    let l = log2_fp(q100);
    let l50 = log2_fp(Q_USER_MIN as u32);
    i64::from((12 * l.saturating_sub(l50)) / 256)
}

/// `log2(x)` in 1/256 units, for `x >= 1`.
///
/// Port of `zg01_log2_fp`. `x == 0` has no logarithm and returns 0, where
/// the C code would be undefined.
fn log2_fp(x: u32) -> u32 {
    if x == 0 {
        return 0;
    }
    let e = 31 - x.leading_zeros();
    let mantissa = u64::from(x) << (31 - e);
    let frac = (((mantissa - 0x8000_0000) << 8) >> 31) as u32;
    e * 256 + frac
}

/// `2^(v / 256) * 256`.
///
/// Port of `zg01_exp2_fp`. The C code shifts a 32-bit value by an
/// arbitrary amount, which is undefined above 31; the shift saturates
/// here instead.
fn exp2_fp(v: u32) -> u32 {
    let i = v >> 8;
    let f = v & 0xff;
    let base = 256 + (f * 177) / 256;
    if i >= 32 {
        return u32::MAX;
    }
    let shifted = u64::from(base) << i;
    shifted.min(u64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::{raw_to_user, user_to_raw, Conv};

    #[test]
    fn the_gain_scale_is_the_driver_one() {
        assert_eq!(raw_to_user(Conv::Gain, 308), 0);
        assert_eq!(user_to_raw(Conv::Gain, 0), 308);
        assert_eq!(raw_to_user(Conv::Gain, 0), -180);
        assert_eq!(raw_to_user(Conv::Gain, 616), 180);
        assert_eq!(user_to_raw(Conv::Gain, 180), 616);
    }

    #[test]
    fn the_frequency_table_is_the_driver_one() {
        assert_eq!(raw_to_user(Conv::Freq, 20), 20);
        assert_eq!(raw_to_user(Conv::Freq, 1896), 1000);
        assert_eq!(raw_to_user(Conv::Freq, 72736), 20000);
        assert_eq!(user_to_raw(Conv::Freq, 1000), 1896);
        assert_eq!(user_to_raw(Conv::Freq, 20000), 72736);
    }

    #[test]
    fn the_q_scale_is_the_driver_one() {
        assert_eq!(raw_to_user(Conv::Q, 12), 100);
        assert_eq!(user_to_raw(Conv::Q, 100), 12);
        assert_eq!(raw_to_user(Conv::Q, 0), 50);
        assert_eq!(user_to_raw(Conv::Q, 50), 0);
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
        assert_eq!(user_to_raw(Conv::Gain, 1000), 616);
        assert_eq!(user_to_raw(Conv::Gain, -1000), 0);
        assert_eq!(user_to_raw(Conv::Q, 0), 0);
        assert_eq!(user_to_raw(Conv::Q, 100000), 60);
    }
}
