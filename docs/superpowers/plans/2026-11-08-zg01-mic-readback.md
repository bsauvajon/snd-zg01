# Plan — MIC state readback

Source spec: `docs/superpowers/specs/2026-11-08-zg01-mic-readback.md`
Branch: `feat/zg01-gui`
Worktree: `/home/brice/repos/snd-zg01/.worktrees/zg01-gui`
Status: phases 1–2 implemented; phase 2 pending review/commit; phases 3–4
pending.

## Phase 1 — protocol, fixture, parser

- [x] Controlled captures for gate, compressor, limiter and EQ fields.
- [x] Confirm compressor switch `1470` and EQ switch `1628` (off→on).
- [x] Fixture `tests/fixtures/mic-state-persisted.bin` (persisted state
      after Reset).
- [x] Pure parser `src/zg01_state.c` + `src/zg01_state.h`.
- [x] Host test `tests/test-state-parser.c`, wrapper
      `tests/test-state-parser.sh`.
- [x] `docs/STATE_DUMP_PROTOCOL.md`; update `MIC_CONTROL_PROTOCOL.md`.
- [x] CI step for the parser test; module object added to the Makefile.
- [x] Run parser test, strict kernel build, review, commit
      (`a732bed`).

DoD met: parser test green on the fixture; strict `W=1` build green.

## Phase 2 — driver collector and cache update

- [x] Collector in the meter-reader path: the first 512-byte frame with
      the `f0 43 00` header and the next seven continuations; 192-byte
      meter frames unchanged.
- [x] Startup probe arms the collector and sends the type-20 request, so
      the existing read populates `dev->param_cache`.
- [x] `State Read` action, `State Read Status` and `State Read Known`
      ALSA controls.
- [x] `Reset to ZG01` re-arms and re-reads after the reset frame.
- [x] Direct fields mapped to `ZG01_PARAM_*`; transformed fields never
      written to the UI cache.
- [x] Two-second timeout; partial/short/unknown header rejected by the
      parser; failed read leaves the cache untouched.
- [x] Host parser tests; `make W=1`.
- [x] Hardware: startup `Status=1`, `Known=0x3fffff`, Gate on/60,
      Limiter on/50 after module load; `State Read` action works; Gate
      80 → Reset → Gate 60 with `Status=1`.
- [x] Phase 2 review and commit (`49328f6`).

DoD met on hardware.

## Phase 3 — GUI

- [x] Read `State Read Status`/known mask; Micro direct fields from the
      device are `Known`, transformed fields `Unknown` (dashed).
- [x] **Lire depuis le ZG01** action; waits for the status; reloads.
- [x] Startup: probe read normally already complete; if not, Unknown.
- [x] Reset flow: wait for the driver's post-reset read, reload.
- [x] Save enabled only after a successful read; no cache-default Save.
- [x] Mixage page notice: section not decoded in this phase.
- [x] Footer provenance badge (`MIC lu · Mixage en cache`).
- [x] Model readback tests (`apply_readback`, `can_save`, status
      transitions) and `make -C gui test`.
- [ ] Hardware `GUI_TEST.md` user confirmation: persisted Gate on/60
      shown; external change + Read; Reset + Read.

## Phase 4 — packaging and release notes

- [x] Desktop/AppStream copy: MIC readback, Mixage cached, live meters
      blocked.
- [x] README status and release note limits.
- [ ] Validators (network URL warnings remain) and final whole-branch
      review.

## Unaddressed / deferred

- Gate threshold `1462` is a transformed device value; currently Unknown.
- Compressor/limiter attack/release/out gain (1475/1477/1481,
  1665/1667/1670) are transformed; Unknown.
- EQ band gain/frequency are biquad coefficients; filter-model inverse
  deferred.
- Mixage/Game/Stream and Advanced state sections: type-20 ids not
  captured; GUI keeps the cache and must label it.
- A `State Read` while another is pending returns `-EBUSY`. Phase 3
  serializes Read/Reset/Save/Preset in the GUI; the driver still returns
  `-EBUSY` for direct ALSA callers.
- A state read/reload marks a transformed MIC field unknown even after the
  user wrote it; the write is known until the next read. This is the
  documented consequence of not decoding the DSP value.
- A `State Read` write that races an already-armed read can surface as a
  failure toast even though the pending read completes; the next Read
  recovers.
- After the first state frame, a stray 512-byte frame would be appended as
  a continuation; the parser rejects the dump and the cache stays intact,
  but the read fails. The 512-byte channel is state-only while armed
  (review L1).
- Firmware variants: the parser requires the observed eight-byte header;
  unknown headers are rejected rather than parsed with wrong offsets.

## Commit plan

Phase 1: `feat(zg01): decode and test the MIC state dump layout` — done
(`a732bed`).
Phase 2: `feat(zg01): read the MIC state dump into the control cache`
Phase 3: `feat(gui): show the ZG01 MIC state and add a read action`
Phase 4: `docs: document MIC readback limits`
