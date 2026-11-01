/*
 * Minimal C dump adapter for the Python/C parser comparison test.
 *
 * It prints exactly what `tools/zgpreset.py dump FILE` prints, so
 * tests/test-zgpreset-compare.sh can compare the two implementations on the
 * same synthetic fixtures with cmp(1).
 */

#include <stdio.h>

#include "../tools/zgpreset.h"

static void json_string(const char *text)
{
    const unsigned char *p = (const unsigned char *)text;

    putchar('"');
    for (; *p; p++) {
        switch (*p) {
        case '"':
            fputs("\\\"", stdout);
            break;
        case '\\':
            fputs("\\\\", stdout);
            break;
        case '\b':
            fputs("\\b", stdout);
            break;
        case '\f':
            fputs("\\f", stdout);
            break;
        case '\n':
            fputs("\\n", stdout);
            break;
        case '\r':
            fputs("\\r", stdout);
            break;
        case '\t':
            fputs("\\t", stdout);
            break;
        default:
            if (*p < 0x20)
                printf("\\u%04x", *p);
            else
                putchar(*p);
            break;
        }
    }
    putchar('"');
}

int main(int argc, char **argv)
{
    struct zgpreset *preset = NULL;
    size_t i;
    int error;

    if (argc != 2) {
        fprintf(stderr, "usage: %s FILE\n", argv[0]);
        return 2;
    }

    error = zgpreset_open(argv[1], &preset);
    if (error) {
        fprintf(stderr, "%s: cannot parse %s: %d\n", argv[0], argv[1], error);
        return 1;
    }

    fputs("{\"block\": ", stdout);
    json_string(zgpreset_block(preset));
    fputs(", \"name\": ", stdout);
    json_string(zgpreset_name(preset));
    fputs(", \"params\": [", stdout);
    for (i = 0; i < preset->count; i++) {
        if (i)
            fputs(", ", stdout);
        printf("{\"address\": %u, \"value\": %u}",
               (unsigned int)preset->params[i].address,
               (unsigned int)preset->params[i].value);
    }
    fputs("]}\n", stdout);

    zgpreset_close(preset);
    return 0;
}
