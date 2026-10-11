#ifndef ZG01_CODEC_H
#define ZG01_CODEC_H

/*
 * ZG01 mixer value codec.
 *
 * Pure integer code shared by the kernel module and the host unit tests
 * (tests/test-param-codec.c): no kernel headers, no floating point.
 *
 * Wire encoding
 * -------------
 * Mixer writes ride on interface 4 bulk endpoints as Yamaha system
 * exclusive messages (F0 43 10 3E 14 ... F7) packetised as USB-MIDI
 * event packets.  Every SysEx data byte must stay in 0x00-0x7f, so a
 * parameter value travels as big-endian base-128 digits, one digit per
 * 7 bits.  EQ frequencies reach 20 kHz (15 bits) and every other value
 * the driver exposes stays below 128, so three digits are enough:
 *
 *     value = d0*16384 + d1*128 + d2      (each digit <= 0x7f)
 *
 * The first EQ analysis read those three digits as a 24-bit
 * hexadecimal number, which is where the "non-uniform frequency
 * deficits" and the "+32768 above 16 kHz" artefacts came from.  See
 * docs/MIC_CONTROL_PROTOCOL.md and docs/VENDOR_PACKAGE.md.
 *
 * Units
 * -----
 * The user-facing kcontrol units are the ones zgctl documents: 0.1 dB
 * for gains, Hz for frequencies, 0.01 Q for bandwidths.
 *
 *   gain:  device = 180 + user(0.1 dB)   [-18.0 dB -> 0, 0.0 -> 180,
 *                                         +18.0 dB -> 360]
 *   freq:  device = user(Hz)             [20 Hz .. 20000 Hz]
 *   Q:     device = 12 * log2(Q / 0.5)   [0.50 -> 0, 1.00 -> 12,
 *                                         16.0 -> 60]
 *
 * The gain zero point (0 dB = 180), the per-band frequency bounds and
 * the Q bounds come from the vendor parameter model described in
 * docs/VENDOR_PACKAGE.md.
 */

/* --- wire value: three base-128 digits, big-endian ------------------- */

#define ZG01_VALUE_DIGITS 3
#define ZG01_VALUE_MAX    0x1fffffUL    /* 128^3 - 1 */

static inline void zg01_value_encode(unsigned long value,
                                     unsigned char out[ZG01_VALUE_DIGITS])
{
    out[0] = (unsigned char)((value >> 14) & 0x7f);
    out[1] = (unsigned char)((value >> 7) & 0x7f);
    out[2] = (unsigned char)(value & 0x7f);
}

static inline unsigned long zg01_value_decode(const unsigned char *in)
{
    return ((unsigned long)(in[0] & 0x7f) << 14) |
           ((unsigned long)(in[1] & 0x7f) << 7) |
           (unsigned long)(in[2] & 0x7f);
}

/* --- gain: 0.1 dB, 0 dB = 180, range -18.0 .. +18.0 dB --------------- */

#define ZG01_GAIN_USER_MIN (-180L)
#define ZG01_GAIN_USER_MAX (180L)
#define ZG01_GAIN_DEV_ZERO 180UL

static inline unsigned long zg01_gain_user_to_dev(long user_01db)
{
    if (user_01db < ZG01_GAIN_USER_MIN)
        user_01db = ZG01_GAIN_USER_MIN;
    if (user_01db > ZG01_GAIN_USER_MAX)
        user_01db = ZG01_GAIN_USER_MAX;
    return (unsigned long)(user_01db + (long)ZG01_GAIN_DEV_ZERO);
}

static inline long zg01_gain_dev_to_user(unsigned long dev)
{
    return (long)dev - (long)ZG01_GAIN_DEV_ZERO;
}

/* --- frequency: the device takes plain Hz --------------------------- */

#define ZG01_FREQ_DEV_MIN 20UL
#define ZG01_FREQ_DEV_MAX 20000UL

static inline unsigned long zg01_freq_user_to_dev(long hz)
{
    if (hz < (long)ZG01_FREQ_DEV_MIN)
        hz = (long)ZG01_FREQ_DEV_MIN;
    if ((unsigned long)hz > ZG01_FREQ_DEV_MAX)
        hz = (long)ZG01_FREQ_DEV_MAX;
    return (unsigned long)hz;
}

static inline long zg01_freq_dev_to_user(unsigned long dev)
{
    return (long)dev;
}

/* --- Q: 12 units per octave, Q = 0.5 * 2^(dev/12) ------------------- */

#define ZG01_Q_USER_MIN 50L      /* 0.50 */
#define ZG01_Q_USER_MAX 1600L    /* 16.0 */
#define ZG01_Q_DEV_MAX  60UL

/*
 * log2(x) in 1/256 units for x >= 1.  The mantissa is squared bit by
 * bit, so the eight fractional bits are exact to well below one unit
 * (the linear mantissa approximation used earlier was off by up to 22
 * units of Q in the 0.01-Q space).
 */
