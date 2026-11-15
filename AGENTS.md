# AGENTS.md

Out-of-tree Linux kernel driver for the Yamaha ZG01 USB audio interface
(`0499:1513`). The device is vendor-specific, not USB Audio Class: packet
formats and the control protocol in `docs/` were reverse-engineered from USB
captures, so do not assume standard UAC behavior.

## Commands

- `make` — build `snd-zg01.ko` from the repository root. `src/` has no
  Makefile; `make -C src` fails.
- `make KDIR=/lib/modules/<ver>/build` — build against another kernel.
- `make -C tools` — build `tools/zgctl`. It is intentionally excluded from
  `all` because kbuild drives that target for the module.
- `make clean` — clean both the module and the CLI.
- `tests/test-kernel-build.sh` — strict gate: `W=1`, fails on any
  warning/error attributed to a `.c`/`.h` in this repo, and requires
  `vermagic` to match the running kernel.
- `bash tests/test-capture-normalizer.sh` and
  `bash tests/test-usbmon-normalizer.sh` — python3 only, no hardware.
- `tests/test-arch-packaging.sh` — needs `makepkg`, `dkms` and kernel headers;
  set `KERNEL_VERSION=<ver>` when the headers differ from `uname -r`.
  `tests/test-arch-zgctl-packaging.sh` degrades to a static PKGBUILD check
  when `makepkg` is absent.
- `tests/test-zg01-lifecycle.sh --cycles N` — requires a real ZG01, the UCM
  profile installed, and a running PipeWire (`wpctl`, `pactl`, `jq`,
  `pw-play`, `pw-record`); not CI-safe.

Clang-built kernels: the Makefile detects `CONFIG_CC_IS_CLANG` in the target
kernel and sets `LLVM=1`. Do not hardcode `LLVM=1`; a user-supplied `LLVM=`
value wins.

## Load and test on hardware

Rebuild, then `sudo modprobe -r snd-zg01 && sudo insmod ./snd-zg01.ko`.
`modprobe snd-zg01` uses the DKMS-installed copy under `/lib/modules`, so it
can run stale code after a local build. Module name uses underscores
(`snd_zg01` in `lsmod`), the file uses a hyphen.

The module must bind before the generic `snd-usb-audio` match; `src/snd-zg01.conf`
(installed to modules-load.d) handles this at boot, but a manual build gets no
mechanism.

## Architecture

`obj-m := snd-zg01.o` links the objects from `src/`: one module, one ALSA
card (`zg01`), four PCM devices, all 48 kHz S32_LE stereo:

| PCM | Name | Direction |
|---|---|---|
| 0 | Game Out | playback |
| 1 | Voice Out | playback, shares EP 0x01 |
| 2 | Voice In | capture, mic pair of EP 0x81 |
| 3 | Streaming In | capture, second pair of EP 0x81 |

- `src/zg01_usb.c` — probe anchored on interface 1 (returns 0 for others),
  claims interfaces 3 and 4 so `snd-usb-audio` cannot, card creation,
  suspend/resume.
- `src/zg01_pcm.c` — PCM ops, `out_chain`/`in_chain` URB chains, the
  shared-endpoint data path, vendor handshake and rate setup.
- `src/zg01_control.c` — ALSA mixer controls over the interface-4 bulk
  protocol (EP 0x03/0x83, 512-byte frames).
- `src/zg01_usb_discovery.c` — debug descriptor dump, best effort.
- `src/zg01.h` — structs, chain state, packet and parameter constants;
  `src/zg01_feedback.h` — implicit-feedback helpers.

Both playback PCMs share one out chain. The URB callback mixes their frames
into each packet; when only Voice Out runs the chain sends keepalive silence.
Both capture PCMs share one in chain: every 16-byte IN frame carries four
32-bit channels, the microphone pair (Voice In, byte 0) and the device's
streaming-mix pair (Streaming In, byte 8), and the callback copies each
running consumer's pair into its own ring. The IN stream also paces playback
(implicit feedback). The UCM profile in `ucm/conf.d/zg01_usb/` is what makes
PipeWire expose Game Out, Voice Out, Voice In and Streaming In separately;
without it only the generic stereo sink exists.

## Pitfalls

- `runtime->buffer_size` is in frames, not bytes. Do not divide by
  `bytes_per_frame`.
- The vendor "Magic Sequence" resets both interfaces to alt 0 and kills live
  URBs. Run it only from `prepare`, only while both chains are stopped and
  cleaned up, under `state_mutex`. Keep interface setup out of `open`.
- USB control buffers must be heap-allocated, never on the stack.
- `-ECONNRESET` bursts on the first Voice In open are benign; the chain
  resubmits. Do not treat them as a stop condition.
- `struct zg01_dev` is `snd_card_new` private data and is released by
  `card->private_free`; never `kfree` it yourself.
- No `EXPORT_SYMBOL` for intra-module symbols.
- Control getters return the driver's cached last-written value, not device
  state. This is documented behavior; do not "fix" it by reading back.
- New protocol work needs a capture first: follow `docs/PROTOCOL_CAPTURE.md`.
  Raw captures live in `capture/` and `captures/`, both gitignored; never
  commit them.

## Packaging and release

- Debian: `./scripts/build-deb.sh` (needs `debhelper`, `devscripts`,
  `dh-dkms`, `dkms`). `debian/rules` builds only `zgctl`; DKMS compiles the
  module at install time. Version bumps touch `PACKAGE_VERSION` in `dkms.conf`
  and `dch -v` in `debian/changelog`.
- Arch: `packaging/arch` (`snd-zg01-dkms-git`) and `packaging/arch-zgctl`
  (`zgctl-git`). After editing `packaging/arch/PKGBUILD`, regenerate
  `packaging/arch/.SRCINFO` (`makepkg --printsrcinfo > .SRCINFO`): the
  contract test byte-compares it.
- CI (`.github/workflows/ci.yml`) builds `W=1` against Ubuntu's
  `linux-headers-generic` (the runner's Azure headers lack ALSA symbols),
  runs the normalizer tests, builds `zgctl`, and runs the Arch packaging
  container job.
- Release: push a `v*` tag; `release-apt.yml` builds the Debian and Arch
  packages and publishes them plus a signed APT repository as release assets.

## References

- `README.md` — feature/status and user-facing behavior
- `docs/INITIALIZATION_ANALYSIS.md` — USB topology and packet formats
- `docs/MIC_CONTROL_PROTOCOL.md` — decoded mic parameter protocol
- `docs/superpowers/specs/` and `plans/` — the Rust/GTK4 `zg01-control`
  GUI (`gui/`, shipped as `snd-zg01-gui` / `zg01-control-git`); read the
  newest spec/plan pair before changing it
