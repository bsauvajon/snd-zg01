#!/usr/bin/env bash
# Contract test between the driver's control table and the device map.
#
# src/zg01_control.c declares one row per kcontrol, each naming a device
# address (src/zg01.h) with a range and a default.  This test checks that
# every row
#
#   * names an address that exists in tools/vendor/device-map-zg01.json
#     and resolves to the expected device parameter,
#   * carries the vendor model's bounds and default,
#   * is unique: page 0 offset 0x24 and page 1 offset 0x24 are different
#     parameters (LIMITER attack and EQ low shape), so two controls
#     sharing an address would silently share one cache slot.
#
# The expectations below are the vendor model's values, taken from
# MemAPI_ZG01.dll (docs/PARAMETER_INVENTORY.md); the device names are the
# ones the library registers (docs/DEVICE_PARAMETER_MAP.md).  The effect
# chain rows were derived the same way and are documented in
# docs/EFFECTS_CONTROL_PLAN.md, including the correspondence between the
# vendor's CamelCase names and the device's registered names, which is the
# one thing neither source carries.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

python3 - "$repo_root" <<'PY'
import json
import re
import sys

root = sys.argv[1]

# ---------------------------------------------------------------- headers
def plain_int(text):
    """An integer literal, tolerating parentheses and u/l suffixes."""
    token = text.strip()
    while token.startswith('(') and token.endswith(')'):
        token = token[1:-1].strip()
    token = re.sub(r'(?i)[ul]+$', '', token)
    if re.fullmatch(r'[+-]?(0x[0-9a-fA-F]+|\d+)', token):
        return int(token, 0)
    return None


def definitions(path):
    """Integer #defines, resolved through the files that hold them."""
    found = {}
    for name in ('src/zg01.h', 'src/zg01_codec.h'):
        with open(f'{root}/{name}') as handle:
            for line in handle:
                match = re.match(r'#define\s+(\w+)\s+(.+?)\s*(?:/\*.*)?$', line)
                if not match:
                    continue
                value = plain_int(match.group(2))
                if value is not None:
                    found[match.group(1)] = value
    return found


defs = definitions('src/zg01.h')
ADDR = re.compile(r'ZG01_ADDR\((\d+),\s*(0x[0-9a-fA-F]+|\d+)\)')
PAGE_SIZE = defs['ZG01_PARAM_PAGE_SIZE']

addresses = {}
with open(f'{root}/src/zg01.h') as handle:
    for line in handle:
        match = re.match(r'#define\s+(ZG01_PARAM_\w+)(\(n\))?\s+(.*)$', line)
        if not match:
            continue
        name, is_fn, value = match.group(1), match.group(2), match.group(3)
        if is_fn:
            continue
        addr = ADDR.search(value)
        if addr:
            addresses[name] = (int(addr.group(1)) * PAGE_SIZE
                               + int(addr.group(2), 0))

# indexed macros: ZG01_PARAM_EQ_GAIN(n) ZG01_ADDR(1, 0x28 + (n))
indexed = re.compile(r'ZG01_ADDR\(\s*(\d+),\s*(0x[0-9a-fA-F]+)\s*\+\s*\(n\)\s*\)')
with open(f'{root}/src/zg01.h') as handle:
    for line in handle:
        match = re.match(r'#define\s+(ZG01_PARAM_\w+)\(n\)\s+(.*)$', line)
        if not match:
            continue
        addr = indexed.search(match.group(2))
        if addr:
            addresses[match.group(1)] = (int(addr.group(1)) * PAGE_SIZE
                                         + int(addr.group(2), 0))

assert addresses, 'no parameter addresses found in src/zg01.h'

# ------------------------------------------------------------------ table
ROW = re.compile(r'\{\s*"([^"]+)",\s*(.*?)\},', re.S)
with open(f'{root}/src/zg01_control.c') as handle:
    source = handle.read()
table = source.split('zg01_param_ctls[] = {', 1)[1].split('\n};', 1)[0]

