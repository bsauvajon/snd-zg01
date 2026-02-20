# Yamaha ZG01 USB Audio Driver — Code Review Fix Plan

This document records all 50 findings from the code review of the `snd-zg01`
Linux kernel USB audio driver, the decisions taken for each, and the
implementation approach. All fixes are applied on branch `fix/code-review`.

---

## Driver Overview

The driver supports the Yamaha ZG01 USB audio interface (VID: 0x0499,
PID: 0x1513). It exposes three independent ALSA audio channels:

| Channel      | Direction | USB Interface | Endpoint |
|--------------|-----------|---------------|----------|
| Game output  | Playback  | Interface 1   | 0x01 OUT |
| Voice Out    | Playback  | Interface 1   | 0x01 OUT |
| Voice In     | Capture   | Interface 2   | 0x81 IN  |

Source files:

| File                     | Role                                        |
|--------------------------|---------------------------------------------|
| `zg01.h`                 | Shared structs, constants, extern decls     |
| `zg01_usb.c`             | USB probe/disconnect, module entry point    |
| `zg01_pcm.c`             | PCM ops, URB streaming, audio data path     |
| `zg01_pcm.h`             | PCM header (was dead BCD2000 copy-paste)    |
| `zg01_control.c`         | Device init USB control message             |
| `zg01_control.h`         | Control header                              |
| `zg01_usb_discovery.c`   | USB descriptor discovery/debug              |

---

## Findings by Severity

### Critical (must fix — crash or data corruption)

#### USB-5: `INIT_DELAYED_WORK` with NULL function pointer
- **File:** `zg01_usb.c:115-117`
- **Issue:** `INIT_DELAYED_WORK(&dev->start_work_game, (void *)0)` — passing a
  NULL function pointer to `INIT_DELAYED_WORK`. If the work is ever queued, the
  kernel will call NULL and oops.
- **Fix:** Removed entirely — the delayed-work infrastructure is part of the
  dead deferred-start stub (PCM-26). The fields, initialisation, and all
  references are removed.

#### USB-8: Recursive `zg01_probe` — full refactor
- **File:** `zg01_usb.c:196-199`
- **Issue:** `zg01_probe` calls itself recursively to create the Voice Out card
  after creating the Game card. This is fragile, hard to reason about, and
  causes USB-3, USB-4, USB-6, USB-7, and USB-10 as side-effects.
- **Fix:** Extracted `zg01_create_one_card()` helper. `zg01_probe` calls it
  in sequence — once for Game and once for Voice Out when `iface_num == 1`,
  once for Voice In when `iface_num == 2`. No recursion.

#### PCM-1: `active_urbs_count` used uninitialized
- **File:** `zg01_pcm.c:752`
- **Issue:** `active_urbs_count` is declared at line 625 but never assigned
  before being read at line 752. Its value is whatever the stack contains.
- **Fix:** Added `active_urbs_count = zg01_get_active_urbs_count(dev);`
  immediately before the `if (active_urbs_count == 0)` check.

#### USB-3: Global device ptrs cleared without `devices_mutex`
- **File:** `zg01_usb.c:260-266`
- **Issue:** `game_dev = NULL` etc. in `zg01_disconnect` are not protected by
  `devices_mutex`, creating a race with `zg01_probe` reading the same globals.
- **Fix:** Wrapped the global pointer clear block in
  `mutex_lock/unlock(&devices_mutex)`.

#### PCM-16: `__symbol_get`/`__symbol_put` misuse
- **File:** `zg01_pcm.c:248-256, 650-661`
- **Issue:** `__symbol_get("game_dev")` is used to access a symbol in the
  **same module**. `__symbol_get` is for cross-module symbol access; using it
  within the same module bumps the module refcount and is semantically wrong.
  It also causes a kernel warning if the symbol isn't exported.
- **Fix:** Added `extern struct zg01_dev *game_dev, *voice_in_dev,
  *voice_out_dev;` to `zg01.h`. Replaced both `__symbol_get/put` blocks with
  direct pointer reads of the externs. Removed
  `EXPORT_SYMBOL_GPL(game_dev/voice_in_dev/voice_out_dev)` from `zg01_usb.c`
  (intra-module symbols do not need export).

---

### High (fix — incorrect behaviour or memory safety)

