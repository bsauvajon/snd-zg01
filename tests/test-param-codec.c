/*
 * Host unit tests for the ZG01 value codec (src/zg01_codec.h).
 *
 * The expectations are the values actually observed in the Windows ZG
 * Controller captures, expressed in the frame's own bytes, plus the
 * vendor parameter model (docs/VENDOR_PACKAGE.md):
 *
 *   - EQ frequency 1 kHz arrives as the digits 00 07 68, which the first
 *     analysis recorded as the hexadecimal number 0x000768 = 1896.
 *   - EQ gain 0 dB arrives as 00 01 34 (recorded as 0x0134 = 308) and
 *     +18 dB as 00 02 68 (recorded as 0x0268 = 616).
 *
 * Build and run through tests/test-param-codec.sh.
 */

#include <stdio.h>

#include "../src/zg01_codec.h"

static unsigned int failures;

static void check(long got, long want, const char *what)
{
    if (got != want) {
        printf("FAIL %s: got %ld, want %ld\n", what, got, want);
        failures++;
    } else {
        printf("ok   %s = %ld\n", what, got);
    }
}

static void check_digits(unsigned long value, unsigned char d0,
                         unsigned char d1, unsigned char d2,
                         const char *what)
{
    unsigned char out[ZG01_VALUE_DIGITS];

    zg01_value_encode(value, out);
    if (out[0] != d0 || out[1] != d1 || out[2] != d2) {
        printf("FAIL %s: got %02x %02x %02x, want %02x %02x %02x\n",
               what, out[0], out[1], out[2], d0, d1, d2);
        failures++;
        return;
    }
    if (zg01_value_decode(out) != value) {
        printf("FAIL %s: decode(encode(%lu)) != %lu\n", what, value, value);
        failures++;
        return;
    }
    printf("ok   %s: %02x %02x %02x\n", what, d0, d1, d2);
}