rows = []
for name, body in ROW.findall(table):
    body = ' '.join(body.split())
    addr_match = re.match(r'(ZG01_PARAM_\w+)(?:\((\d)\))?', body)
    type_match = re.search(r'SNDRV_CTL_ELEM_TYPE_(\w+)', body)
    tail = body[type_match.end():]
    fields = [f.strip() for f in tail.split(',') if f.strip()]
    assert addr_match and type_match and len(fields) >= 4, (name, body)
    macro = addr_match.group(1)
    if addr_match.group(2) is not None:
        addr = addresses[macro] + int(addr_match.group(2))
    else:
        addr = addresses[macro]
    conv = re.search(r'CONV_(\w+)', fields[2]).group(1)

    def resolve(token):
        value = plain_int(token)
        if value is not None:
            return value
        if token in defs:
            return defs[token]
        raise AssertionError(f'cannot resolve {token!r} in row {name!r}')

    rows.append({'name': name, 'addr': addr, 'type': type_match.group(1),
                 'min': resolve(fields[0]), 'max': resolve(fields[1]),
                 'conv': conv, 'def': resolve(fields[3])})

# -------------------------------------------------------- generated table
with open(f'{root}/src/zg01_controls_generated.h') as handle:
    generated_source = handle.read()
gen_row = re.compile(
    r'\{\s*"([^"]+)",\s*(\d+),\s*SNDRV_CTL_ELEM_TYPE_(\w+),\s*'
    r'(-?\d+),\s*(-?\d+),\s*CONV_(\w+),\s*(-?\d+)\s*\},')
generated_rows = [
    {'name': name, 'addr': int(addr), 'type': kind,
     'min': int(vmin), 'max': int(vmax), 'conv': conv, 'def': int(default)}
    for name, addr, kind, vmin, vmax, conv, default
    in gen_row.findall(generated_source)
]
assert len(generated_rows) == 159, (
    f'generated table has {len(generated_rows)} rows, expected 159')
assert len({row['addr'] for row in generated_rows}) == 159, (
    'generated table has duplicate addresses')
max_generated = max(row['addr'] for row in generated_rows)
assert f'#define ZG01_EXTRA_PARAM_MAX_ADDR {max_generated}' in generated_source
cache_size = defs['ZG01_PARAM_PAGES'] * defs['ZG01_PARAM_PAGE_SIZE']
assert max_generated < cache_size, (
    f'generated address 0x{max_generated:03x} is outside the '
    f'{cache_size}-entry cache')
assert '#include "zg01_controls_generated.h"' in source, \
    'src/zg01_control.c does not include the generated table'

# ------------------------------------------------------------------- map
data = json.load(open(f'{root}/tools/vendor/device-map-zg01.json'))
params = {}
for key, records in data['groups'].items():
    kind, page = (int(part) for part in key.split('/'))
    if kind != 1:
        continue
    for record in records:
        params[page * 256 + record['offset']] = record['name']

with open(f'{root}/tools/vendor/name-map-zg01.json') as handle:
    vendor_names = json.load(handle)
with open(f'{root}/tools/vendor/parameter-model-zg01.json') as handle:
    vendor_model = json.load(handle)


def q_user(dev):
    """Driver's 0.01-Q value for a device step (anchors 50 and 1600)."""
    return round(50 * 2 ** (dev / 12))