static inline unsigned int zg01_log2_fp(unsigned long x)
{
    unsigned int e = 0;
    unsigned int frac = 0;
    unsigned long y = x;
    unsigned long m;
    unsigned int i;

    if (x == 0)
        return 0;

    while (y >= 2) {
        y >>= 1;
        e++;
    }
    if (e == 0)
        return 0;               /* x == 1 */

    /* Normalise the mantissa to Q31: value in [1, 2). */
    m = (unsigned long)((unsigned long long)x << (31 - e));

    /* Nine fractional bits, then round to eight. */
    for (i = 0; i < 9; i++) {
        unsigned long long sq = (unsigned long long)m * m;   /* Q62 */

        frac <<= 1;
        if (sq >= (1ULL << 63)) {       /* mantissa^2 >= 2 */
            frac |= 1;
            m = (unsigned long)(sq >> 32);
        } else {
            m = (unsigned long)(sq >> 31);
        }
    }
    frac = (frac + 1) >> 1;

    return e * 256 + frac;
}

/*
 * 2^(v / 256) * 256, the inverse of zg01_log2_fp.  Q31 factors of
 * 2^(1/2^k) are multiplied in for every set bit of the fraction.
 */
static inline unsigned int zg01_exp2_fp(unsigned int v)
{
    static const unsigned long sqrt2_q31[8] = {
        3037000500UL,   /* 2^(1/2)   */
        2553695365UL,   /* 2^(1/4)   */
        2341987720UL,   /* 2^(1/8)   */
        2242581548UL,   /* 2^(1/16)  */
        2194578198UL,   /* 2^(1/32)  */
        2171188663UL,   /* 2^(1/64)  */
        2159253276UL,   /* 2^(1/128) */
        2153311139UL,   /* 2^(1/256) */
    };
    unsigned int i = v >> 8;
    unsigned int f = v & 0xff;
    unsigned long long r = 1ULL << 31;      /* 1.0 in Q31 */
    unsigned int k;

    for (k = 0; k < 8; k++) {
        if (f & (1u << (7 - k)))
            r = (r * (unsigned long long)sqrt2_q31[k]) >> 31;
    }

    /* r>>23 == 2^(f/256) * 256; scaling by 2^i completes the result. */
    return (unsigned int)((r >> 23) << i);
}

static inline unsigned long zg01_q_user_to_dev(long q_01)
{
    unsigned int l, l50;

    if (q_01 < ZG01_Q_USER_MIN)
        q_01 = ZG01_Q_USER_MIN;
    if (q_01 > ZG01_Q_USER_MAX)
        q_01 = ZG01_Q_USER_MAX;

    l = zg01_log2_fp((unsigned long)q_01);
    l50 = zg01_log2_fp((unsigned long)ZG01_Q_USER_MIN);
    return (unsigned long)((12u * (l - l50)) / 256u);
}

/* Smallest user value whose device value is not below dev. */
static inline long zg01_q_user_at_least(unsigned long dev)
{
    long lo = ZG01_Q_USER_MIN, hi = ZG01_Q_USER_MAX, mid;

    while (lo < hi) {
        mid = lo + (hi - lo) / 2;
        if ((long)zg01_q_user_to_dev(mid) < (long)dev)
            lo = mid + 1;
        else
            hi = mid;
    }

    return lo;
}

/* Largest user value whose device value is not above dev. */
static inline long zg01_q_user_at_most(unsigned long dev)
{
    long lo = ZG01_Q_USER_MIN, hi = ZG01_Q_USER_MAX, mid;

    while (lo < hi) {
        mid = lo + (hi - lo + 1) / 2;
        if ((long)zg01_q_user_to_dev(mid) > (long)dev)
            hi = mid - 1;
        else
            lo = mid;
    }

    return lo;
}

/*
 * The user value reported for a device step.  It must convert back to the
 * same step: reporting the physical value (0.5 * 2^(dev/12) rounded to 0.01)
 * did not survive the inverse, so a read followed by a write - exactly what
 * alsactl store/restore does - moved the setting by one step for 46 of the 61
 * device values.  The user scale is finer than the device one, so the values
 * that map to a step form a contiguous interval; the physical value stays the
 * answer whenever it falls inside it (that keeps the nominal anchors: 0.50,
 * 1.00 at step 12, 16.0 at the top), otherwise the closest value in the
 * interval is used.  Cost is two log-scale binary searches, microseconds
 * against a USB write.
 */
static inline long zg01_q_dev_to_user(unsigned long dev)
{
    unsigned int v;
    long lower, upper, physical;

    if (dev > ZG01_Q_DEV_MAX)
        dev = ZG01_Q_DEV_MAX;

    lower = zg01_q_user_at_least(dev);
    if ((long)zg01_q_user_to_dev(lower) != (long)dev)
        return lower;                 /* no user value reports this step */
    upper = zg01_q_user_at_most(dev);

    v = (unsigned int)((dev * 256u) / 12u);
    physical = (long)((50UL * zg01_exp2_fp(v) + 128UL) >> 8);
    if (physical < lower)
        physical = lower;
    if (physical > upper)
        physical = upper;

    return physical;
}

#endif /* ZG01_CODEC_H */