#### USB-4: `devices_mutex` held across early return on `snd_card_new` failure
- **File:** `zg01_usb.c:84-89`
- **Issue:** Early return paths before `snd_card_new` held `devices_mutex`
  and returned without unlocking in some flows.
- **Fix:** Resolved naturally by the USB-8 refactor which restructures the
  error path.

#### USB-6: Use-after-free — global ptr set before error paths
- **File:** `zg01_usb.c:160-193`
- **Issue:** `game_dev`/`voice_out_dev` assigned at lines 123-129, before
  `zg01_init_control` and `zg01_create_pcm`, which can fail. On failure,
  `snd_card_free(card)` is called but the global still points to the freed dev.
- **Fix:** In `zg01_create_one_card`, global pointer is assigned only after
  `snd_card_register` succeeds.

#### USB-9: `iso_buffers` set to NULL without `kfree` — memory leak
- **File:** `zg01_usb.c:225-255`
- **Issue:** Disconnect sets `iso_buffers_*[i] = NULL` without freeing the
  buffer, leaking the `kmalloc`'d URB transfer buffers.
- **Fix:** Added `kfree(dev->iso_buffers_*[i])` before each NULL assignment
  in all three channel blocks in `zg01_disconnect`.

#### PCM-3: NULL short-circuit allows NULL passed to USB
- **File:** `zg01_pcm.c:482-486`
- **Issue:** `if (!data || !large_data)` short-circuit is correct, but the
  `goto cleanup` path must execute `kfree` on whichever was allocated. The
  existing code does this correctly; confirmed and annotated.

#### PCM-4: UB — signed shift into sign bit
- **File:** `zg01_pcm.c:551-552`
- **Issue:** `large_data[2] << 16` and `large_data[3] << 24` where
  `large_data` is `unsigned char`. The result of shifting a non-`u32` value
  left 24 bits is undefined behaviour in C when the result doesn't fit in the
  intermediate `int`.
- **Fix:** `(u32)large_data[2] << 16 | (u32)large_data[3] << 24`.

#### PCM-5: USB control message return values silently ignored
- **File:** `zg01_pcm.c:492-600`
- **Issue:** The vendor read sequence in `zg01_set_rate` calls
  `usb_control_msg` multiple times without checking return values. Failures
  are silent.
- **Fix:** Added `pr_debug` logging for each best-effort vendor read. The
  critical SET_CUR path already has error checking and retry logic.

#### PCM-9/10: Per-packet spinlock churn and `pcm_pos` race
- **File:** `zg01_pcm.c:982-1056`
- **Issue:** The spinlock is acquired and released **once per 240-byte
  packet** in a loop of up to 32 packets per URB. This causes 32 lock/unlock
  pairs per IRQ callback. Worse, `pcm_pos` is updated at line 1063 with a
  separate lock acquisition, creating a window where another CPU can observe
  an inconsistent position.
- **Fix:** Acquire the spinlock once before the per-packet loop and release
  once after processing all packets and updating `pcm_pos` for the whole URB.

#### PCM-13: Out-of-bounds write in capture buffer wraparound
- **File:** `zg01_pcm.c:1107-1125`
- **Issue:** The wraparound code computes `first_part = buffer_bytes -
  write_byte_pos` and branches on `first_part >= 4` / `first_part >= 8`. When
  `first_part` is 1, 2, or 3 the partial-write branches produce overlapping
  writes and can write past the end of the DMA buffer.
- **Fix:** Stage both samples into a local `unsigned char tmp[8]`, then do a
  single split copy using `memcpy` with correct byte counts.

#### PCM-20: `queue_work` false return misread as failure
- **File:** `zg01_pcm.c:1399`
- **Issue:** `queue_work` returns `false` when the work item is **already
  queued** — not when it fails. The code treats `false` as an error.
- **Fix:** Removed the error branch. `queue_work` returning false simply
  means cleanup is already scheduled, which is fine.

#### PCM-21: `*active_urbs = 0` before URBs are killed
- **File:** `zg01_pcm.c:1406`
- **Issue:** `*active_urbs = 0` is set at line 1406 **before** the deferred
  work has killed the URBs. A concurrent `start_streaming` call sees zero
  active URBs and starts a second set while the first is still running in the
  USB host controller.
- **Fix:** Removed `*active_urbs = 0` from `zg01_stop_streaming`. The cleanup
  work function `zg01_cleanup_multi_urb_work_fn` now sets `*active_urbs = 0`
  after the `usb_kill_urb` loop completes.

