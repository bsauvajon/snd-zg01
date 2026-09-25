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

## Save (`Save to ZG01`, EP 0x03)

```
04 f0 43 30 | 04 3e 14 03 | 04 02 01 00 | 07 00 01 f7 | 00...
```

Present exactly once in the two captures where `Save to ZG01` was
pressed, absent otherwise. Settings apply live; Save only persists them.

## Reads

At screen open the app issues read requests on EP `0x03`:

- `04 f0 43 20 | 04 3e 14 01 | 04 01 00 00 | 04 00 00 <id> | 05 f7`
- `04 f0 43 30 | 04 3e 14 01 | 04 01 00 02 | 04 <id> 00 00 | 06 00 f7`
  (one per id, e.g. `0x08..0x0b`)

The device answers on EP `0x83` with frames that echo the query and
carry the value, e.g. for id `0x08`:

```
04 f0 43 10 | 04 3e 14 01 | 04 01 00 02 | 04 08 00 00 | 04 00 00 00 | 04 00 00 00 | 05 f7 00 00
```

On a full open the device also pushes a dense state dump (several
512-byte `0x83` frames) listing many parameter ids and values; that is
the most complete read source but is not decoded yet.

## Other observed frames

- `04 f0 43 00 | 04 3e ...` carries ASCII strings split across words
  (`_Preset Voice`, `Soprano voice`, `Tenor voice`, `_Monitor Eq`).
- `04 f0 43 10 | 04 3e 14 00 | 04 70 02 00 | ...` carries version/identity
  strings (`BOOT`, `V1.0`, `MAIN`, `ICE`).
- The 192-byte `0x83` frames (`04 f0 43 10 | 04 3e 14 02 | ...`, ~50 ms)
  are level meters, not parameters.

## Open questions

- Checksum/terminator semantics (the trailing `f7`, and why type is
  `0x05` for parameter writes but `0x07` for save/keepalive).
- EQ value id and EQ band addressing (likely ids `0x11-0x20`).
- GATE/COMP sub-parameter ids (attack, release, ratio, knee, ...).
- Whether a read is required before a write, or writes are independent.
- The earlier GATE drag reached `0x69` (105) while the UI range appears
  to be 0-100; confirm the upper bound.
