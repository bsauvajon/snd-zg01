# ZG01 mixer control protocol (reverse engineered)

Decoded from libvirt + usbmon captures on the MIC SETTINGS screen of ZG
Controller (see `captures/`, normalized with `tools/normalize-usbmon.py`).
This describes the device protocol, not the driver.

Status: the mic parameter **write** and **save** paths are decoded; the
read path is partially decoded. EQ and the remaining sub-parameters are
not mapped yet.

## Transports

Two USB paths are involved:

- **EP0 vendor requests** for enumeration/status only (`0xc0 bRequest 7`
  -> `80bb00`, `0xc0 bRequest 4` -> 1-byte poll, `0xc0 bRequest 6` ->
  UTF-16 channel names).
- **Interface 4 bulk** for every mixer parameter:
  - host -> device on EP `0x03`
  - device -> host on EP `0x83`
  - 512-byte frames (some device frames are shorter).

## Frame format

A frame is a sequence of 4-byte words, zero padded to 512 bytes. The
first byte of a word is a type:

- `0x04` data word: `04 <b1> <b2> <b3>`
- `0x05`/`0x07` end word (type varies with message class)
- `0x00` padding (all-zero word)

Parameter IDs live in the first data byte of a word.

## Parameter write (EP 0x03)

```
04 f0 43 10 | 04 3e 14 01 | 04 01 00 00 | 04 <id> 00 00 | 04 00 00 00 | 04 00 <val_hi> <val_lo> | 05 f7 00 00 | 00...
```

`<val_hi>:<val_lo>` is the parameter value, 16-bit big-endian-ish (the
byte order was not distinguishable on the tested values, all < 256).
Value scale observed on GATE/COMP/LIMITER: equals the number shown in
the UI (`0`, `50`, `100` -> `0x0000`, `0x0032`, `0x0064`).

## Parameter IDs

| Block | Enable | Value |
|---|---|---|
| GATE | `0x02` | `0x03` |
| COMP | `0x09` | `0x0a` |
| EQ | `0x11` | unknown |
| LIMITER | `0x21` | `0x22` |

Enable writes use value `0`/`1`. The gaps suggest per-block id ranges:
GATE `0x02-0x08`, COMP `0x09-0x10`, EQ `0x11-0x20`, LIMITER `0x21+`.
Those correspond to the sub-parameters of each block (attack, release,
ratio, EQ bands, ...) and are still unmapped.

### EQ ids

EQ edits reach the card live (no `Save to ZG01` needed). They use the
same write frame but with `word2 = 04 01 00 02`:

```
04 f0 43 10 | 04 3e 14 01 | 04 01 00 02 | 04 <id> 00 00 | 04 00 00 00 | 04 00 <val> | 05 f7
```

Observed ids:

| Id | Meaning | Values seen |
|---|---|---|
| `0x24` | Low Shape type | `0`, `1` |
| `0x27` | High Shape type | `0`, `1` |
| `0x28` | band 1 gain | `0`..`616` |
| `0x29` | band 2 gain | `0`..`616` |
| `0x2a` | band 3 gain | `0`..`616` |
| `0x2b` | band 4 gain | `0`..`616` |
| `0x2c` | band 1 frequency | `20`, `1896` |
| `0x30` | Q (band still to confirm) | `0`, `48` |

Gain is linear in dB: `0 dB -> 308`, `+18 dB -> 616`, `-18 dB -> 0`,
so `value = 308 + 17.111 * dB` (range `0`..`616` for +/-18 dB).

Q is logarithmic: `0.5 -> 0`, `1 -> 12`, `2 -> 24`, `4 -> 36`, `8 -> 48`,
so `value = 12 * (log2(Q) + 1)`, i.e. 12 units per octave, band Q from
0.5 to 8.

Frequency is piecewise but not a clean `2f`: the app's own values show
a small, non-uniform deficit below the 16 kHz boundary, e.g. `1 kHz ->
1896` (not 2000).  Measured on band 2 (id `0x2d`):

| Hz | 50 | 100 | 200 | 500 | 1000 | 2000 | 5000 | 10000 | 15000 | 17000 | 20000 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| raw | 50 | 100 | 328 | 884 | 1896 | 3920 | 9992 | 19984 | 29976 | 66664 | 72736 |