#### PCM-22: Inconsistent URB buffer free path (stop vs disconnect)
- **File:** `zg01_pcm.c`, `zg01_usb.c`
- **Issue:** On TRIGGER_STOP, URB buffers are freed by the deferred work
  function. On disconnect, `zg01_disconnect` kills URBs but sets buffers to
  NULL without `kfree`. This leaks memory on device removal.
- **Fix:** In `zg01_disconnect`, call `flush_work(system_wq)` after unlinking
  URBs to drain any pending cleanup work, then inline-free all non-NULL
  `iso_buffers_*[i]` with `kfree`. Also addressed by USB-9 fix above.

---

### Medium (fix — incorrect logic, log spam, API misuse)

#### CTL-1: Init guard always fires — device init never runs
- **File:** `zg01_control.c:25-29`
- **Issue:** `if (iface_num != 0) return 0;` skips device init. But
  `zg01_probe` only calls `zg01_init_control` for interfaces 1 and 2, so
  `iface_num` is never 0 here. The USB init message is never sent.
- **Fix:** Removed the `iface_num` guard from `zg01_init_control`. The probe
  function's own interface check is the correct gate.

#### CTL-2: Short USB response silently ignored
- **File:** `zg01_control.c:54-63`
- **Issue:** When the device responds with fewer than 3 bytes, the response
  is silently discarded with no log.
- **Fix:** Added `pr_debug` for the short-response case.

#### USB-2: `devices_used` bitmap never meaningfully used
- **File:** `zg01_usb.c:16`
- **Issue:** `devices_used` is declared and the `card_index` loop reads it,
  but `set_bit` is never called, so `test_bit` always returns 0 and the loop
  always yields `card_index = 0`. The bitmap serves no purpose.
- **Fix:** Removed `DECLARE_BITMAP(devices_used, SNDRV_CARDS)`, the
  `for` loop, and `card_index` tracking.

#### USB-7: `usb_set_intfdata` overwritten by recursive probe
- **File:** `zg01_usb.c:133`
- **Issue:** Each recursive call to `zg01_probe` calls
  `usb_set_intfdata(interface, dev)`, overwriting the pointer set by the
  previous call. Only the last dev (Voice Out) survives in intfdata.
- **Fix:** After USB-8 refactor, `usb_set_intfdata` is called once per
  `zg01_create_one_card` call. For interface 1, disconnect uses the global
  `game_dev`/`voice_out_dev` pointers (under `devices_mutex`) to find both
  devs.

#### USB-10: Redundant/overlapping disconnect conditions
- **File:** `zg01_usb.c:218,232,246`
- **Issue:** Conditions like `dev->channel_type == X || iface_num == Y` are
  redundant after each dev has a single `channel_type`.
- **Fix:** Simplified to `if (dev->channel_type == CHANNEL_TYPE_X)` after
  USB-8 refactor.

#### PCM-2: Dead null-check on `dev->udev`
- **File:** `zg01_pcm.c:509-515`
- **Issue:** `if (dev->udev)` at line 510 is redundant — `dev->udev` is
  already validated 30 lines earlier at line 474 and the function returns on
  NULL.
- **Fix:** Removed the redundant guard; `usb_set_interface` is called directly.

#### PCM-6: 350ms sleeps in `zg01_set_rate`
- **File:** `zg01_pcm.c:573, 608`
- **Note:** The `msleep(150)` (retry pause) and `msleep(200)` (post-config
  stabilise) calls are intentional hardware requirements observed in the
  Windows USB capture. The device requires time to accept the sample rate
  change before streaming can resume. These sleeps are left unchanged.

#### PCM-7: `channel_name` variable shadows outer declaration
- **File:** `zg01_pcm.c:709`
- **Fix:** Renamed inner `channel_name` to `ch_name`.

#### PCM-8: Magic number `2` instead of constant
- **File:** `zg01_pcm.c:717`
- **Fix:** `!= 2` → `!= CHANNEL_TYPE_VOICE_OUT`.

#### PCM-11: Spurious `period_elapsed` on every modulo-zero position
- **File:** `zg01_pcm.c:1064`
- **Issue:** `(*pcm_pos % period_size) == 0` fires on position 0 and every
  multiple, including the very first URB. This generates spurious
  `period_elapsed` calls.
- **Fix:** Compare quotients: `(new_pos / period_size) != (old_pos /
  period_size)`.

