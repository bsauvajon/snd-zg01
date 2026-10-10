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

# ------------------------------------------------------------------- map
data = json.load(open(f'{root}/tools/vendor/device-map-zg01.json'))
params = {}
for key, records in data['groups'].items():
    kind, page = (int(part) for part in key.split('/'))
    if kind != 1:
        continue
    for record in records:
        params[page * 256 + record['offset']] = record['name']

# --------------------------------------------------------- expectations
# control name -> (device parameter, kcontrol min, kcontrol max, raw default)
#
# min/max are what the control advertises (the codec's user units for the
# converted rows, the device steps for CONV_ID rows); def is always the
# raw device value the cache is seeded with.
EXPECTED = {
    'P48V Capture Switch': ('mic_p48v_onf', 0, 1, 0),
    'Mute Capture Switch': ('mic_mute_onf', 0, 1, 0),
    'Gate Capture Switch': ('mic_gate_onf', 0, 1, 0),
    'Gate': ('mic_gate_level', 0, 100, 50),
    'Gate Threshold': ('mic_gate_thhold', 1, 73, 23),
    'Gate Range': ('mic_gate_range', 0, 73, 17),
    'Gate Attack': ('mic_gate_attack', 57, 283, 237),
    'Gate Hold': ('mic_gate_hold', 0, 213, 101),
    'Gate Decay': ('mic_gate_decay', 24, 300, 188),
    'Compressor Capture Switch': ('mic_comp_onf', 0, 1, 0),
    'Compressor': ('mic_comp_level', 0, 100, 50),
    'Compressor Threshold': ('mic_comp_thhold', 19, 73, 52),
    'Compressor Ratio': ('mic_comp_ratio', 0, 120, 14),
    'Compressor Attack': ('mic_comp_attack', 57, 283, 237),
    'Compressor Release': ('mic_comp_release', 24, 300, 228),
    'Compressor Knee': ('mic_comp_knee', 0, 2, 1),
    'Compressor Out Gain': ('mic_comp_outgain', 0, 180, 50),
    'Limiter Capture Switch': ('mic_limt_onf', 0, 1, 1),
    'Limiter': ('mic_limt_level', 0, 100, 50),
    'Limiter Threshold': ('mic_limt_thhold', 19, 73, 65),
    'Limiter Attack': ('mic_limt_attack', 57, 283, 237),
    'Limiter Release': ('mic_limt_release', 24, 300, 228),
    'Limiter Knee': ('mic_limt_knee', 0, 2, 1),
    'Limiter Out Gain': ('mic_limt_outgain', 0, 180, 0),
    'EQ Capture Switch': ('mic_eq_onf', 0, 1, 0),
    'EQ Low Shape': ('mic_eq_v2_1_SHAPE', 0, 1, 1),
    'EQ High Shape': ('mic_eq_v2_4_SHAPE', 0, 1, 1),
    'EQ Band 1 Gain': ('mic_eq_v2_1_G', -180, 180, 180),
    'EQ Band 2 Gain': ('mic_eq_v2_2_G', -180, 180, 180),
    'EQ Band 3 Gain': ('mic_eq_v2_3_G', -180, 180, 180),
    'EQ Band 4 Gain': ('mic_eq_v2_4_G', -180, 180, 180),
    'EQ Band 1 Frequency': ('mic_eq_v2_1_F', 20, 1000, 100),
    'EQ Band 2 Frequency': ('mic_eq_v2_2_F', 20, 20000, 500),
    'EQ Band 3 Frequency': ('mic_eq_v2_3_F', 20, 20000, 2500),
    'EQ Band 4 Frequency': ('mic_eq_v2_4_F', 500, 20000, 10000),
    'EQ Band 1 Q': ('mic_eq_v2_1_Q', 50, 1600, 12),
    'EQ Band 2 Q': ('mic_eq_v2_2_Q', 50, 1600, 12),
    'EQ Band 3 Q': ('mic_eq_v2_3_Q', 50, 1600, 12),
    'EQ Band 4 Q': ('mic_eq_v2_4_Q', 50, 1600, 12),

    # Microphone effect chain (docs/EFFECTS_CONTROL_PLAN.md).
    'Voice Transformer':             ('vic_trn_on', 0, 1, 0),
    'Voice Transformer Type':        ('vic_trn_type', 0, 1, 0),
    'Voice Harmony Transpose':       ('vic_trn_hrm_trn', 28, 100, 64),
    'Voice Harmony Formant':         ('vic_trn_hrm_fmt', 2, 126, 64),
    'Voice Harmony Level':           ('vic_trn_hrm_lvl', 0, 127, 127),
    'Voice Harmony Correct':         ('vic_trn_hrm_corr', 0, 100, 64),
    'Voice Pitch 1':                 ('vic_trn_pit_pit1', 40, 88, 64),
    'Voice Pitch 1 Fine':            ('vic_trn_pit_fin1', 14, 114, 64),
    'Voice Pitch 1 Level':           ('vic_trn_pit_lvl1', 0, 127, 100),
    'Voice Pitch 2':                 ('vic_trn_pit_pit2', 40, 88, 64),
    'Voice Pitch 2 Fine':            ('vic_trn_pit_fin2', 14, 114, 64),
    'Voice Pitch 2 Level':           ('vic_trn_pit_lvl2', 0, 127, 100),
    'Voice Harmony Out Level':       ('voice_harm_level', -180, 180, 180),
    'Voice Pitch Out Level':         ('voice_pitch_level', -180, 180, 180),
    'Voice FX':                      ('vic_fx_on', 0, 1, 0),
    'Voice FX Type':                 ('vic_fx_type', 0, 1, 0),
    'Voice Radio Cutoff':            ('vic_fx_rdo_cut', 50, 113, 61),
    'Voice Radio Drive':             ('vic_fx_rdo_drv', 1, 25, 10),
    'Voice Radio LPF':               ('vic_fx_rdo_lpf', 34, 60, 50),
    'Voice Radio Out Level':         ('voice_radio_level', -180, 180, 180),
    'Voice Tremolo Speed':           ('vic_fx_trm_spd', 1, 127, 89),
    'Voice Tremolo Depth':           ('vic_fx_rdo_dep', 0, 127, 64),
    'Voice Tremolo Out Level':       ('voice_tremolo_level', -180, 180, 180),
    'Voice Reverb':                  ('vic_rev_on', 0, 1, 0),
    'Voice Reverb Wet':              ('vic_rev_wet', 1, 127, 64),
    'Voice Reverb Type':             ('vic_rev_type', 0, 4, 0),
    'Voice Reverb Time':             ('vic_rev_time', 0, 57, 25),
    'Voice Reverb Init Delay':       ('vic_rev_init', 0, 63, 16),
    'Voice Reverb High Ratio':       ('vic_rev_high', 1, 10, 8),
    'Voice Reverb Density':          ('vic_rev_dens', 0, 4, 4),
    'Voice Reverb Diffusion':        ('vic_rev_diff', 0, 10, 10),
    'Voice Reverb HPF':              ('vic_rev_hpf', 0, 52, 0),
    'Voice Reverb LPF':              ('vic_rev_lpf', 34, 60, 53),
    'Voice Tone':                    ('vic_tc_on', 0, 1, 0),
    'Voice Tone High Switch':        ('vic_tc_hi_on', 0, 1, 1),
    'Voice Tone High Shape':         ('vic_tc_hi_sh', 0, 1, 1),
    'Voice Tone High Gain':          ('vic_tc_hi_g', -180, 180, 180),
    'Voice Tone High Frequency':     ('vic_tc_hi_f', 60, 124, 112),
    'Voice Tone High Q':             ('vic_tc_hi_q', 50, 1600, 12),
    'Voice Tone Mid Switch':         ('vic_tc_md_on', 0, 1, 1),
    'Voice Tone Mid Gain':           ('vic_tc_md_g', -180, 180, 180),
    'Voice Tone Mid Frequency':      ('vic_tc_md_f', 4, 124, 72),
    'Voice Tone Mid Q':              ('vic_tc_md_q', 50, 1600, 12),
    'Voice Tone Low Switch':         ('vic_tc_lo_on', 0, 1, 1),
    'Voice Tone Low Shape':          ('vic_tc_lo_sh', 0, 1, 1),
    'Voice Tone Low Gain':           ('vic_tc_lo_g', -180, 180, 180),
    'Voice Tone Low Frequency':      ('vic_tc_lo_f', 4, 72, 32),
    'Voice Tone Low Q':              ('vic_tc_lo_q', 50, 1600, 12),
    'Sound Effect':                  ('se_onf', 0, 1, 0),
    'Sound Effect Level':            ('se_level', 0, 2, 1),
    'Sound Effect Level dB':         ('se_db_level', 1, 73, 53),
    'Sound Effect Frequency':        ('se_freq', 29, 52, 44),
    'Sound Effect Waveform':         ('se_wave', 0, 3, 0),
    'Echo':                          ('ech_on', 0, 1, 0),
    'Echo Stereo':                   ('ech_stereo', 0, 1, 0),
    'Echo Time':                     ('ech_time', 0, 2, 1),
    'Echo Delay Level':              ('ech_dly_lvl', 0, 103, 103),
    'Echo Reverb Level':             ('ech_rev_lvl', 0, 103, 91),
    'Echo Delay Time':               ('ech_dly_time', 200, 7430, 2400),
    'Echo Delay High Ratio':         ('ech_dly_hi_rat', 1, 10, 8),
    'Echo Feedback':                 ('ech_feed', 1, 127, 94),
    'Echo Reverb Type':              ('ech_rev_type', 0, 4, 0),
    'Echo Reverb Time':              ('ech_rev_time', 0, 57, 25),
    'Echo Init Delay':               ('ech_ini_del', 0, 63, 16),
    'Echo High Ratio':               ('ech_hi_rat', 1, 10, 8),
    'Echo Density':                  ('ech_dens', 0, 4, 4),
    'Echo Diffusion':                ('ech_diff', 0, 10, 10),
    'Echo HPF':                      ('ech_hpf', 0, 52, 0),
    'Echo LPF':                      ('ech_lpf', 34, 60, 53),

    # Game effect chain.
    'Game Effect 1':                 ('game_fx_a', 0, 1, 0),
    'Game Effect 2':                 ('game_fx_b', 0, 1, 0),
    'Game Effect 3':                 ('game_fx_c', 0, 1, 0),
    'Game FX':                       ('gam_fx_onf', 0, 1, 0),
    'Game FX Type':                  ('gam_fx_type', 0, 100, 0),
    'Game De-esser M Switch':        ('gam_dee_m_on', 0, 1, 0),
    'Game De-esser M Low Frequency': ('gam_dee_m_lo', 4, 96, 36),
    'Game De-esser M High Frequency': ('gam_dee_m_hi', 60, 124, 93),
    'Game De-esser M Threshold':     ('gam_dee_m_thr', 19, 73, 57),
    'Game De-esser M Ratio':         ('gam_dee_m_rat', 0, 120, 10),
    'Game De-esser M Attack':        ('gam_dee_m_att', 57, 283, 237),
    'Game De-esser M Release':       ('gam_dee_m_rel', 24, 300, 228),
    'Game De-esser M Gain':          ('gam_dee_m_gain', 0, 180, 37),
    'Game De-esser S Switch':        ('gam_dee_s_onf', 0, 1, 0),
    'Game De-esser S Low Frequency': ('gam_dee_s_lo', 4, 96, 36),
    'Game De-esser S High Frequency': ('gam_dee_s_hi', 60, 124, 93),
    'Game De-esser S Threshold':     ('gam_dee_s_thr', 19, 73, 57),
    'Game De-esser S Ratio':         ('gam_dee_s_rat', 0, 120, 10),
    'Game De-esser S Attack':        ('gam_dee_s_att', 57, 283, 237),
    'Game De-esser S Release':       ('gam_dee_s_rel', 24, 300, 228),
    'Game De-esser S Gain':          ('gam_dee_s_gain', 0, 180, 37),
    'Game EQ M':                     ('gam_eq_m_onf', 0, 1, 0),
    'Game EQ M High Switch':         ('gam_eq_m_hi_on', 0, 1, 1),
    'Game EQ M High Shape':          ('gam_eq_m_hi_sh', 0, 1, 1),
    'Game EQ M High Gain':           ('gam_eq_m_hi_g', -180, 180, 180),
    'Game EQ M High Frequency':      ('gam_eq_m_hi_f', 60, 124, 112),
    'Game EQ M High Q':              ('gam_eq_m_hi_q', 50, 1600, 12),
    'Game EQ M Mid High Switch':     ('gam_eq_m_mh_on', 0, 1, 1),
    'Game EQ M Mid High Gain':       ('gam_eq_m_mh_g', -180, 180, 180),
    'Game EQ M Mid High Frequency':  ('gam_eq_m_mh_f', 4, 124, 96),
    'Game EQ M Mid High Q':          ('gam_eq_m_mh_q', 50, 1600, 12),
    'Game EQ M Mid Low Switch':      ('gam_eq_m_ml_on', 0, 1, 1),
    'Game EQ M Mid Low Gain':        ('gam_eq_m_ml_g', -180, 180, 180),
    'Game EQ M Mid Low Frequency':   ('gam_eq_m_ml_f', 4, 124, 72),
    'Game EQ M Mid Low Q':           ('gam_eq_m_ml_q', 50, 1600, 12),
    'Game EQ M Low Switch':          ('gam_eq_m_lo_on', 0, 1, 1),
    'Game EQ M Low Shape':           ('gam_eq_m_lo_sh', 0, 1, 1),
    'Game EQ M Low Gain':            ('gam_eq_m_lo_g', -180, 180, 180),
    'Game EQ M Low Frequency':       ('gam_eq_m_lo_f', 4, 72, 32),
    'Game EQ M Low Q':               ('gam_eq_m_lo_q', 50, 1600, 12),
    'Game EQ S':                     ('gam_eq_s_onf', 0, 1, 0),
    'Game EQ S High Switch':         ('gam_eq_s_hi_on', 0, 1, 1),
    'Game EQ S High Shape':          ('gam_eq_s_hi_sh', 0, 1, 1),
    'Game EQ S High Gain':           ('gam_eq_s_hi_g', -180, 180, 180),
    'Game EQ S High Frequency':      ('gam_eq_s_hi_f', 60, 124, 112),
    'Game EQ S High Q':              ('gam_eq_s_hi_q', 50, 1600, 12),
    'Game EQ S Mid High Switch':     ('gam_eq_s_mh_on', 0, 1, 1),
    'Game EQ S Mid High Gain':       ('gam_eq_s_mh_g', -180, 180, 180),
    'Game EQ S Mid High Frequency':  ('gam_eq_s_mh_f', 4, 124, 96),
    'Game EQ S Mid High Q':          ('gam_eq_s_mh_q', 50, 1600, 12),
    'Game EQ S Mid Low Switch':      ('gam_eq_s_ml_on', 0, 1, 1),
    'Game EQ S Mid Low Gain':        ('gam_eq_s_ml_g', -180, 180, 180),
    'Game EQ S Mid Low Frequency':   ('gam_eq_s_ml_f', 4, 124, 72),
    'Game EQ S Mid Low Q':           ('gam_eq_s_ml_q', 50, 1600, 12),
    'Game EQ S Low Switch':          ('gam_eq_s_lo_on', 0, 1, 1),
    'Game EQ S Low Shape':           ('gam_eq_s_lo_sh', 0, 1, 1),
    'Game EQ S Low Gain':            ('gam_eq_s_lo_g', -180, 180, 180),
    'Game EQ S Low Frequency':       ('gam_eq_s_lo_f', 4, 72, 32),
    'Game EQ S Low Q':               ('gam_eq_s_lo_q', 50, 1600, 12),
    'Game EQ':                       ('gam_eq_t_onf', 0, 1, 0),
    'Game EQ Band 1 Switch':         ('game_eq_v2_1_ONF', 0, 1, 1),
    'Game EQ Band 1 Gain':           ('game_eq_v2_1_G', -180, 180, 180),
    'Game EQ Band 1 Frequency':      ('game_eq_v2_1_F', 20, 1000, 31),
    'Game EQ Band 1 Q':              ('game_eq_v2_1_Q', 50, 1600, 42),
    'Game EQ Band 2 Switch':         ('game_eq_v2_2_ONF', 0, 1, 1),
    'Game EQ Band 2 Gain':           ('game_eq_v2_2_G', -180, 180, 180),
    'Game EQ Band 2 Frequency':      ('game_eq_v2_2_F', 20, 20000, 63),
    'Game EQ Band 2 Q':              ('game_eq_v2_2_Q', 50, 1600, 42),
    'Game EQ Band 3 Switch':         ('game_eq_v2_3_ONF', 0, 1, 1),
    'Game EQ Band 3 Gain':           ('game_eq_v2_3_G', -180, 180, 180),
    'Game EQ Band 3 Frequency':      ('game_eq_v2_3_F', 20, 20000, 125),
    'Game EQ Band 3 Q':              ('game_eq_v2_3_Q', 50, 1600, 42),
    'Game EQ Band 4 Switch':         ('game_eq_v2_4_ONF', 0, 1, 1),
    'Game EQ Band 4 Gain':           ('game_eq_v2_4_G', -180, 180, 180),
    'Game EQ Band 4 Frequency':      ('game_eq_v2_4_F', 20, 20000, 250),
    'Game EQ Band 4 Q':              ('game_eq_v2_4_Q', 50, 1600, 42),
    'Game EQ Band 5 Switch':         ('game_eq_v2_5_ONF', 0, 1, 1),
    'Game EQ Band 5 Gain':           ('game_eq_v2_5_G', -180, 180, 180),
    'Game EQ Band 5 Frequency':      ('game_eq_v2_5_F', 20, 20000, 500),
    'Game EQ Band 5 Q':              ('game_eq_v2_5_Q', 50, 1600, 42),
    'Game EQ Band 6 Switch':         ('game_eq_v2_6_ONF', 0, 1, 1),
    'Game EQ Band 6 Gain':           ('game_eq_v2_6_G', -180, 180, 180),
    'Game EQ Band 6 Frequency':      ('game_eq_v2_6_F', 20, 20000, 1000),
    'Game EQ Band 6 Q':              ('game_eq_v2_6_Q', 50, 1600, 42),
    'Game EQ Band 7 Switch':         ('game_eq_v2_7_ONF', 0, 1, 1),
    'Game EQ Band 7 Gain':           ('game_eq_v2_7_G', -180, 180, 180),
    'Game EQ Band 7 Frequency':      ('game_eq_v2_7_F', 20, 20000, 2000),
    'Game EQ Band 7 Q':              ('game_eq_v2_7_Q', 50, 1600, 42),
    'Game EQ Band 8 Switch':         ('game_eq_v2_8_ONF', 0, 1, 1),
    'Game EQ Band 8 Gain':           ('game_eq_v2_8_G', -180, 180, 180),
    'Game EQ Band 8 Frequency':      ('game_eq_v2_8_F', 20, 20000, 4000),
    'Game EQ Band 8 Q':              ('game_eq_v2_8_Q', 50, 1600, 42),
    'Game EQ Band 9 Switch':         ('game_eq_v2_9_ONF', 0, 1, 1),
    'Game EQ Band 9 Gain':           ('game_eq_v2_9_G', -180, 180, 180),
    'Game EQ Band 9 Frequency':      ('game_eq_v2_9_F', 20, 20000, 8000),
    'Game EQ Band 9 Q':              ('game_eq_v2_9_Q', 50, 1600, 42),
    'Game EQ Band 10 Switch':        ('game_eq_v2_10_ONF', 0, 1, 1),
    'Game EQ Band 10 Gain':          ('game_eq_v2_10_G', -180, 180, 180),
    'Game EQ Band 10 Frequency':     ('game_eq_v2_10_F', 500, 20000, 16000),
    'Game EQ Band 10 Q':             ('game_eq_v2_10_Q', 50, 1600, 42),
}

by_name = {row['name']: row for row in rows}
missing = sorted(set(EXPECTED) - set(by_name))
extra = sorted(set(by_name) - set(EXPECTED))
assert not missing, f'controls promised but absent from the table: {missing}'
assert not extra, f'controls absent from the expectations: {extra}'

seen = {}
for name, (device, vmin, vmax, vdef) in EXPECTED.items():
    row = by_name[name]
    expected_addr = next(addr for addr, dev in params.items() if dev == device)
    assert row['addr'] == expected_addr, (
        f'{name}: address 0x{row["addr"]:03x} but {device} lives at '
        f'0x{expected_addr:03x} (docs/DEVICE_PARAMETER_MAP.md)')
    assert (row['min'], row['max'], row['def']) == (vmin, vmax, vdef), (
        f'{name}: min/max/default {row["min"]}/{row["max"]}/{row["def"]} '
        f'but the vendor model says {vmin}/{vmax}/{vdef}')
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

printf 'control address contract passed\n'
