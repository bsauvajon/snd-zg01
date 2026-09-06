# ZG01 USB device notes

Notes from Windows USB captures. They describe the device, not the driver.
The driver no longer matches every detail here; read `src/zg01.h` for the
current implementation.

## Device information

- Vendor ID: 0x0499 (Yamaha)
- Product ID: 0x1513 (ZG01)
- USB class: vendor specific (0xef) with an interface association descriptor
- bcdUSB: 0x0210 (USB 2.1)
- bcdDevice: 0x0150

## Configuration descriptor

The device exposes five interfaces:

| Interface | Class bytes | Purpose | Endpoints |
|---|---|---|---|
| 0 | 0xff 0x01 0x20 | Control | none |
| 1 | 0xff 0x02 0x20 | Playback streaming | alt 1: ISO OUT 0x01 (280 bytes) |
| 2 | 0xff 0x02 0x20 | Capture streaming | alt 1: ISO IN 0x81, interrupt IN 0x84 |
| 3 | 0xff 0x03 0xff | MIDI | bulk OUT 0x02, bulk IN 0x82 (512 bytes) |
| 4 | 0xff 0xff 0xff | Additional bulk | bulk OUT 0x03, bulk IN 0x83 (512 bytes) |

Alt setting 0 on interfaces 1 and 2 has no endpoints. The driver parks both
interfaces there before it registers the PCM devices.

## Initialization sequence

Observed on Windows:

1. Standard enumeration and `SET_CONFIGURATION(1)`.
2. Vendor control request:

   ```
   bmRequestType: 0xc0   (vendor, device-to-host)
   bRequest:      7
   wValue:        0x0000
   wIndex:        0
   response:      0x80 0xbb 0x00 (3 bytes)
   ```

3. `SET_INTERFACE(1, 0)` and `SET_INTERFACE(2, 0)` (streaming disabled).
4. `SET_INTERFACE(1, 1)` and `SET_INTERFACE(2, 1)` when a stream starts.

`zg01_control.c` sends the vendor request once per card and checks the
0x80bb00 response. The device does not use standard USB Audio Class
descriptors; the packet formats below are reverse engineered.

## Audio packet formats

- Interface 1 (playback, EP 0x01): 240-byte packets, 6 frames of 40 bytes.
  Frame: Voice_L(4), Voice_R(4), Game_L(4), Game_R(4), then 24 pad bytes.
- Interface 2 (capture, EP 0x81): 108-byte packets.
  Header 8 bytes (counter + 0x60000000 marker), 6 frames of 16 bytes
  (L 4, R 4, pad 8), trailer 4 bytes (counter repeat).

The 40-byte playback frame carries both sinks on one endpoint. The driver
writes the Game and Voice samples into separate frame slots, which is why the
driver exposes two playback PCM devices on the shared endpoint instead of
mixing in userspace.

## Unused hardware

- Interfaces 3 and 4 (MIDI and bulk) are not implemented.
- The interrupt IN endpoint 0x84 is not implemented.
- The clock validity read returns -EAGAIN on some hosts, so the driver
  does not use it.

## Further work

See `docs/PROTOCOL_CAPTURE.md` for the capture workflow that documents how
to record new control messages for knobs, buttons, and routing.
