#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "zg01_meters.h"
#include "fixtures/meters/frames.h"

static unsigned char frame_short[100];
static unsigned char frame_control[512];
static unsigned char frame_bad_cin[sizeof(frame_gate_closed_quiet)];
static unsigned char frame_bad_f7[sizeof(frame_gate_closed_quiet)];

int main(void)
{
    struct zg01_meter_values v;
    struct zg01_meter_values sentinel = { -1, -1, -1, -1 };

    memcpy(frame_short, frame_gate_closed_quiet, sizeof(frame_short));
    memcpy(frame_control, frame_gate_closed_quiet, sizeof(frame_gate_closed_quiet));
    memcpy(frame_bad_cin, frame_gate_closed_quiet, sizeof(frame_bad_cin));
    frame_bad_cin[100] = 0x05;
    memcpy(frame_bad_f7, frame_gate_closed_quiet, sizeof(frame_bad_f7));
    frame_bad_f7[sizeof(frame_bad_f7) - 1] = 0x00;

    assert(zg01_meter_parse(frame_gate_closed_quiet,
                            sizeof(frame_gate_closed_quiet), &v) == 0);
    assert(v.mic_in == 52 && v.mic_out == 0);
    assert(v.pc_in == 38 && v.stream_mix == 38);

    assert(zg01_meter_parse(frame_mic_tap, sizeof(frame_mic_tap), &v) == 0);
    assert(v.mic_in == 118 && v.mic_out == 121);
    assert(v.pc_in == 38 && v.stream_mix == 126);

    assert(zg01_meter_parse(frame_mic_decay_peak_held,
                            sizeof(frame_mic_decay_peak_held), &v) == 0);
    assert(v.mic_in == 102 && v.mic_out == 108);

    v = sentinel;
    assert(zg01_meter_parse(frame_short, sizeof(frame_short), &v) == -EINVAL);
    assert(zg01_meter_parse(frame_control, sizeof(frame_control), &v) == -EINVAL);
    assert(zg01_meter_parse(frame_bad_cin, sizeof(frame_bad_cin), &v) == -EINVAL);
    assert(zg01_meter_parse(frame_bad_f7, sizeof(frame_bad_f7), &v) == -EINVAL);
    assert(v.mic_in == -1 && v.mic_out == -1);
    assert(v.pc_in == -1 && v.stream_mix == -1);
    puts("zg01 meter parser contract passed");
    return 0;
}
