# Vendor parameter model (TOOLS for ZG)

Notes on the Windows "TOOLS for ZG" package, why it is in this repository's
documentation, how to extract the parameter model from it, and what is
still not known.  The package itself is **not** vendored here.

## What the package is

`TOOLS_for_ZG_V200_Win.zip` (~173 MB) is Yamaha's Windows package for the
ZG series: the Yamaha Steinberg USB driver plus **ZG Controller V2.0.0**,
the application this project reverse engineers for its Linux driver.

It contains, after unpacking:

| Path | Content |
|---|---|
| `Install_ZG.exe` | Inno Setup 6.1 installer |
| `tmp/Yamaha ZG Application.msi` | MSI database (file names, components) |
| `tmp/YmhZGApp.cab` | payload: `Yamaha ZG Controller.exe` (Qt 6 QML application), `MemAPI_ZG01.dll`, `MemAPI_ZG02.dll`, Qt runtime, 26 factory `*.zgpreset` files |

Yamaha serves it from `https://data.yamaha.com/files/download/software/6/2179276/TOOLS_for_ZG_V200_Win.zip`
(the support page may ask for an account depending on the region; the file
itself is served without one).  Check the size and keep the archive out of
Git.

## Why it matters for the driver

ZG Controller is the *same client* as the one we captured on USB, so the
package does not contain the wire bytes - those still come from the
captures (`docs/PROTOCOL_CAPTURE.md`).  What it does contain, and what the
captures cannot give, is the **parameter model**:

- parameter names, which label the ids seen on the wire;
- value bounds and defaults, which decide the unit and the range of every
  control (0.1 dB for gains with 0 dB at 180, Hz for frequencies, 12 units
  per octave for Q, 0..100 for the gate/compressor/limiter levels);
- the four-band structure of the MIC EQ and its per-band frequency ranges,
  which is what pins the EQ id groups to bands 1-4;
- the sub-parameters the driver does not expose yet (attack, release,
  ratio, knee, hold, ...), with their ranges, ready to be labelled in a
  future capture.

`MemAPI_ZG01.dll` is the library that implements the device parameter API
(exports `queryMemAPI`, `queryMemAPIGUID`, `queryVenderProductID`); it
carries one 56-byte descriptor per parameter, each holding a pointer to the
parameter's name plus its bounds and default.

## Extracting the parameter model

No Windows and no root are needed:

```bash
unzip TOOLS_for_ZG_V200_Win.zip                     # -> Install_ZG.exe
innoextract -e Install_ZG.exe                       # -> tmp/*.msi, tmp/*.cab
cabextract -d app 'tmp/YmhZGApp.cab'                # -> app/MemAPI_ZG01.dll
msiinfo export 'tmp/Yamaha ZG Application.msi' File  # real file names

tools/vendor/extract-memapi-params.py app/MemAPI_ZG01.dll params.json
```

`innoextract` and `cabextract` can be installed without root (for example
`apt-get download innoextract cabextract …` then `dpkg-deb -x` into a
private sysroot); innoextract 1.9 reads the Inno Setup 6.1 data with a
version warning.  `cabextract` names the payload after the MSI file keys,
so `msiinfo export … File` is what maps `ZG_IconId.exe` to `Yamaha ZG
Controller.exe`.

The extractor reports 1102 parameters for the ZG01 and 538 for the ZG02
(whose names are snake_case, e.g. `mic_eq_lo_g`).

## MIC parameters (ZG01)

As extracted by the tool; the wire id is the one observed in the captures
(`docs/MIC_CONTROL_PROTOCOL.md`).

