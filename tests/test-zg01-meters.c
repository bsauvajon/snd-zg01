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
    struct zg01_meter_values sentinel;
    unsigned int i;

    memset(&sentinel, -1, sizeof(sentinel));
    memcpy(frame_short, frame_gate_closed_quiet, sizeof(frame_short));
    memcpy(frame_control, frame_gate_closed_quiet, sizeof(frame_gate_closed_quiet));
    memcpy(frame_bad_cin, frame_gate_closed_quiet, sizeof(frame_bad_cin));
    frame_bad_cin[100] = 0x05;
    memcpy(frame_bad_f7, frame_gate_closed_quiet, sizeof(frame_bad_f7));
    frame_bad_f7[sizeof(frame_bad_f7) - 1] = 0x00;

    assert(zg01_meter_parse(frame_gate_closed_quiet,
                            sizeof(frame_gate_closed_quiet), &v) == 0);
    assert(v.mic_in == 52 && v.mic_out == 0);
    assert(v.stream_mix[2] == 38 && v.stream_mix[3] == 39);
    assert(v.stream_mix[6] == 38 && v.stream_mix[7] == 38);
    for (i = 0; i < 8; i++) {
        assert(v.pc_in[i] == 0 && v.hdmi_in[i] == 0 && v.game_mix[i] == 0);
    }

    assert(zg01_meter_parse(frame_mic_tap, sizeof(frame_mic_tap), &v) == 0);
    assert(v.mic_in == 118 && v.mic_out == 121);
    assert(v.stream_mix[0] == 121 && v.stream_mix[1] == 121);
    assert(v.stream_mix[2] == 38 && v.stream_mix[3] == 38);
    assert(v.stream_mix[6] == 126 && v.stream_mix[7] == 126);
    assert(v.pc_in[0] == 0 && v.pc_in[7] == 0);

    assert(zg01_meter_parse(frame_mic_decay_peak_held,
                            sizeof(frame_mic_decay_peak_held), &v) == 0);
    assert(v.mic_in == 102 && v.mic_out == 108);

    assert(zg01_meter_parse(frame_all_groups,
                            sizeof(frame_all_groups), &v) == 0);
    assert(v.mic_in == 50 && v.mic_out == 55);
    for (i = 0; i < 8; i++) {
        assert(v.pc_in[i] == 60 + (int)i);
        assert(v.hdmi_in[i] == 70 + (int)i);
        assert(v.game_mix[i] == 80 + (int)i);
        assert(v.stream_mix[i] == 90 + (int)i);
    }

    v = sentinel;
    assert(zg01_meter_parse(frame_short, sizeof(frame_short), &v) == -EINVAL);
    assert(zg01_meter_parse(frame_control, sizeof(frame_control), &v) == -EINVAL);
    assert(zg01_meter_parse(frame_bad_cin, sizeof(frame_bad_cin), &v) == -EINVAL);
    assert(zg01_meter_parse(frame_bad_f7, sizeof(frame_bad_f7), &v) == -EINVAL);
    assert(v.mic_in == -1 && v.mic_out == -1);
    assert(v.pc_in[0] == -1 && v.stream_mix[7] == -1);
    puts("zg01 meter parser contract passed");
    return 0;
}
