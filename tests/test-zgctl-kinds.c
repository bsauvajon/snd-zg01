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

    /* Hz for the mic EQ and the ten-band game EQ only */
    expect_kind(KIND_FREQ, "Game EQ Band 1 Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_FREQ, "Game EQ Band 10 Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Game EQ M High Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Voice Tone High Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_INT, "Sound Effect Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);

    /* bandwidths keep their 0.01 Q unit in the effect chain too */
    expect_kind(KIND_Q, "Voice Tone High Q", SNDRV_CTL_ELEM_TYPE_INTEGER);
    expect_kind(KIND_Q, "Game EQ Band 1 Q", SNDRV_CTL_ELEM_TYPE_INTEGER);

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

    if (failures) {
        printf("%d classification failure(s)\n", failures);
        return 1;
    }

    printf("zgctl unit classification contract passed\n");
    return 0;
}