# --------------------------------------------------------- expectations
# control name -> device parameter.  Addresses, bounds and defaults are
# not repeated here: they come from the device map and the vendor model
# (tools/vendor/parameter-model-zg01.json, joined by name-map-zg01.json),
# then converted to the units the control advertises.  A transcription
# error in either source therefore cannot hide behind a copy in this
# file.
EXPECTED = {
    'P48V Capture Switch': 'mic_p48v_onf',
    'Mute Capture Switch': 'mic_mute_onf',
    'Gate Capture Switch': 'mic_gate_onf',
    'Gate': 'mic_gate_level',
    'Gate Threshold': 'mic_gate_thhold',
    'Gate Range': 'mic_gate_range',
    'Gate Attack': 'mic_gate_attack',
    'Gate Hold': 'mic_gate_hold',
    'Gate Decay': 'mic_gate_decay',
    'Compressor Capture Switch': 'mic_comp_onf',
    'Compressor': 'mic_comp_level',
    'Compressor Threshold': 'mic_comp_thhold',
    'Compressor Ratio': 'mic_comp_ratio',
    'Compressor Attack': 'mic_comp_attack',
    'Compressor Release': 'mic_comp_release',
    'Compressor Knee': 'mic_comp_knee',
    'Compressor Out Gain': 'mic_comp_outgain',
    'Limiter Capture Switch': 'mic_limt_onf',
    'Limiter': 'mic_limt_level',
    'Limiter Threshold': 'mic_limt_thhold',
    'Limiter Attack': 'mic_limt_attack',
    'Limiter Release': 'mic_limt_release',
    'Limiter Knee': 'mic_limt_knee',
    'Limiter Out Gain': 'mic_limt_outgain',
    'EQ Capture Switch': 'mic_eq_onf',
    'EQ Low Shape': 'mic_eq_v2_1_SHAPE',
    'EQ High Shape': 'mic_eq_v2_4_SHAPE',
    'EQ Band 1 Gain': 'mic_eq_v2_1_G',
    'EQ Band 2 Gain': 'mic_eq_v2_2_G',
    'EQ Band 3 Gain': 'mic_eq_v2_3_G',
    'EQ Band 4 Gain': 'mic_eq_v2_4_G',
    'EQ Band 1 Frequency': 'mic_eq_v2_1_F',
    'EQ Band 2 Frequency': 'mic_eq_v2_2_F',
    'EQ Band 3 Frequency': 'mic_eq_v2_3_F',
    'EQ Band 4 Frequency': 'mic_eq_v2_4_F',
    'EQ Band 1 Q': 'mic_eq_v2_1_Q',
    'EQ Band 2 Q': 'mic_eq_v2_2_Q',
    'EQ Band 3 Q': 'mic_eq_v2_3_Q',
    'EQ Band 4 Q': 'mic_eq_v2_4_Q',
    'Voice Transformer': 'vic_trn_on',
    'Voice Transformer Type': 'vic_trn_type',
    'Voice Harmony Transpose': 'vic_trn_hrm_trn',
    'Voice Harmony Formant': 'vic_trn_hrm_fmt',
    'Voice Harmony Level': 'vic_trn_hrm_lvl',
    'Voice Harmony Correct': 'vic_trn_hrm_corr',
    'Voice Pitch 1': 'vic_trn_pit_pit1',
    'Voice Pitch 1 Fine': 'vic_trn_pit_fin1',
    'Voice Pitch 1 Level': 'vic_trn_pit_lvl1',
    'Voice Pitch 2': 'vic_trn_pit_pit2',
    'Voice Pitch 2 Fine': 'vic_trn_pit_fin2',
    'Voice Pitch 2 Level': 'vic_trn_pit_lvl2',
    'Voice Harmony Out Level': 'voice_harm_level',
    'Voice Pitch Out Level': 'voice_pitch_level',
    'Voice FX': 'vic_fx_on',
    'Voice FX Type': 'vic_fx_type',
    'Voice Radio Cutoff': 'vic_fx_rdo_cut',
    'Voice Radio Drive': 'vic_fx_rdo_drv',
    'Voice Radio LPF': 'vic_fx_rdo_lpf',
    'Voice Radio Out Level': 'voice_radio_level',
    'Voice Tremolo Speed': 'vic_fx_trm_spd',
    'Voice Tremolo Depth': 'vic_fx_rdo_dep',
    'Voice Tremolo Out Level': 'voice_tremolo_level',
    'Voice Reverb': 'vic_rev_on',
    'Voice Reverb Wet': 'vic_rev_wet',
    'Voice Reverb Type': 'vic_rev_type',
    'Voice Reverb Time': 'vic_rev_time',
    'Voice Reverb Init Delay': 'vic_rev_init',
    'Voice Reverb High Ratio': 'vic_rev_high',
    'Voice Reverb Density': 'vic_rev_dens',
    'Voice Reverb Diffusion': 'vic_rev_diff',
    'Voice Reverb HPF': 'vic_rev_hpf',
    'Voice Reverb LPF': 'vic_rev_lpf',
    'Voice Tone': 'vic_tc_on',
    'Voice Tone High Switch': 'vic_tc_hi_on',
    'Voice Tone High Shape': 'vic_tc_hi_sh',
    'Voice Tone High Gain': 'vic_tc_hi_g',
    'Voice Tone High Frequency': 'vic_tc_hi_f',
    'Voice Tone High Q': 'vic_tc_hi_q',
    'Voice Tone Mid Switch': 'vic_tc_md_on',
    'Voice Tone Mid Gain': 'vic_tc_md_g',
    'Voice Tone Mid Frequency': 'vic_tc_md_f',
    'Voice Tone Mid Q': 'vic_tc_md_q',
    'Voice Tone Low Switch': 'vic_tc_lo_on',
    'Voice Tone Low Shape': 'vic_tc_lo_sh',
    'Voice Tone Low Gain': 'vic_tc_lo_g',
    'Voice Tone Low Frequency': 'vic_tc_lo_f',
    'Voice Tone Low Q': 'vic_tc_lo_q',
    'Sound Effect': 'se_onf',
    'Sound Effect Level': 'se_level',
    'Sound Effect Level Steps': 'se_db_level',
    'Sound Effect Frequency': 'se_freq',
    'Sound Effect Waveform': 'se_wave',
    'Echo': 'ech_on',
    'Echo Stereo': 'ech_stereo',
    'Echo Time': 'ech_time',
    'Echo Delay Level': 'ech_dly_lvl',
    'Echo Reverb Level': 'ech_rev_lvl',
    'Echo Delay Time': 'ech_dly_time',
    'Echo Delay High Ratio': 'ech_dly_hi_rat',
    'Echo Feedback': 'ech_feed',
    'Echo Reverb Type': 'ech_rev_type',
    'Echo Reverb Time': 'ech_rev_time',
    'Echo Init Delay': 'ech_ini_del',
    'Echo High Ratio': 'ech_hi_rat',
    'Echo Density': 'ech_dens',
    'Echo Diffusion': 'ech_diff',
    'Echo HPF': 'ech_hpf',
    'Echo LPF': 'ech_lpf',
    'Game Effect 1': 'game_fx_a',
    'Game Effect 2': 'game_fx_b',
    'Game Effect 3': 'game_fx_c',
    'Game FX': 'gam_fx_onf',
    'Game FX Type': 'gam_fx_type',
    'Game De-esser M Switch': 'gam_dee_m_on',
    'Game De-esser M Low Frequency': 'gam_dee_m_lo',
    'Game De-esser M High Frequency': 'gam_dee_m_hi',
    'Game De-esser M Threshold': 'gam_dee_m_thr',
    'Game De-esser M Ratio': 'gam_dee_m_rat',
    'Game De-esser M Attack': 'gam_dee_m_att',
    'Game De-esser M Release': 'gam_dee_m_rel',
    'Game De-esser M Gain': 'gam_dee_m_gain',
    'Game De-esser S Switch': 'gam_dee_s_onf',
    'Game De-esser S Low Frequency': 'gam_dee_s_lo',
    'Game De-esser S High Frequency': 'gam_dee_s_hi',
    'Game De-esser S Threshold': 'gam_dee_s_thr',
    'Game De-esser S Ratio': 'gam_dee_s_rat',
    'Game De-esser S Attack': 'gam_dee_s_att',
    'Game De-esser S Release': 'gam_dee_s_rel',
    'Game De-esser S Gain': 'gam_dee_s_gain',
    'Game EQ M': 'gam_eq_m_onf',
    'Game EQ M High Switch': 'gam_eq_m_hi_on',
    'Game EQ M High Shape': 'gam_eq_m_hi_sh',
    'Game EQ M High Gain': 'gam_eq_m_hi_g',
    'Game EQ M High Frequency': 'gam_eq_m_hi_f',
    'Game EQ M High Q': 'gam_eq_m_hi_q',
    'Game EQ M Mid High Switch': 'gam_eq_m_mh_on',
    'Game EQ M Mid High Gain': 'gam_eq_m_mh_g',
    'Game EQ M Mid High Frequency': 'gam_eq_m_mh_f',
    'Game EQ M Mid High Q': 'gam_eq_m_mh_q',
    'Game EQ M Mid Low Switch': 'gam_eq_m_ml_on',
    'Game EQ M Mid Low Gain': 'gam_eq_m_ml_g',
    'Game EQ M Mid Low Frequency': 'gam_eq_m_ml_f',
    'Game EQ M Mid Low Q': 'gam_eq_m_ml_q',
    'Game EQ M Low Switch': 'gam_eq_m_lo_on',
    'Game EQ M Low Shape': 'gam_eq_m_lo_sh',
    'Game EQ M Low Gain': 'gam_eq_m_lo_g',
    'Game EQ M Low Frequency': 'gam_eq_m_lo_f',
    'Game EQ M Low Q': 'gam_eq_m_lo_q',
    'Game EQ S': 'gam_eq_s_onf',
    'Game EQ S High Switch': 'gam_eq_s_hi_on',
    'Game EQ S High Shape': 'gam_eq_s_hi_sh',
    'Game EQ S High Gain': 'gam_eq_s_hi_g',
    'Game EQ S High Frequency': 'gam_eq_s_hi_f',
    'Game EQ S High Q': 'gam_eq_s_hi_q',
    'Game EQ S Mid High Switch': 'gam_eq_s_mh_on',
    'Game EQ S Mid High Gain': 'gam_eq_s_mh_g',
    'Game EQ S Mid High Frequency': 'gam_eq_s_mh_f',
    'Game EQ S Mid High Q': 'gam_eq_s_mh_q',
    'Game EQ S Mid Low Switch': 'gam_eq_s_ml_on',
    'Game EQ S Mid Low Gain': 'gam_eq_s_ml_g',
    'Game EQ S Mid Low Frequency': 'gam_eq_s_ml_f',
    'Game EQ S Mid Low Q': 'gam_eq_s_ml_q',
    'Game EQ S Low Switch': 'gam_eq_s_lo_on',
    'Game EQ S Low Shape': 'gam_eq_s_lo_sh',
    'Game EQ S Low Gain': 'gam_eq_s_lo_g',
    'Game EQ S Low Frequency': 'gam_eq_s_lo_f',
    'Game EQ S Low Q': 'gam_eq_s_lo_q',
    'Game EQ': 'gam_eq_t_onf',
    'Game EQ Band 1 Switch': 'game_eq_v2_1_ONF',
    'Game EQ Band 1 Gain': 'game_eq_v2_1_G',
    'Game EQ Band 1 Frequency': 'game_eq_v2_1_F',
    'Game EQ Band 1 Q': 'game_eq_v2_1_Q',
    'Game EQ Band 2 Switch': 'game_eq_v2_2_ONF',
    'Game EQ Band 2 Gain': 'game_eq_v2_2_G',
    'Game EQ Band 2 Frequency': 'game_eq_v2_2_F',
    'Game EQ Band 2 Q': 'game_eq_v2_2_Q',
    'Game EQ Band 3 Switch': 'game_eq_v2_3_ONF',
    'Game EQ Band 3 Gain': 'game_eq_v2_3_G',
    'Game EQ Band 3 Frequency': 'game_eq_v2_3_F',
    'Game EQ Band 3 Q': 'game_eq_v2_3_Q',
    'Game EQ Band 4 Switch': 'game_eq_v2_4_ONF',
    'Game EQ Band 4 Gain': 'game_eq_v2_4_G',
    'Game EQ Band 4 Frequency': 'game_eq_v2_4_F',
    'Game EQ Band 4 Q': 'game_eq_v2_4_Q',
    'Game EQ Band 5 Switch': 'game_eq_v2_5_ONF',
    'Game EQ Band 5 Gain': 'game_eq_v2_5_G',
    'Game EQ Band 5 Frequency': 'game_eq_v2_5_F',
    'Game EQ Band 5 Q': 'game_eq_v2_5_Q',
    'Game EQ Band 6 Switch': 'game_eq_v2_6_ONF',
    'Game EQ Band 6 Gain': 'game_eq_v2_6_G',
    'Game EQ Band 6 Frequency': 'game_eq_v2_6_F',
    'Game EQ Band 6 Q': 'game_eq_v2_6_Q',
    'Game EQ Band 7 Switch': 'game_eq_v2_7_ONF',
    'Game EQ Band 7 Gain': 'game_eq_v2_7_G',
    'Game EQ Band 7 Frequency': 'game_eq_v2_7_F',
    'Game EQ Band 7 Q': 'game_eq_v2_7_Q',
    'Game EQ Band 8 Switch': 'game_eq_v2_8_ONF',
    'Game EQ Band 8 Gain': 'game_eq_v2_8_G',
    'Game EQ Band 8 Frequency': 'game_eq_v2_8_F',
    'Game EQ Band 8 Q': 'game_eq_v2_8_Q',
    'Game EQ Band 9 Switch': 'game_eq_v2_9_ONF',
    'Game EQ Band 9 Gain': 'game_eq_v2_9_G',
    'Game EQ Band 9 Frequency': 'game_eq_v2_9_F',
    'Game EQ Band 9 Q': 'game_eq_v2_9_Q',
    'Game EQ Band 10 Switch': 'game_eq_v2_10_ONF',
    'Game EQ Band 10 Gain': 'game_eq_v2_10_G',
    'Game EQ Band 10 Frequency': 'game_eq_v2_10_F',
    'Game EQ Band 10 Q': 'game_eq_v2_10_Q',
}

