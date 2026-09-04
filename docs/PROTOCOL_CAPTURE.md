# ZG01 control-protocol capture

The driver already transports Game Out, Voice Out, and Voice In audio.
Hardware knobs, buttons, routing, and DSP remain proprietary. Capture
one deliberate action at a time on Windows so those controls can be implemented
from observed behavior rather than guessed USB messages.

Do not capture or redistribute firmware updates. Close unrelated audio and MIDI
applications, and do not disconnect the ZG01 while its firmware is updating.

## Install the capture tools

Install the current Yamaha ZG Controller, Wireshark, and USBPcap on native
Windows. During Wireshark installation, enable USBPcap. Reboot if requested.

Identify the ZG01 by USB ID `0499:1513`. Capture on the USBPcap interface
containing it and use this Wireshark display filter:

```text
usb.idVendor == 0x0499 && usb.idProduct == 0x1513
```

Keep an untouched baseline capture containing: connect the ZG01, start ZG
Controller, wait five seconds, then close it.

## Capture matrix

Start each trace from the same saved preset. Wait one second, perform exactly
one action, wait another second, and stop the capture.

| File name | Action |
|---|---|
| `00-baseline.pcapng` | Open and close the controller; change nothing |
| `10-volume-up-one.pcapng` | Main volume one repeatable increment up |
| `11-volume-down-one.pcapng` | Main volume one increment down |
| `20-mic-gain-up-one.pcapng` | Mic gain one increment up |
| `30-mic-mute-on.pcapng` | Toggle mic mute off to on |
| `31-mic-mute-off.pcapng` | Toggle mic mute on to off |
| `40-game-voice-left.pcapng` | Game/voice balance one increment left |
| `41-game-voice-right.pcapng` | Game/voice balance one increment right |
| `50-effect-on.pcapng` | Enable exactly one named effect |
| `51-effect-off.pcapng` | Disable that same effect |

Add one pair for every remaining button and one trace per routing/preset change.
Record starting and ending UI values in a neighboring `NOTES.md`. Never put
personal audio in a trace intended for publication.

## Export control transfers

With Wireshark's `tshark.exe` directory in PowerShell's `PATH`, export a stable
tab-separated form. Replace the input and output names for each capture:

```powershell
tshark -r 10-volume-up-one.pcapng `
  -Y "usb.idVendor == 0x0499 && usb.idProduct == 0x1513 && usb.setup.bRequest" `
  -T fields -E separator=/t -E occurrence=f `
  -e frame.number -e frame.time_relative -e usb.bus_id `
  -e usb.idVendor -e usb.idProduct -e usb.setup.bmRequestType `
  -e usb.setup.bRequest -e usb.setup.wValue -e usb.setup.wIndex `
  -e usb.setup.wLength -e usb.capdata |
  Out-File -Encoding utf8 10-volume-up-one.tsv
```

The normalizer accepts UTF-8 from PowerShell 7 and UTF-16 with a byte-order
mark from Windows PowerShell 5.1. The explicit UTF-8 writer above keeps files
portable before they leave Windows.

`length` in normalized output is USB's requested maximum (`wLength`), while
`actual_length` is the payload present on that record. A smaller actual value is
a valid short response; an empty value can be the setup half of a split IN
transfer and must be correlated with its completion during analysis.

Normalize the TSV on Linux into diff-friendly JSON Lines:

```bash
python3 tools/normalize-usbpcap.py 10-volume-up-one.tsv > 10-volume-up-one.jsonl
```

Commit small sanitized JSONL traces and notes only after inspecting them. Keep
raw captures outside Git until their contents and redistribution safety have
been reviewed.

## Analysis order

1. Subtract baseline/open-close traffic from every action trace.
2. Compare up/down and on/off pairs for request, value, index, length, and byte
   offsets that change.
3. Repeat a suspected action at three known UI values to distinguish absolute
   values from relative increments.
4. Replay nothing on Linux until a message's direction, bounds, and expected
   response are understood.
5. Add the decoded message as a fixture and unit test before exposing it through
   ALSA, the future `zgctl` CLI, or the GUI.
