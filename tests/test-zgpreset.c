/*
 * Contract test for the C MBDF preset parser (tools/zgpreset.h).
 *
 * The fixtures are synthetic MBDF files built by tests/test-zgpreset-c.sh;
 * no Yamaha or user preset is read or committed.  The layout is the same one
 * tests/test-zgpreset.py exercises, so the C port and the Python reference
 * agree on the container, schema and value rules.
 */

#include <errno.h>
#include <stdio.h>
#include <string.h>

#include "../tools/zgpreset.h"

static unsigned int checks;
static unsigned int failures;

static void check_int(long got, long want, const char *what)
{
    checks++;
    if (got != want) {
        printf("FAIL %s: got %ld, want %ld\n", what, got, want);
        failures++;
    }
}

static void check_str(const char *got, const char *want, const char *what)
{
    checks++;
    if (strcmp(got, want)) {
        printf("FAIL %s: got \"%s\", want \"%s\"\n", what, got, want);
        failures++;
    }
}

static void check_param(const struct zgpreset *preset, size_t index,
                        unsigned int address, unsigned int width,
                        unsigned int value)
{
    char label[64];

    checks++;
    if (index >= preset->count) {
        printf("FAIL param %zu is missing\n", index);
        failures++;
        return;
    }

    snprintf(label, sizeof(label), "param %zu address", index);
    check_int(preset->params[index].address, address, label);
    snprintf(label, sizeof(label), "param %zu width", index);
    check_int(preset->params[index].width, width, label);
    snprintf(label, sizeof(label), "param %zu value", index);
    check_int(preset->params[index].value, value, label);
}

static struct zgpreset *open_ok(const char *directory, const char *name)
{
    char path[4096];
    struct zgpreset *preset = NULL;
    int error;

    snprintf(path, sizeof(path), "%s/%s", directory, name);
    error = zgpreset_open(path, &preset);
    checks++;
    if (error) {
        printf("FAIL open %s: got %d, want 0\n", name, error);
        failures++;
        return NULL;
    }
    return preset;
}

static void expect_error(const char *directory, const char *name, int want)
{
    char path[4096];
    char label[128];
    struct zgpreset *preset = NULL;
    int error;

    snprintf(path, sizeof(path), "%s/%s", directory, name);
    error = zgpreset_open(path, &preset);
    snprintf(label, sizeof(label), "open %s", name);
    check_int(error, want, label);
    snprintf(label, sizeof(label), "open %s clears the result", name);
    check_int(preset != NULL, 0, label);
    zgpreset_close(preset);
}

static void expect_invalid(const char *directory, const char *name)
{
    expect_error(directory, name, -EINVAL);
}

int main(int argc, char **argv)
{
    struct zgpreset *preset = NULL;
    char path[4096];

    if (argc != 2) {
        printf("usage: %s FIXTURE_DIR\n", argv[0]);
        return 2;
    }

    preset = open_ok(argv[1], "mic.zgpreset");
    if (preset) {
        check_str(preset->block, "MIC INPUT", "mic block");
        check_str(preset->name, "Custom", "mic name");
        check_int(preset->count, 3, "mic count");
        check_param(preset, 0, 0x120, 1, 1);
        check_param(preset, 1, 0x121, 1, 0);
        check_param(preset, 2, 0x128, 2, 150);
    }
    zgpreset_close(preset);

    /* Game Effect: the schema header is two bytes, not six, so this pins
     * the marker search and the end-relative value start. */
    preset = open_ok(argv[1], "game.zgpreset");
    if (preset) {
        check_str(preset->block, "GAME EFFECT", "game block");
        check_str(preset->name, "Game", "game name");
        check_int(preset->count, 2, "game count");
        check_param(preset, 0, 0x075, 1, 3);
        check_param(preset, 1, 0x076, 1, 1);
    }
    zgpreset_close(preset);

    /* A long UTF-8 display name is preserved in full. */
    preset = open_ok(argv[1], "long-name.zgpreset");
    if (preset) {
        char want[128];

        memset(want, 'A', 62);
        memcpy(want + 62, "\xc3\xa9 tail", sizeof("\xc3\xa9 tail"));
        check_str(preset->name, want, "long utf-8 name");
    }
    zgpreset_close(preset);

    /* Metadata-only containers parse, but an empty preset does not. */
    expect_invalid(argv[1], "empty.zgpreset");

    /* .zg01 backups are a follow-up, not a supported input. */
    expect_error(argv[1], "backup.zg01", -ENOTSUP);

    expect_invalid(argv[1], "overlong.zgpreset");
    expect_invalid(argv[1], "bad-magic.zgpreset");
    expect_invalid(argv[1], "bad-width.zgpreset");
    expect_invalid(argv[1], "short-values.zgpreset");
    expect_invalid(argv[1], "missing-values.zgpreset");
    expect_invalid(argv[1], "not-mbdf.zgpreset");
    expect_invalid(argv[1], "missing-marker.zgpreset");
    expect_invalid(argv[1], "truncated-schema.zgpreset");

    /* I/O errors keep errno's sign; argument errors are EINVAL. */
    preset = NULL;
    snprintf(path, sizeof(path), "%s/does-not-exist", argv[1]);
    check_int(zgpreset_open(path, &preset), -ENOENT, "missing file");
    check_int(preset != NULL, 0, "missing file clears the result");

    preset = NULL;
    check_int(zgpreset_open(NULL, &preset), -EINVAL, "null path");
    check_int(preset != NULL, 0, "null path clears the result");
    check_int(zgpreset_open(path, NULL), -EINVAL, "null result");

    if (failures) {
        printf("%u of %u checks failed\n", failures, checks);
        return 1;
    }
    printf("zgpreset: %u C parser checks passed\n", checks);
    return 0;
}