by_name = {row['name']: row for row in rows}
missing = sorted(set(EXPECTED) - set(by_name))
extra = sorted(set(by_name) - set(EXPECTED))
assert not missing, f'controls promised but absent from the table: {missing}'
assert not extra, f'controls absent from the expectations: {extra}'

seen = {}
for name, device in EXPECTED.items():
    row = by_name[name]
    expected_addr = next(addr for addr, dev in params.items() if dev == device)
    assert row['addr'] == expected_addr, (
        f'{name}: address 0x{row["addr"]:03x} but {device} lives at '
        f'0x{expected_addr:03x} (docs/DEVICE_PARAMETER_MAP.md)')

    vendor_name = vendor_names.get(device)
    assert vendor_name, f'{name}: {device} has no vendor-model name'
    model = vendor_model.get(vendor_name)
    assert model, f'{name}: {vendor_name} is absent from the vendor model'
    vmin, vmax, vdef = model['min'], model['max'], model['default']
    if row['conv'] == 'GAIN':
        vmin -= 180
        vmax -= 180
    elif row['conv'] == 'Q':
        vmin, vmax = q_user(vmin), q_user(vmax)
    # FREQ and ID are identity conversions: Hz and device steps.

    assert (row['min'], row['max'], row['def']) == (vmin, vmax, vdef), (
        f'{name}: min/max/default {row["min"]}/{row["max"]}/{row["def"]} '
        f'but {vendor_name} converts to {vmin}/{vmax}/{vdef}')
    assert row['min'] <= row['max'], f'{name}: empty range'
    if row['conv'] == 'ID':
        # same units on both sides: the default must be settable
        assert row['min'] <= row['def'] <= row['max'], (
            f'{name}: default {row["def"]} outside {row["min"]}..{row["max"]}')
    # the cache is seeded with a device value, which the encoder must take
    assert 0 <= row['def'] <= 0x3fffff, f'{name}: default out of range'
    assert row['addr'] not in seen, (
        f'{name} and {seen[row["addr"]]} share address 0x{row["addr"]:03x}: '
        f'they would share one cache slot')
    seen[row['addr']] = name

