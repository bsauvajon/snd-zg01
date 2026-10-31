/*
 * Contract test for zgctl's unit classification.
 *
 * The CLI turns values into human units, so the classification decides both
 * what it accepts and what it prints: the EQ band gains are 0.1 dB around
 * 0 dB, while the dynamics "Out Gain" controls are raw device steps with no
 * established unit (docs/MIC_CONTROL_PROTOCOL.md).  Treating the latter as
 * dB made `zgctl set "Compressor Out Gain" 10` write 100 and reject every
 * value from 19 upwards.
 *
 * The CLI is included rather than linked so its static helpers can be
 * called directly; BUILD is the only thing that differs.
 */
#define main zgctl_cli_main
#include "../tools/zgctl.c"
#undef main

static int failures;

static void expect_kind(enum kind want, const char *name, int type)
{
    struct snd_ctl_elem_info info;

    memset(&info, 0, sizeof(info));
    info.type = type;
    strncpy((char *)info.id.name, name, sizeof(info.id.name) - 1);

    if (kind_of(&info) != want) {
        printf("FAIL %-22s classified %d, want %d\n", name, kind_of(&info),
               want);
        failures++;
    }
}

static void expect_parse(enum kind k, const char *text, long want)
{
    long got = -1;

    if (parse_value(k, text, &got) != 0 || got != want) {
        printf("FAIL parse \"%s\" as kind %d -> %ld, want %ld\n", text, k, got,
               want);
        failures++;
    }
}

static void expect_format(enum kind k, long value, const char *want)
{
    char out[64];

    format_value(k, value, out, sizeof(out));
    if (strcmp(out, want)) {
        printf("FAIL format %ld as kind %d -> \"%s\", want \"%s\"\n", value, k,
               out, want);
        failures++;
    }
}

/*
 * The preset loader converts a device value to the control's user unit
 * before it plans or writes anything; the same helper backs both paths.
 */
static void expect_preset_user(int conv, unsigned int value, long want)
{
    struct zg01_user_control ctl;
    long got = -1;

    memset(&ctl, 0, sizeof(ctl));
    ctl.conv = conv;
    if (preset_user_value(&ctl, value, &got) != 0 || got != want) {
        printf("FAIL preset convert conv %d from %u -> %ld, want %ld\n", conv,
               value, got, want);
        failures++;
    }
}

static void expect_preset_user_fails(int conv, unsigned int value)
{
    struct zg01_user_control ctl;
    long got;

    memset(&ctl, 0, sizeof(ctl));
    ctl.conv = conv;
    if (preset_user_value(&ctl, value, &got) == 0) {
        printf("FAIL preset convert conv %d accepted %u -> %ld\n", conv,
               value, got);
        failures++;
    }
}

static void expect_preset_status(enum preset_status want,
                                 const struct zg01_user_control *ctl,
                                 unsigned int value, int allow_raw)
{
    long user = (long)value;
    enum preset_status got = preset_status_of(ctl, value, allow_raw, &user);

    if (got != want) {
        printf("FAIL preset status for %s value %u -> %d, want %d\n",
               ctl ? ctl->name : "unknown", value, got, want);
        failures++;
    }
}

static void test_preset_plan(void)
{
    static const struct zg01_user_control gain = {
        0x128, "EQ Band 1 Gain", -180, 180, CONV_GAIN, 180, 0,
    };
    static const struct zg01_user_control raw = {
        0x0f0, "Monitor EQ Band 4 Switch Steps", 0, ZG01_VALUE_MAX,
        CONV_ID, 0, ZG01_CTL_RAW,
    };
    static const struct zg01_user_control read_only = {
        0x028, "Mic Level Status", 0, 127, CONV_ID, 103,
        ZG01_CTL_READONLY,
    };

    /* device value to user unit, as used by dry-run and the write path */
    expect_preset_user(CONV_ID, 150, 150);
    expect_preset_user(CONV_GAIN, 0, -180);
    expect_preset_user(CONV_GAIN, 180, 0);
    expect_preset_user(CONV_GAIN, 360, 180);
    expect_preset_user(CONV_FREQ, 1000, 1000);
    expect_preset_user(CONV_Q, 0, 50);
    expect_preset_user(CONV_Q, 12, 100);
    expect_preset_user(CONV_Q, 60, 1599);
    expect_preset_user_fails(CONV_Q, ZG01_Q_DEV_MAX + 1);

    /* an address with no control, or a read-only row, is unsupported */
    expect_preset_status(PRESET_UNSUPPORTED, NULL, 5, 0);
    expect_preset_status(PRESET_UNSUPPORTED, &read_only, 5, 1);

    /* no clamping: +22.0 dB is past the +18.0 dB control maximum */
    expect_preset_status(PRESET_OUT_OF_RANGE, &gain, 400, 0);
    expect_preset_status(PRESET_APPLY, &gain, 150, 0);

    /* raw rows need the explicit opt-in */
    expect_preset_status(PRESET_RAW, &raw, 1, 0);
    expect_preset_status(PRESET_APPLY, &raw, 1, 1);
}