| Vendor name | Range | Default | Wire id |
|---|---|---|---|
| `MicGateOnf` | 0..1 | 0 | `0x02` |
| `MicGateLevel` | 0..100 | 50 | `0x03` |
| `MicGateThreshold` | 1..73 | 23 | ? |
| `MicGateRange` | 0..73 | 17 | ? |
| `MicGateAttack` | 57..283 | 237 | ? |
| `MicGateHold` | 0..213 | 101 | ? |
| `MicGateDecay` | 24..300 | 188 | ? |
| `MicCompOnf` | 0..1 | 0 | `0x09` |
| `MicCompLevel` | 0..100 | 50 | `0x0a` |
| `MicCompThreshold` | 19..73 | 52 | ? |
| `MicCompRatio` | 0..120 | 14 | ? |
| `MicCompAttack` | 57..283 | 237 | ? |
| `MicCompRelease` | 24..300 | 228 | ? |
| `MicCompKnee` | 0..2 | 1 | ? |
| `MicCompOutgain` | 0..180 | 50 | ? |
| `MicEqOnf` | 0..1 | 0 | `0x11` |
| `MicEqV21F/G/Q/Shape/Onf` | 20..1000 Hz / 0..360 / 0..60 / 0..1 / 0..1 | 100 / 180 / 12 / 1 / 1 | `0x2c` / `0x28` / `0x30` / `0x24` / ? |
| `MicEqV22F/G/Q/Shape/Onf` | 20..20000 Hz / 0..360 / 0..60 / 0..1 / 0..1 | 500 / 180 / 12 / 0 / 1 | `0x2d` / `0x29` / `0x31` / ? / ? |
| `MicEqV23F/G/Q/Shape/Onf` | 20..20000 Hz / 0..360 / 0..60 / 0..1 / 0..1 | 2500 / 180 / 12 / 0 / 1 | `0x2e` / `0x2a` / `0x32` / ? / ? |
| `MicEqV24F/G/Q/Shape/Onf` | 500..20000 Hz / 0..360 / 0..60 / 0..1 / 0..1 | 10000 / 180 / 12 / 1 / 1 | `0x2f` / `0x2b` / `0x33` / `0x27` / ? |
| `MicLimiterOnf` | 0..1 | 1 | `0x21` |
| `MicLimiterLevel` | 0..100 | 50 | `0x22` |
| `MicLimiterThreshold` | 19..73 | 65 | ? |
| `MicLimiterAttack` | 57..283 | 237 | ? |
| `MicLimiterRelease` | 24..300 | 228 | ? |
| `MicLimiterKnee` | 0..2 | 1 | ? |
| `MicLimiterOutgain` | 0..180 | 0 | ? |

Reading notes:

- gains live in 0.1 dB: 0 dB is 180, so the range is -18.0..+18.0 dB;
- the EQ band frequency ranges (20-1k, 20-20k, 20-20k, 500-20k) are the
  ones the captures show on ids `0x2c`-`0x2f`, which is what fixes the
  band order;
- Q is 12 units per octave: 0.5, 1 and 16 map to 0, 12 and 60;
- `MicEqV21Onf`..`V24Onf` (per-band enable) and `MicGateRange` etc. have
  no identified id yet: the driver only writes the ids listed above.

The ZG01 library also carries a second, index-based MIC EQ family
(`MicEqLoFreq` 4..72, `MicEqMdFreq` 4..124, `MicEqHiFreq` 60..124, with
`MicEqLoGain`/…), whose bounds match a 1/12-octave index grid anchored at
20 Hz (index 4 = 20 Hz, 32 = 100 Hz, 60 = 500 Hz, 112 = 10 kHz,
124 = 20 kHz).  It was **not** tied to the captured wire ids - the
captured frequency bytes decode to plain Hz, which matches the `V21`-`V24`
family - so it is left aside here.

Several EQ families coexist for a reason: the ZG Controller EQ editor
takes a `bandNums` property of 4 or 10 and drives every band through a
`YQParameterAccessor` that carries a `paramID`, while the package ships
both a ZG01 and a ZG02 library (the ZG02 one is snake_case, e.g.
`mic_eq_lo_g`).  A family has to be matched to the device and the screen
before its ids are trusted, which is what the captured ranges did for the
four-band MIC EQ.

## Caveats

- The descriptor slots were decoded by structure (name pointer at offset
  40 of a 56-byte record); that max/min/default are slots 4/5/6 is an
  inference, corroborated by the captured bytes for the EQ gains and
  frequencies and by the raw levels for GATE/COMP/LIMITER.  Treat a value
  that contradicts a capture as an open question, not as the truth.
- Yamaha's EULA forbids reverse engineering and redistribution of the
  software, while interoperability analysis of a purchased device is what
  this project is about.  Keep the vendor archive, the extracted payload
  and the `*.zgpreset` files out of Git (see `.gitignore`); commit derived
  facts only, as this document does.
- The factory `*.zgpreset` files (MIC EQ presets "Dynamic Mic.",
  "Condenser Mic.", "Headset Mic.", monitor EQ, game effect presets) are
  Yamaha's MBDF format, chunked and bit-packed.  They were not decoded;
  they are a promising source of ground truth for the preset block that
  the application sends after every EQ write.