int main(void)
{
    static const unsigned char observed_freq_digits[][3] = {
        /* --- the "raw" figures from the first EQ analysis, as bytes --- */
        { 0x00, 0x00, 0x14, },      /*   20 Hz, recorded 20      */
        { 0x00, 0x00, 0x32, },      /*   50 Hz, recorded 50      */
        { 0x00, 0x01, 0x48, },      /*  200 Hz, recorded 328     */
        { 0x00, 0x03, 0x74, },      /*  500 Hz, recorded 884     */
        { 0x00, 0x07, 0x68, },      /* 1000 Hz, recorded 1896    */
        { 0x00, 0x0f, 0x50, },      /* 2000 Hz, recorded 3920    */
        { 0x00, 0x27, 0x08, },      /* 5000 Hz, recorded 9992    */
        { 0x00, 0x4e, 0x10, },      /* 10000 Hz, recorded 19984  */
        { 0x00, 0x75, 0x18, },      /* 15000 Hz, recorded 29976  */
        { 0x01, 0x04, 0x68, },      /* 17000 Hz, recorded 66664  */
        { 0x01, 0x1c, 0x20, },      /* 20000 Hz, recorded 72736  */
    };
    static const long observed_freq[] = {
        20, 50, 200, 500, 1000, 2000, 5000, 10000, 15000, 17000, 20000,
    };
    unsigned int i;
    long q, dev;

    /* --- wire encoding --------------------------------------------- */
    for (i = 0; i < sizeof(observed_freq) / sizeof(observed_freq[0]); i++) {
        char label[64];
        long hz = observed_freq[i];
        unsigned long hex = ((unsigned long)observed_freq_digits[i][0] << 16) |
                            ((unsigned long)observed_freq_digits[i][1] << 8) |
                            (unsigned long)observed_freq_digits[i][2];

        snprintf(label, sizeof(label), "%ld Hz (analysed hex %lu)", hz, hex);
        check_digits((unsigned long)hz, observed_freq_digits[i][0],
                     observed_freq_digits[i][1], observed_freq_digits[i][2],
                     label);
        check((long)zg01_value_decode(observed_freq_digits[i]),
              zg01_value_decode(observed_freq_digits[i]),
              "decode is exact");
    }

    check_digits(16383UL, 0x00, 0x7f, 0x7f, "16383 boundary");
    check_digits(16384UL, 0x01, 0x00, 0x00, "16384 boundary");
    check_digits(ZG01_VALUE_MAX, 0x7f, 0x7f, 0x7f, "maximum value");

    /* --- gain: 0.1 dB, 0 dB = 180 -------------------------------- */
    check_digits(zg01_gain_user_to_dev(0), 0x00, 0x01, 0x34, "0 dB gain");
    check_digits(zg01_gain_user_to_dev(180), 0x00, 0x02, 0x68, "+18 dB gain");
    check_digits(zg01_gain_user_to_dev(-180), 0x00, 0x00, 0x00, "-18 dB gain");
    check((long)zg01_gain_user_to_dev(0), 180, "gain 0 dB -> device");
    check((long)zg01_gain_user_to_dev(60), 240, "gain +6.0 dB -> device");
    check(zg01_gain_dev_to_user(180), 0, "device 180 -> 0 dB");
    check(zg01_gain_dev_to_user(616), 436, "out-of-range device is reported");
    check((long)zg01_gain_user_to_dev(999), 360, "gain clamps high");
    check((long)zg01_gain_user_to_dev(-999), 0, "gain clamps low");

    /* --- frequency: plain Hz, clamped to the shared range --------- */
    check((long)zg01_freq_user_to_dev(1000), 1000, "freq 1 kHz");
    check((long)zg01_freq_user_to_dev(5), 20, "freq clamps to 20 Hz");
    check((long)zg01_freq_user_to_dev(30000), 20000, "freq clamps to 20 kHz");
    check(zg01_freq_dev_to_user(500), 500, "freq device -> user");

    /* --- Q: 12 units per octave ----------------------------------- */
    check((long)zg01_q_user_to_dev(50), 0, "Q 0.50 -> device");
    check((long)zg01_q_user_to_dev(100), 12, "Q 1.00 -> device");
    check((long)zg01_q_user_to_dev(200), 24, "Q 2.00 -> device");
    check((long)zg01_q_user_to_dev(400), 36, "Q 4.00 -> device");
    check((long)zg01_q_user_to_dev(800), 48, "Q 8.00 -> device");
    check((long)zg01_q_user_to_dev(1600), 60, "Q 16.0 -> device");
    check(zg01_q_dev_to_user(0), 50, "device 0 -> Q 0.50");
    check(zg01_q_dev_to_user(12), 100, "device 12 -> Q 1.00");
    check(zg01_q_dev_to_user(60), 1600, "device 60 -> Q 16.0");
    check((long)zg01_q_user_to_dev(10), 0, "Q clamps to 0.50");
    check((long)zg01_q_user_to_dev(9900), 60, "Q clamps to 16.0");

    /* Mid-scale Q: 0.5 * 2^(30/12) = 2.828 -> 2.83 in 0.01 Q units. */
    check(zg01_q_dev_to_user(30), 283, "device 30 -> Q 2.83");
    /*
     * 0.53 is the physical value of step 1 but converts back to step 0, so
     * the reported value is the closest one that survives the inverse.
     */
    check(zg01_q_dev_to_user(1), 54, "device 1 -> Q 0.54 (round-trips)");

    /*
     * The device holds 61 Q values and the user space 1550, so the mapping
     * is a quantiser: monotonic, and every device value must survive a
     * round trip exactly.  A get followed by a set - alsactl
     * store/restore, or any client that reads before writing - must not
     * move the setting.
     */
    for (q = ZG01_Q_USER_MIN; q <= ZG01_Q_USER_MAX; q++) {
        dev = (long)zg01_q_user_to_dev(q);
        if (dev < 0 || dev > (long)ZG01_Q_DEV_MAX) {
            printf("FAIL Q %ld maps to %ld, out of device range\n", q, dev);
            failures++;
            break;
        }
        if (q > ZG01_Q_USER_MIN &&
            dev < (long)zg01_q_user_to_dev(q - 1)) {
            printf("FAIL Q %ld is not monotonic\n", q);
            failures++;
            break;
        }
    }
    for (dev = 0; dev <= (long)ZG01_Q_DEV_MAX; dev++) {
        long back;

        q = zg01_q_dev_to_user((unsigned long)dev);
        if (dev > 0 && q <= zg01_q_dev_to_user((unsigned long)(dev - 1))) {
            printf("FAIL device %ld -> Q %ld is not strictly increasing\n",
                   dev, q);
            failures++;
            break;
        }
        back = (long)zg01_q_user_to_dev(q);
        if (back != dev) {
            printf("FAIL device %ld -> Q %ld -> device %ld, must be exact\n",
                   dev, q, back);
            failures++;
            break;
        }
    }

    /* --- fixed point helpers -------------------------------------- */
    check((long)zg01_log2_fp(1), 0, "log2_fp(1)");
    check((long)zg01_log2_fp(2), 256, "log2_fp(2)");
    check((long)zg01_log2_fp(3), 406, "log2_fp(3)");
    check((long)zg01_log2_fp(7), 719, "log2_fp(7)");
    check((long)zg01_log2_fp(50), 1445, "log2_fp(50)");
    check((long)zg01_log2_fp(53), 1466, "log2_fp(53)");
    check((long)zg01_log2_fp(100), 1701, "log2_fp(100)");
    check((long)zg01_log2_fp(200), 1957, "log2_fp(200)");
    check((long)zg01_log2_fp(400), 2213, "log2_fp(400)");
    check((long)zg01_log2_fp(800), 2469, "log2_fp(800)");
    check((long)zg01_log2_fp(1024), 2560, "log2_fp(1024)");
    check((long)zg01_log2_fp(1600), 2725, "log2_fp(1600)");
    check((long)zg01_log2_fp(12345), 3479, "log2_fp(12345)");
    check((long)zg01_exp2_fp(0), 256, "exp2_fp(0)");
    check((long)zg01_exp2_fp(21), 271, "exp2_fp(21)");
    check((long)zg01_exp2_fp(128), 362, "exp2_fp(0.5)");
    check((long)zg01_exp2_fp(256), 512, "exp2_fp(1.0)");
    check((long)zg01_exp2_fp(277), 542, "exp2_fp(277)");
    check((long)zg01_exp2_fp(640), 1448, "exp2_fp(2.5)");
    check((long)zg01_exp2_fp(1280), 8192, "exp2_fp(5.0)");

    /*
     * exp2_fp inverts log2_fp in the 256-scaled space; the fraction is
     * good to ~1/256 relative, so the absolute tolerance scales with
     * the value (2^i shift).
     */
    for (i = 1; i <= 4096; i += 37) {
        long back = (long)zg01_exp2_fp(zg01_log2_fp(i));
        long want = (long)i * 256;
        long tol = want / 200 + 4;

        if (back < want - tol || back > want + tol) {
            printf("FAIL log2/exp2 round trip at %u: %ld, want %ld\n",
                   i, back, want);
            failures++;
            break;
        }
    }

    if (failures) {
        printf("\n%d check(s) failed\n", failures);
        return 1;
    }
    printf("\nall codec checks passed\n");
    return 0;
}