The +32768 offset appears for `f >= 16384` (16384 = 2^14, where `2f`
reaches 2^15), so the driver interpolates this measured table rather
than a formula.  Band ranges: band 1 20 Hz-1 kHz, bands 2 and 3
20 Hz-20 kHz, band 4 500 Hz-20 kHz; the underlying scale is shared.

Shape: `0x24` is the low-frequency shelf type and `0x27` the
high-frequency shelf type; in shelf mode the band Q is hidden (band 1 for
low shape, band 4 for high shape).

Id layout: gains `0x28`-`0x2b` (bands 1-4), frequencies `0x2c`-`0x2f`,
Q `0x30`-`0x33`.

The companion `04 f0 43 00` preset block that the app sends after each
OK stores the band-1 gain as `0x0168` (`360`), a different (0.1 dB)
scale than the live write, and truncates the frequency value to its low
bits; the live write is what the card accepts.

After each EQ OK the app also sends the full `04 f0 43 00` preset block
and an apply command `04 f0 43 30 | 04 3e 14 03 | 04 02 06 00 | 07 00 01 f7`.


## Save (`Save to ZG01`, EP 0x03)

```
04 f0 43 30 | 04 3e 14 03 | 04 02 01 00 | 07 00 01 f7 | 00...
```

Present exactly once in the two captures where `Save to ZG01` was
pressed, absent otherwise. Settings apply live; Save only persists them.

## Reset / reload (`RESET`, EP 0x03)

```
04 f0 43 30 | 04 3e 14 03 | 04 02 04 00 | 07 00 01 f7 | 00...
```

Same shape as save, with code `0x04` instead of `0x01`. Observed once,
right after disabling the GATE and pressing `RESET`; the app then
re-reads the whole state (type-20 request plus type-30 per-id reads).
It reloads the persisted settings, dropping unsaved edits.

## Reads

The parameter write frame ends with a terminator word of type `0x05`
(`05 f7 00 00`); a `0x04` there makes the firmware ignore the frame.
The app also sends a keepalive about once a second:

```
04 f0 43 10 | 04 3e 14 00 | 07 04 00 f7
```

At screen open the app issues read requests on EP `0x03`:

- `04 f0 43 20 | 04 3e 14 01 | 04 01 00 00 | 04 00 00 <id> | 05 f7`
- `04 f0 43 30 | 04 3e 14 01 | 04 01 00 02 | 04 <id> 00 00 | 06 00 f7`
  (one per id, e.g. `0x08..0x0b`)

The `04 f0 43 30` form is answered on EP `0x83` with a fixed status, not
the parameter value; e.g. for id `0x08` it returned both `04 00 00 00`
and `04 00 00 01`, and for the mic LIMITER id `0x22` it always returns
`04 00 00 01` regardless of the value written.  The mic value space is
therefore not readable through this request.

The `04 f0 43 20` form makes the device push a **full state dump** on EP
`0x83`: several 512-byte frames of parameter descriptors (counts, min/max,
defaults and names such as `_Mic Eq`).  That dump is the real read source;
it is not decoded yet.

## Other observed frames

- `04 f0 43 00 | 04 3e ...` carries ASCII strings split across words
  (`_Preset Voice`, `Soprano voice`, `Tenor voice`, `_Monitor Eq`).
- `04 f0 43 10 | 04 3e 14 00 | 04 70 02 00 | ...` carries version/identity
  strings (`BOOT`, `V1.0`, `MAIN`, `ICE`).
- The 192-byte `0x83` frames (`04 f0 43 10 | 04 3e 14 02 | ...`, ~50 ms)
  are level meters, not parameters.

## Open questions

- Meaning of the trailing `f7`, and why the terminator type is `0x05`
  for parameter writes but `0x07` for save/keepalive.
- The full 512-byte state dump layout (the read source for values).
- GATE/COMP sub-parameter ids (attack, release, ratio, knee, ...).
- Whether a keepalive or an initial state read is required before the
  firmware accepts writes.
- The state dump that the type-20 request triggers (the read source for
  values) is not decoded yet.
- MIC EQ values: the earlier GATE drag reached `0x69` (105), but
  controlled writes at 0/50/100 gave 0/0x32/0x64, so the scale is 0-100
  and the 105 was a transient drag value.