int main(void)
{
    /* switches */
    expect_kind(KIND_BOOL, "P48V Capture Switch", SNDRV_CTL_ELEM_TYPE_BOOLEAN);
    expect_kind(KIND_BOOL, "Limiter Capture Switch", SNDRV_CTL_ELEM_TYPE_BOOLEAN);

    /* raw device steps: levels and the dynamics sub-parameters */
    expect_kind(KIND_INT, "Gate", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Gate Threshold", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Compressor Out Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Limiter Out Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);

    /* the annotated units of the EQ */
    expect_kind(KIND_GAIN, "EQ Band 1 Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_GAIN, "EQ Band 4 Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_FREQ, "EQ Band 1 Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_Q, "EQ Band 1 Q", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "EQ Low Shape", SNDRV_CTL_ELEM_TYPE_INTEGER);

    /* the effect chain's 0.1 dB controls */
    expect_kind(KIND_GAIN, "Voice Harmony Out Level", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_GAIN, "Voice Tone High Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_GAIN, "Game EQ M High Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_GAIN, "Game EQ Band 1 Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_GAIN, "Game EQ Band 10 Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
    /* but the de-esser and dynamics gains stay raw */
    expect_kind(KIND_INT, "Game De-esser M Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Game De-esser S Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);

    /* selectors are integers: their switch is a separate control */
    expect_kind(KIND_INT, "Voice Transformer Type", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Voice FX Type", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Voice Tone High Shape", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Voice Tone Low Shape", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Game EQ M High Shape", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Game EQ M Low Shape", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Game EQ S High Shape", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Game EQ S Low Shape", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_BOOL, "Voice Tone High Switch",
                SNDRV_CTL_ELEM_TYPE_BOOLEAN);
    expect_kind(KIND_INT, "Sound Effect Level Steps",
                SNDRV_CTL_ELEM_TYPE_INTEGER);

    /* Hz for the mic EQ and the ten-band game EQ only */
    expect_kind(KIND_FREQ, "Game EQ Band 1 Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_FREQ, "Game EQ Band 10 Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Game EQ M High Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Voice Tone High Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Sound Effect Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);

    /* bandwidths keep their 0.01 Q unit in the effect chain too */
    expect_kind(KIND_Q, "Voice Tone High Q", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_Q, "Game EQ Band 1 Q", SNDRV_CTL_ELEM_TYPE_INTEGER);

    /* the V2 monitor EQ takes its units from the generated control map */
    expect_kind(KIND_GAIN, "Monitor EQ V2 Band 1 Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_FREQ, "Monitor EQ V2 Band 1 Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_Q, "Monitor EQ V2 Band 1 Q", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Binaural Voice Position Steps", SNDRV_CTL_ELEM_TYPE_INTEGER);
    /* recovered from a _ForBackup mirror: still device steps, not raw */
    expect_kind(KIND_INT, "Streaming Mic Level Steps", SNDRV_CTL_ELEM_TYPE_INTEGER);

    /* the action controls are not in the map and keep the name fallback */
    expect_kind(KIND_INT, "Save to ZG01", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Reset to ZG01", SNDRV_CTL_ELEM_TYPE_INTEGER);

    /* a raw step is taken and shown as a number, up to the top of its range */
    expect_parse(KIND_INT, "10", 10);
    expect_parse(KIND_INT, "180", 180);
    expect_format(KIND_INT, 50, "50");
    expect_format(KIND_INT, 180, "180");

    /* the EQ gain keeps its dB handling */
    expect_parse(KIND_GAIN, "+6.0", 60);
    expect_format(KIND_GAIN, 60, "+6.0 dB");

    /* and the other annotated families are untouched */
    expect_parse(KIND_FREQ, "1k", 1000);
    expect_format(KIND_FREQ, 1000, "1000 Hz (1.00 kHz)");
    expect_parse(KIND_Q, "2.00", 200);
    expect_format(KIND_Q, 200, "2.00");
    expect_format(KIND_BOOL, 1, "on");

    test_preset_plan();

    if (failures) {
        printf("%d classification failure(s)\n", failures);
        return 1;
    }

    printf("zgctl unit classification contract passed\n");
    return 0;
}