# The generated table must not alias, or reuse a name from, the manual one:
# two rows on one address would silently share a cache slot.
for row in generated_rows:
    assert row['name'] not in by_name, (
        f'{row["name"]!r} is in both the manual and the generated table')
    assert row['addr'] not in seen, (
        f'{row["name"]} and {seen[row["addr"]]} share address '
        f'0x{row["addr"]:03x}: they would share one cache slot')
    seen[row['addr']] = row['name']
assert len(seen) == 367, f'{len(seen)} writable controls, expected 367'

# The two bytes the frame carries are the base-128 digits of the address
# (docs/MIC_CONTROL_PROTOCOL.md): every address of the map must split into
# two 7-bit digits, and the split must be the address itself - a parameter
# the driver never derived this way would travel under a wrong digit.
page0_high = 0
for addr, dev in sorted(params.items()):
    flag, param_id = addr // 128, addr % 128
    assert 0 <= flag <= 0x7f, f'{dev}: high digit 0x{flag:02x} is not 7-bit'
    assert 0 <= param_id <= 0x7f, f'{dev}: low digit 0x{param_id:02x} is not 7-bit'
    assert flag * 128 + param_id == addr, f'{dev}: digits do not rebuild 0x{addr:03x}'
    if addr // 128 == 1:
        page0_high += 1

