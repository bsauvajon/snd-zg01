/*
 * Host-side test for the MIC state dump parser.
 *
 * Build and run through tests/test-state-parser.sh; no kernel headers or
 * device are needed.  The fixture is the eight-frame dump captured from
 * the persisted state after Reset to ZG01.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>

#include "zg01_state.h"

static void fail(const char *message)
{
    fprintf(stderr, "state parser: %s\n", message);
    exit(1);
}

static void expect(const struct zg01_state_values *v, enum zg01_state_field f,
                   unsigned int value, const char *name)
{
    if (v->value[f] != value) {
        fprintf(stderr, "state parser: %s = %u, expected %u\n", name,
                v->value[f], value);
        exit(1);
    }
    if (!(v->known & ZG01_STATE_KNOWN(f))) {
        fprintf(stderr, "state parser: %s not marked known\n", name);
        exit(1);
    }
}

int main(int argc, char **argv)
{
    const char *path = argc > 1 ? argv[1] :
                       "tests/fixtures/mic-state-persisted.bin";
    unsigned char *frames;
    struct zg01_state_values v;
    FILE *file;
    long size;
    u32 all;

    file = fopen(path, "rb");
    if (!file)
        fail("cannot open fixture");
    if (fseek(file, 0, SEEK_END) != 0)
        fail("cannot seek fixture");
    size = ftell(file);
    if (size != ZG01_STATE_DUMP_SIZE)
        fail("fixture is not eight 512-byte frames");
    rewind(file);
    frames = malloc((size_t)size);
    if (!frames)
        fail("out of memory");
    if (fread(frames, 1, (size_t)size, file) != (size_t)size)
        fail("short fixture read");
    fclose(file);

    if (zg01_state_parse(frames, (unsigned int)size, &v) != 0)
        fail("persisted fixture was rejected");

    expect(&v, ZG01_SF_GATE_SWITCH, 1, "gate switch");
    expect(&v, ZG01_SF_GATE_VALUE, 60, "gate value");
    expect(&v, ZG01_SF_GATE_RANGE, 17, "gate range");
    expect(&v, ZG01_SF_GATE_ATTACK, 109, "gate attack");
    expect(&v, ZG01_SF_GATE_HOLD, 101, "gate hold");
    expect(&v, ZG01_SF_GATE_DECAY, 60, "gate decay");
    expect(&v, ZG01_SF_COMP_SWITCH, 1, "compressor switch");
    expect(&v, ZG01_SF_COMP_VALUE, 30, "compressor value");
    expect(&v, ZG01_SF_COMP_THRESHOLD, 61, "compressor threshold");
    expect(&v, ZG01_SF_COMP_RATIO, 6, "compressor ratio");
    expect(&v, ZG01_SF_COMP_KNEE, 1, "compressor knee");
    expect(&v, ZG01_SF_LIM_SWITCH, 1, "limiter switch");
    expect(&v, ZG01_SF_LIM_VALUE, 50, "limiter value");
    expect(&v, ZG01_SF_LIM_THRESHOLD, 65, "limiter threshold");
    expect(&v, ZG01_SF_LIM_KNEE, 1, "limiter knee");
    expect(&v, ZG01_SF_EQ_SWITCH, 1, "eq switch");
    expect(&v, ZG01_SF_EQ_LOW_SHAPE, 1, "eq low shape");
    expect(&v, ZG01_SF_EQ_HIGH_SHAPE, 1, "eq high shape");
    expect(&v, ZG01_SF_EQ_Q1, 7, "eq q1");
    expect(&v, ZG01_SF_EQ_Q2, 12, "eq q2");
    expect(&v, ZG01_SF_EQ_Q3, 17, "eq q3");
    expect(&v, ZG01_SF_EQ_Q4, 5, "eq q4");

    all = (1u << ZG01_SF_COUNT) - 1u;
    if (v.known != all)
        fail("known mask does not cover the direct fields");

    if (zg01_state_parse(frames, ZG01_STATE_DUMP_SIZE - 1, &v) != -EINVAL)
        fail("short dump was accepted");

    /* Every payload header/descriptor byte is part of the layout contract. */
    {
        unsigned int i;

        for (i = 0; i < 8; i++) {
            unsigned int packet = i / 3;
            unsigned int byte = i % 3;
            unsigned int index = packet * 4 + 1 + byte;
            u8 saved = frames[index];

            frames[index] ^= 0x01;
            if (zg01_state_parse(frames, ZG01_STATE_DUMP_SIZE, &v) != -EINVAL) {
                fprintf(stderr,
                        "state parser: header byte %u change was accepted\n",
                        i);
                exit(1);
            }
            frames[index] = saved;
        }
    }

    /* A meter frame uses a different CIN, not the state continuation. */
    frames[0] = 0x05;
    if (zg01_state_parse(frames, ZG01_STATE_DUMP_SIZE, &v) != -EINVAL)
        fail("meter-like frame was accepted as state");
    frames[0] = 0x04;

    /* A data byte outside USB-MIDI's 7-bit range is not a state byte. */
    {
        unsigned int packet = 1460 / 3;
        unsigned int byte = 1460 % 3;
        unsigned int index = packet * 4 + 1 + byte;

        frames[index] = 0x80;
        if (zg01_state_parse(frames, ZG01_STATE_DUMP_SIZE, &v) != -EINVAL)
            fail("non-7-bit payload byte was accepted");
        frames[index] = 0x01;
    }

    free(frames);
    printf("state parser: ok\n");
    return 0;
}
