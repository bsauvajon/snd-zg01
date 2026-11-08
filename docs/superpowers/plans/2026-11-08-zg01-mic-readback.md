# Plan — MIC state readback

Source spec: `docs/superpowers/specs/2026-11-08-zg01-mic-readback.md`
Branch: `feat/zg01-gui`
Worktree: `/home/brice/repos/snd-zg01/.worktrees/zg01-gui`
Status: phase 1 in progress; phases 2–4 pending.

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
- [ ] Run `tests/test-state-parser.sh`, strict kernel build, review, commit.

DoD: parser test green on the fixture; strict `W=1` build green; docs
reviewed; no hardware needed.

## Phase 2 — driver collector and cache update

- [ ] `zg01_state` collector in the meter-reader path: state frames
      (`f0 43 00`) accumulate to eight; meter frames unchanged.
- [ ] Startup probe arms the collector before sending the type-20
      request, so the existing read populates `dev->param_cache`.
- [ ] `State Read` (write 1) and `State Read Status` (read-only) ALSA
      controls; a read status/mask for the GUI.
- [ ] `Reset to ZG01` re-arms and re-reads after the reset frame.
- [ ] Map the direct fields to `ZG01_PARAM_*` addresses and write the
      cache under `param_mutex`; never write transformed fields.
- [ ] Host tests for partial/aborted/wrong-header reads; the cache is
      untouched on failure.
- [ ] Hardware check: after `insmod`, `amixer cget` on the direct fields
      returns Gate on/60 (the persisted value), Limiter on/50, etc.
- [ ] `make W=1`, module load, meter regression.

DoD: direct fields populated from the device at load; manual refresh
after an external change updates them; failure leaves the cache and the
status error; no meter regression.

## Phase 3 — GUI

- [ ] Read `State Read Status`/known mask; Micro fields from the device
      are `Known`, transformed fields `Unknown` (dashed).
- [ ] **Lire depuis le ZG01** action; waits for the status; reloads.
- [ ] Startup: probe read normally already complete; if not, Unknown.
- [ ] Reset flow: wait for the driver's post-reset read, reload.
- [ ] Save enabled only after a successful read; no cache-default Save.
- [ ] Mixage page notice: section not decoded in this phase.
- [ ] GUI tests with a fake backend; `make -C gui test`.
- [ ] Hardware `GUI_TEST.md`: persisted Gate on/60 shown; external change
      + Read; Reset + Read.

DoD: manual log/screenshots show the persisted state; EQ gain/freq
dashed; Save disabled before the first read, enabled after.

## Phase 4 — packaging and release notes

- [ ] Desktop/AppStream copy: MIC readback, Mixage cached, live meters
      blocked.
- [ ] Release note limits.
- [ ] Validators; final whole-branch review.

DoD: validators green; notes accurate.

## Unaddressed / deferred

- Gate threshold `1462` is a transformed device value; currently planned
  Unknown. Inverse curve or sweep table later.
- Compressor/limiter attack/release/out gain (1475/1477/1481,
  1665/1667/1670) are transformed; Unknown in phase 2.
- EQ band gain/frequency are biquad coefficients; filter-model inverse
  deferred.
- Mixage/Game/Stream and Advanced state sections: type-20 ids not
  captured; GUI keeps the cache and must label it.
- EQ switch `1628`, compressor switch `1470` confirmed; firmware
  variants still need header/length validation.
- The temporary `state_read` module parameter in `src/zg01_usb.c` is a
  development hook from the investigation; remove it when the ALSA
  `State Read` control lands.
- Hardware acceptance depends on the ZG01 being attached; the persisted
  state used for the fixture is the user's current state (Gate on/60).

## Commit plan

Phase 1: `feat(zg01): decode and test the MIC state dump layout`
(parser, fixture, docs, CI, Makefile). The temporary `state_read` hook is
not part of this commit.
Phase 2: `feat(zg01): read the MIC state dump into the control cache`
Phase 3: `feat(gui): show the ZG01 MIC state and add a read action`
Phase 4: `docs: document MIC readback limits`