# Page 0 offsets at or above 0x80 need the high digit 0x01: that half of the
# page holds the game effect, binaural and monitor parameters, and the
# application drives some of them (its monitor volume sliders write page 0
# offset 0xe9 and 0xea), so the digit cannot be 0x00 as it is for the low
# half.  The count is pinned so the space stays visible.
assert page0_high == 128, page0_high

for name, row in by_name.items():
    flag, param_id = row['addr'] // 128, row['addr'] % 128
    assert 0 <= flag <= 0x7f and 0 <= param_id <= 0x7f

# the frame builders must derive the two digits from the address: a
# hardcoded page byte is how page 0 parameters used to be unreadable
for func in ('zg01_param_write_locked', 'zg01_param_read', 'zg01_param_parse'):
    segment = source.split(f'{func}(', 1)[1].split('\n}\n', 1)[0]
    for macro in ('ZG01_ADDR_FLAG', 'ZG01_ADDR_ID'):
        assert macro in segment, f'{func} does not derive the digits ({macro})'

# a put is a user action and must always reach the device: the cache is
# seeded from the model defaults and cannot be refreshed (page 0 answers
# reads with a status, and a reset does not touch it), so deduplicating a
# write against it swallowed real settings changes.
put = source.split('zg01_ctl_put(', 1)[1].split('\n}\n', 1)[0]
assert 'zg01_param_write_locked(dev, c->addr, raw)' in put, \
    'zg01_ctl_put must write the parameter under param_mutex'
assert 'mutex_lock(&dev->param_mutex)' in put, \
    'zg01_ctl_put must serialize the write and its cache update'
assert '*zg01_cache(dev, c->addr) == raw' not in put, \
    'zg01_ctl_put must not skip a write because of the cached value'

print(f'{len(rows)} controls: addresses, bounds and defaults match the device map')
PY

# Exercise the production macros, not a Python re-implementation of them.
work=$(mktemp -d /tmp/snd-zg01-address-digits.XXXXXX)
trap 'rm -rf "$work"' EXIT
cc -std=c11 -Wall -Wextra -Werror -I "$repo_root/src" \
   -o "$work/address-digits" "$repo_root/tests/test-address-digits.c"
"$work/address-digits"

printf 'control address contract passed\n'