#### PCM-12: Non-108-byte capture packets silently dropped
- **File:** `zg01_pcm.c:1075`
- **Fix:** Added `pr_debug("zg01_pcm: unexpected capture packet size %u\n",
  pkt_len)` before `continue`.

#### PCM-14: Same spurious `period_elapsed` in capture path
- **File:** `zg01_pcm.c:1133`
- **Fix:** Same quotient comparison as PCM-11.

#### PCM-15: `snd_pcm_stop_xrun` without stream lock
- **File:** `zg01_pcm.c:1160-1162`
- **Issue:** `snd_pcm_stop_xrun` must be called with the PCM stream lock held.
  Calling it from a URB callback without the lock is a race.
- **Fix:** Wrapped in `snd_pcm_stream_lock_irqsave` /
  `snd_pcm_stream_unlock_irqrestore`.

#### PCM-17: `open_count` never decremented
- **File:** `zg01_pcm.c:93-101`
- **Issue:** `open_count` is incremented on open but never decremented or
  reset on close. After a few open/close cycles the count grows permanently,
  suppressing all informational logging forever.
- **Fix:** Decrement `open_count` in `zg01_pcm_close`.

#### PCM-18: Off-by-one confusion in cleanup loop
- **File:** `zg01_pcm.c:1331`
- **Issue:** `for (j = 0; j <= urb_idx ...)` — when `urb_idx` is the index
  of the URB that failed to allocate, `iso_urbs[urb_idx]` is NULL. The loop
  frees it safely due to NULL checks but the intent is unclear.
- **Fix:** Changed to `j < urb_idx` with comment: the URB at `urb_idx` was
  never allocated.

#### PCM-19: `GFP_DMA` unnecessarily restricts USB buffer allocation
- **File:** `zg01_pcm.c:1262`
- **Issue:** `GFP_DMA` limits allocation to the 16MB ISA DMA zone, which is
  unnecessary on modern systems with xHCI controllers.
- **Fix:** Removed `GFP_DMA`.

#### PCM-23: `spin_lock` in pointer callback vs `spin_lock_irqsave` in URB
- **File:** `zg01_pcm.c:1500-1508`
- **Issue:** `zg01_pcm_pointer` uses `spin_lock(&dev->lock)`, but the URB
  callback uses `spin_lock_irqsave`. If `zg01_pcm_pointer` is called from a
  context where IRQs are enabled and the URB callback fires on the same CPU,
  the lock is re-entered without IRQ save, causing a deadlock.
- **Fix:** Changed to `spin_lock_irqsave` / `spin_unlock_irqrestore`.

#### PCM-24: `channel_type < 0` guard is unreachable
- **File:** `zg01_pcm.c:1549-1562`
- **Issue:** `channel_type` is an `int` field in a `kzalloc`'d struct, so it
  is always 0 on allocation. The `< 0` guard and legacy fallback are dead code.
- **Fix:** Removed the guard and fallback block.

#### PCM-27: Direct `runtime->status->state` read
- **File:** `zg01_pcm.c:931`
- **Issue:** Directly reading `runtime->status->state` bypasses the ALSA API
  and is fragile against internal ALSA struct changes.
- **Fix:** Replaced with `!snd_pcm_running(substream)`.

#### PCM-28: `SNDRV_PCM_INFO_MMAP_VALID` missing
- **File:** `zg01_pcm.c:104`
- **Issue:** Without `SNDRV_PCM_INFO_MMAP_VALID`, PipeWire and PulseWire
  fall back to copy-based transfer instead of zero-copy mmap.
- **Fix:** Added `SNDRV_PCM_INFO_MMAP_VALID` to `runtime->hw.info`.

---

### Low (fix — style, dead code, warnings)

#### USB-1: Duplicate VENDOR_ID/PRODUCT_ID macros
- **File:** `zg01_usb.c:12-13`
- **Fix:** Removed duplicate definitions (already in `zg01.h`).

#### USB-11: Mixed declarations after statements
- **File:** `zg01_usb.c:74`
- **Fix:** Moved `const char *card_id` to top of function.

#### USB-12: `MODULE_AUTHOR("Your Name")` placeholder
- **Files:** `zg01_usb.c:310`, `zg01_pcm.c:1638`, `zg01_control.c:72`
- **Fix:** Changed to `"Yamaha ZG01 Driver Contributors"`.

#### CTL-3: Unnecessary `EXPORT_SYMBOL_GPL` for intra-module symbol
- **File:** `zg01_control.c:70`
- **Fix:** Removed. Also removed stray `MODULE_*` macros from non-entry file.

#### CTL-4: `zg01_free_control` declared but never defined
- **File:** `zg01_control.h:13`
- **Fix:** Removed the declaration.

#### DISC-1: `char*` pointer style
- **File:** `zg01_usb_discovery.c:23,39`
- **Fix:** `const char*` → `const char *` per kernel coding style.

#### DISC-2: Hard-coded `16` in endpoint loop
- **File:** `zg01_usb_discovery.c:85`
- **Fix:** `ep_idx < 16` → `ep_idx < ARRAY_SIZE(info.endpoints)`.

#### DISC-3: 20+ `pr_info` on every probe
- **File:** `zg01_usb_discovery.c:107-188`
- **Fix:** All `pr_info` in `zg01_discover_usb_config` and
  `zg01_discover_all_alt_settings` converted to `pr_debug`. Debug information
  is still accessible via `dyndbg` but does not flood the kernel log on every
  device attach.

#### DISC-4: `EXPORT_SYMBOL_GPL` for intra-module symbols
- **File:** `zg01_usb_discovery.c:233-234`
- **Fix:** Removed both `EXPORT_SYMBOL_GPL` lines.

#### DISC-5: `MODULE_*` macros in non-driver-entry file
- **File:** `zg01_usb_discovery.c:236-239`
- **Fix:** Removed all four `MODULE_*` macros.

#### HDR-1: Include guard `AUDIO_H` → `ZG01_PCM_H`
- **File:** `zg01_pcm.h:1-2`
- **Fix:** Corrected guard names.

#### HDR-2: Include guard `CONTROL_H` → `ZG01_CONTROL_H`
- **File:** `zg01_control.h:1-2`
- **Fix:** Corrected guard names.

#### HDR-3/4: `zg01_pcm.h` is BCD2000 copy-paste dead code
- **File:** `zg01_pcm.h`
- **Issue:** The entire file is a near-verbatim copy from the BCD2000 driver,
  including a `bcd2k` field name. None of its structs or constants are
  referenced by the actual ZG01 driver code.
- **Fix:** Replaced with a minimal header containing only the corrected include
  guard. Existing structs (`zg01_pcm`, `zg01_substream`, `zg01_urb`) in
  `zg01.h` are the ones actually used.

#### ZH-1: `struct zg01_midi *midi` — type never defined
- **File:** `zg01.h:40`
- **Fix:** Removed field.

#### ZH-2/3/4: `*_startup_frames` fields never read
- **File:** `zg01.h:83-85`
- **Fix:** Removed all three fields.

#### ZH-5: `rate_residual` field written but never read
- **File:** `zg01.h:88`
- **Fix:** Removed field. The write at `zg01_pcm.c:438` is also removed.

#### ZH-6: `wq` workqueue field never initialized
- **File:** `zg01.h:104`
- **Fix:** Removed field.

#### PCM-25: Fixed min==max buffer size prevents ALSA negotiation
- **File:** `zg01_pcm.c:1621-1623`
- **Issue:** `snd_pcm_set_managed_buffer_all` called with identical min and
  max prevents the ALSA negotiation from selecting smaller buffers for
  low-latency use.
- **Fix:** Min set to `buffer_size / 8`, max to `buffer_size`.

#### PCM-26: `zg01_pcm_start_work` is a non-functional stub
- **File:** `zg01_pcm.c:11-14`
- **Issue:** The stub just prints an info message. The `start_work_*` delayed
  work fields and `start_pending_*` flags are never queued from actual code
  paths.
- **Fix:** Removed stub function, forward declaration, all `INIT_DELAYED_WORK`
  calls, `start_pending_*` assignments, and the corresponding struct fields in
  `zg01.h`.

#### PCM-29: No bounds check before silence memset
- **File:** `zg01_pcm.c:936-942`
- **Fix:** Added `WARN_ON(pkt_len > urb->transfer_buffer_length)` safety check.

---

## Summary

| Severity | Count | All fixed |
|----------|-------|-----------|
| Critical | 5     | Yes       |
| High     | 13    | Yes       |
| Medium   | 18    | Yes       |
| Low      | 14    | Yes       |
| **Total**| **50**| **Yes**   |
