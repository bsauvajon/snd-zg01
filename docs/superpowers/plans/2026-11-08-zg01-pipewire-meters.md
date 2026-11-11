# ZG01 PipeWire Meters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show live MIC, VOICE and GAME VU levels in the GUI from PipeWire while the ZG01 card meters stay blocked on EP `0x83`.

**Architecture:** `zg01-core` gains a `pipewire` feature that discovers the three ZG01 nodes with `pw-dump`, runs one `pw-record` child per node, reduces raw `s16` buffers to 50 ms peaks, and sends `LinuxMetersSnapshot`s. The GUI keeps the card meter and adds the Linux bar to the existing `Meter` widget; the two readings update independently.

**Tech Stack:** Rust 2021, `zg01-core`, GTK4/libadwaita GUI, PipeWire command-line tools (`pw-dump`, `pw-record`).

**Spec:** `docs/superpowers/specs/2026-11-08-zg01-pipewire-meters-design.md`

## Global Constraints

- No new build or link dependency. `pw-dump` and `pw-record` are runtime tools from the `pipewire` package.
- The `pipewire` Cargo feature is enabled by default; disabling it compiles the module's worker out and hides every Linux bar.
- Sources and mapping: `Voice In` → MIC strip and Micro `MIC IN`; `Voice Out` → VOICE strip; `Game Out` → GAME strip; STREAM OUT has no Linux meter.
- Snapshot interval is 50 ms. Each recorder uses `--rate 48000 --channels 2 --format s16 --latency 20ms --raw -`.
- Sink monitors set `-P stream.capture.sink=true`; the `Voice In` source does not.
- Levels use the card meter scale: `peak == 0` → `0.0`; otherwise `db = 20*log10(peak/32768)` clamped to `-127..=0` and `level = (db + 127) / 127`.
- `None` means unreadable; a missing source has no bar, a present source with no reading is dashed, and only a captured silent buffer reads zero.
- Card meters, the driver, `docs/METER_PROTOCOL.md` and the `0x03`/`0x83` protocol are untouched.
- Closing the window, disconnecting the card, or stopping the pump must reap every `pw-record` child.
- The true pre-transformation microphone input remains out of scope. `Meter Mic In` is the pre-gate card meter to restore later.

## Review Focus

- A `pw-record` child survives closing the window or disconnecting the card.
- PipeWire, `pw-dump` or a node is missing: the GUI still works and shows no fake bar.
- A card snapshot and a Linux snapshot overwrite each other's bar.
- A snapshot from an old backend generation moves the bars of the newly connected card.
- The `Voice In` source is configured like a sink, or a sink misses `stream.capture.sink=true`, so the bar stays dashed.

---

### Task 0: Record the design and the plan

**Files:**
- Add: `docs/superpowers/specs/2026-11-08-zg01-pipewire-meters-design.md`
- Add: `docs/superpowers/plans/2026-11-08-zg01-pipewire-meters.md`

- [ ] **Step 1: Commit the documents**

```bash
git add docs/superpowers/specs/2026-11-08-zg01-pipewire-meters-design.md \
        docs/superpowers/plans/2026-11-08-zg01-pipewire-meters.md
git commit -m "docs(gui): design the PipeWire VU meters"
```

---

### Task 1: Add Linux meter ids and the screen mapping

**Files:**
- Modify: `gui/crates/zg01-core/src/mixage.rs:14-140`
- Test: `gui/crates/zg01-core/tests/mixage.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub enum LinuxMeterId { MicIn, VoiceOut, GameOut }` with `ALL: [LinuxMeterId; 3]` and `index(self) -> usize` (0, 1, 2 in that order).
- Produces: `StripModel.linux_meter: Option<LinuxMeterId>` and `StreamOutputModel.linux_meter: Option<LinuxMeterId>`.

- [ ] **Step 1: Write the failing test**

Add to `gui/crates/zg01-core/tests/mixage.rs`:

```rust
#[test]
fn strips_map_to_their_linux_meters() {
    let model = MixageModel::from_controls(&controls_with_mixage());
    assert_eq!(model.strips[0].linux_meter, Some(LinuxMeterId::MicIn));
    assert_eq!(model.strips[1].linux_meter, Some(LinuxMeterId::VoiceOut));
    assert_eq!(model.strips[2].linux_meter, Some(LinuxMeterId::GameOut));
    assert_eq!(model.output.linux_meter, None);
}

#[test]
fn linux_meter_ids_have_stable_slots() {
    assert_eq!(LinuxMeterId::MicIn.index(), 0);
    assert_eq!(LinuxMeterId::VoiceOut.index(), 1);
    assert_eq!(LinuxMeterId::GameOut.index(), 2);
    assert_eq!(LinuxMeterId::ALL.len(), 3);
}
```

Update the existing import to `use zg01_core::mixage::{LinuxMeterId, MeterId, MixageModel};`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd gui && cargo test -p zg01-core --test mixage`
Expected: compile error, `LinuxMeterId` not found.

- [ ] **Step 3: Implement the enum and the fields**

In `gui/crates/zg01-core/src/mixage.rs`, add above `StripModel`:

```rust
/// One of the three Linux-side sources the GUI can meter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinuxMeterId {
    /// The `Voice In` source.
    MicIn,
    /// The `Voice Out` sink monitor.
    VoiceOut,
    /// The `Game Out` sink monitor.
    GameOut,
}

impl LinuxMeterId {
    /// Every Linux meter, in snapshot-slot order.
    pub const ALL: [LinuxMeterId; 3] = [
        LinuxMeterId::MicIn,
        LinuxMeterId::VoiceOut,
        LinuxMeterId::GameOut,
    ];

    /// The slot of this source in a `LinuxMetersSnapshot`.
    pub const fn index(self) -> usize {
        match self {
            LinuxMeterId::MicIn => 0,
            LinuxMeterId::VoiceOut => 1,
            LinuxMeterId::GameOut => 2,
        }
    }
}
```

Add `pub linux_meter: Option<LinuxMeterId>,` to `StripModel` and `StreamOutputModel`, with a doc comment explaining that `None` means no Linux source. Set MIC → `Some(LinuxMeterId::MicIn)`, VOICE → `Some(LinuxMeterId::VoiceOut)`, GAME → `Some(LinuxMeterId::GameOut)`, STREAM OUT → `None` in `from_controls`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd gui && cargo test -p zg01-core --test mixage`
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add gui/crates/zg01-core/src/mixage.rs gui/crates/zg01-core/tests/mixage.rs
git commit -m "feat(gui): map the strips to their Linux meter sources"
```

---

### Task 2: Discover nodes, build recorder arguments and convert peaks

**Files:**
- Create: `gui/crates/zg01-core/src/pipewire.rs`
- Modify: `gui/crates/zg01-core/src/lib.rs`
- Modify: `gui/crates/zg01-core/Cargo.toml`
- Test: `gui/crates/zg01-core/src/pipewire.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: `LinuxMeterId` from Task 1.
- Produces: `pub struct Nodes { pub mic_in: Option<u64>, pub voice_out: Option<u64>, pub game_out: Option<u64> }`.
- Produces: `pub fn parse_pw_dump(json: &str) -> Result<Nodes, PipewireError>`.
- Produces: `pub fn discover_nodes() -> Result<Nodes, PipewireError>` (runs `pw-dump`).
- Produces: `pub fn recorder_args(node: u64, sink: bool) -> Vec<String>`.
- Produces: `pub fn level_from_peak(peak: u32) -> f64`.
- Produces: `pub const LINUX_METER_INTERVAL: Duration` (50 ms).
- Produces: `pub struct LinuxMetersSnapshot { pub values: [Option<f64>; 3], pub available: [bool; 3], pub generation: u64 }` with `level(self, LinuxMeterId) -> Option<f64>`, `is_available(self, LinuxMeterId) -> bool`, `belongs_to(self, u64) -> bool`.

- [ ] **Step 1: Write the failing tests**

Create `gui/crates/zg01-core/src/pipewire.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::{level_from_peak, parse_pw_dump, recorder_args, LinuxMetersSnapshot};
    use crate::mixage::LinuxMeterId;

    const DUMP: &str = r#"[
      {"id": 69, "type": "PipeWire:Interface:Node",
       "info": {"props": {"node.name": "alsa_output.usb-Yamaha_Corporation_Yamaha_ZG01-01.HiFi__VoiceOut__sink",
                          "node.description": "Yamaha ZG01 Voice Out", "object.serial": "69"}}},
      {"id": 70, "type": "PipeWire:Interface:Node",
       "info": {"props": {"node.name": "alsa_output.usb-Yamaha_Corporation_Yamaha_ZG01-01.HiFi__Game__sink",
                          "node.description": "Yamaha ZG01 Game Out", "object.serial": "70"}}},
      {"id": 71, "type": "PipeWire:Interface:Node",
       "info": {"props": {"node.name": "alsa_input.usb-Yamaha_Corporation_Yamaha_ZG01-01.HiFi__VoiceIn__source",
                          "node.description": "Yamaha ZG01 Voice In", "object.serial": "71"}}},
      {"id": 12, "type": "PipeWire:Interface:Node",
       "info": {"props": {"node.name": "alsa_output.pci-0000_00_1f.3.analog-stereo",
                          "object.serial": "12"}}}
    ]"#;

    #[test]
    fn the_dump_selects_the_three_zg01_nodes() {
        let nodes = parse_pw_dump(DUMP).expect("a valid dump");
        assert_eq!(nodes.mic_in, Some(71));
        assert_eq!(nodes.voice_out, Some(69));
        assert_eq!(nodes.game_out, Some(70));
    }

    #[test]
    fn unrelated_nodes_and_missing_nodes_are_not_invented() {
        let nodes = parse_pw_dump("[]").expect("an empty dump");
        assert_eq!(nodes.mic_in, None);
        assert_eq!(nodes.voice_out, None);
        assert_eq!(nodes.game_out, None);
        assert!(parse_pw_dump("not json").is_err());
    }

    #[test]
    fn sinks_capture_their_monitor_and_the_source_does_not() {
        let sink = recorder_args(70, true);
        assert!(sink
            .windows(2)
            .any(|pair| pair[0] == "-P" && pair[1] == "stream.capture.sink=true"));
        assert!(sink
            .windows(2)
            .any(|pair| pair[0] == "--target" && pair[1] == "70"));
        assert!(sink.iter().any(|arg| arg == "--raw"));
        assert!(sink.iter().any(|arg| arg == "s16"));
        assert!(sink.iter().any(|arg| arg == "48000"));

        let source = recorder_args(71, false);
        assert!(!source.iter().any(|arg| arg == "stream.capture.sink=true"));
    }

    #[test]
    fn peaks_use_the_card_db_scale() {
        assert_eq!(level_from_peak(0), 0.0);
        assert!((level_from_peak(32768) - 1.0).abs() < 1e-12);
        let minus_thirty = level_from_peak(1036);
        assert!((minus_thirty * 127.0 - 97.0).abs() < 0.01);
    }

    #[test]
    fn a_snapshot_reads_by_id_and_generation() {
        let snapshot = LinuxMetersSnapshot {
            values: [Some(0.25), None, Some(0.75)],
            available: [true, true, false],
            generation: 7,
        };
        assert_eq!(snapshot.level(LinuxMeterId::MicIn), Some(0.25));
        assert_eq!(snapshot.level(LinuxMeterId::VoiceOut), None);
        assert!(snapshot.is_available(LinuxMeterId::VoiceOut));
        assert!(!snapshot.is_available(LinuxMeterId::GameOut));
        assert!(snapshot.belongs_to(7));
        assert!(!snapshot.belongs_to(8));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd gui && cargo test -p zg01-core pipewire`
Expected: compile error, the module is not declared.

- [ ] **Step 3: Implement the pure functions and the module export**

In `gui/crates/zg01-core/Cargo.toml`:

```toml
[features]
default = ["pipewire"]
pipewire = []
```

In `gui/crates/zg01-core/src/lib.rs`:

```rust
/// The Linux-side meter sources read through PipeWire.
pub mod pipewire;
```

In `gui/crates/zg01-core/src/pipewire.rs`, implement:

```rust
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::mixage::LinuxMeterId;

pub const LINUX_METER_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nodes {
    pub mic_in: Option<u64>,
    pub voice_out: Option<u64>,
    pub game_out: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum PipewireError {
    #[error("could not run {tool}: {source}")]
    Run { tool: &'static str, source: std::io::Error },
    #[error("{tool} failed: {status}")]
    Status { tool: &'static str, status: std::process::ExitStatus },
    #[error("could not parse pw-dump: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinuxMetersSnapshot {
    pub values: [Option<f64>; 3],
    pub available: [bool; 3],
    pub generation: u64,
}

impl LinuxMetersSnapshot {
    pub fn level(self, id: LinuxMeterId) -> Option<f64> { self.values[id.index()] }
    pub fn is_available(self, id: LinuxMeterId) -> bool { self.available[id.index()] }
    pub fn belongs_to(self, generation: u64) -> bool { self.generation == generation }
}

pub fn recorder_args(node: u64, sink: bool) -> Vec<String> {
    let mut args = vec!["--target".to_string(), node.to_string()];
    if sink {
        args.push("-P".to_string());
        args.push("stream.capture.sink=true".to_string());
    }
    args.extend([
        "--rate".to_string(), "48000".to_string(),
        "--channels".to_string(), "2".to_string(),
        "--format".to_string(), "s16".to_string(),
        "--latency".to_string(), "20ms".to_string(),
        "--raw".to_string(), "-".to_string(),
    ]);
    args
}

pub fn level_from_peak(peak: u32) -> f64 {
    if peak == 0 {
        return 0.0;
    }
    let db = 20.0 * (f64::from(peak) / 32768.0).log10();
    ((db.clamp(-127.0, 0.0) + 127.0) / 127.0).clamp(0.0, 1.0)
}

pub fn parse_pw_dump(json: &str) -> Result<Nodes, PipewireError> {
    let dump: Value = serde_json::from_str(json)?;
    let mut nodes = Nodes::default();
    let Some(objects) = dump.as_array() else { return Ok(nodes) };
    for object in objects {
        let props = &object["info"]["props"];
        let name = props["node.name"].as_str().unwrap_or_default();
        let description = props["node.description"].as_str().unwrap_or_default();
        let serial = props["object.serial"]
            .as_str()
            .and_then(|serial| serial.parse::<u64>().ok())
            .or_else(|| props["object.serial"].as_u64());
        let Some(serial) = serial else { continue };
        if !(name.contains("ZG01") || description.starts_with("Yamaha ZG01")) {
            continue;
        }
        if name.ends_with("VoiceIn__source") || description == "Yamaha ZG01 Voice In" {
            nodes.mic_in = nodes.mic_in.or(Some(serial));
        } else if name.ends_with("VoiceOut__sink") || description == "Yamaha ZG01 Voice Out" {
            nodes.voice_out = nodes.voice_out.or(Some(serial));
        } else if name.ends_with("Game__sink") || description == "Yamaha ZG01 Game Out" {
            nodes.game_out = nodes.game_out.or(Some(serial));
        }
    }
    Ok(nodes)
}

pub fn discover_nodes() -> Result<Nodes, PipewireError> {
    let output = Command::new("pw-dump").output().map_err(|source| PipewireError::Run {
        tool: "pw-dump",
        source,
    })?;
    if !output.status.success() {
        return Err(PipewireError::Status { tool: "pw-dump", status: output.status });
    }
    parse_pw_dump(&String::from_utf8_lossy(&output.stdout))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd gui && cargo test -p zg01-core pipewire`
Expected: all five tests pass.

- [ ] **Step 5: Commit**

```bash
git add gui/crates/zg01-core/Cargo.toml gui/crates/zg01-core/src/lib.rs gui/crates/zg01-core/src/pipewire.rs
git commit -m "feat(core): discover the ZG01 PipeWire nodes and convert peaks"
```

---

### Task 3: Run the recorders and publish snapshots

**Files:**
- Modify: `gui/crates/zg01-core/src/pipewire.rs`
- Test: `gui/crates/zg01-core/src/pipewire.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: Task 2's `discover_nodes`, `recorder_args`, `level_from_peak`, `LinuxMetersSnapshot`, `Nodes`.
- Produces: `pub fn spawn_linux_meters(generation: u64, interval: Duration, stop: Arc<AtomicBool>) -> Receiver<LinuxMetersSnapshot>`.
- Produces: private `fn stop_child(slot: &Arc<Mutex<Option<Child>>>)`.

- [ ] **Step 1: Write the failing lifecycle test**

Add to the test module:

```rust
#[cfg(feature = "pipewire")]
#[test]
fn stopping_a_recorder_kills_its_child() {
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    let slot = Arc::new(Mutex::new(Some(
        Command::new("sh")
            .arg("-c")
            .arg("sleep 30")
            .spawn()
            .expect("a child"),
    )));
    super::stop_child(&slot);
    assert!(slot.lock().expect("the slot").is_none());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd gui && cargo test -p zg01-core stopping_a_recorder`
Expected: compile error, `stop_child` not found.

- [ ] **Step 3: Implement the worker**

Add to `pipewire.rs`:

```rust
#[cfg(feature = "pipewire")]
use std::io::Read;
#[cfg(feature = "pipewire")]
use std::process::{Child, Stdio};
use std::sync::atomic::AtomicBool;
#[cfg(feature = "pipewire")]
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
#[cfg(feature = "pipewire")]
use std::sync::mpsc::Sender;
use std::sync::Arc;
#[cfg(feature = "pipewire")]
use std::sync::Mutex;
#[cfg(feature = "pipewire")]
use std::thread::JoinHandle;
#[cfg(feature = "pipewire")]
use std::time::Instant;

#[cfg(feature = "pipewire")]
type Update = (LinuxMeterId, Option<f64>);

#[cfg(feature = "pipewire")]
struct Recorder {
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    join: JoinHandle<()>,
}

#[cfg(feature = "pipewire")]
impl Recorder {
    fn start(id: LinuxMeterId, node: u64, sink: bool, updates: Sender<Update>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let thread_stop = stop.clone();
        let thread_child = child.clone();
        let join = std::thread::Builder::new()
            .name(format!("zg01-pw-{}", id.index()))
            .spawn(move || recorder_loop(id, node, sink, thread_stop, thread_child, updates))
            .expect("could not start a PipeWire recorder");
        Recorder { stop, child, join }
    }

    fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        stop_child(&self.child);
        let _ = self.join.join();
    }
}

#[cfg(feature = "pipewire")]
fn stop_child(slot: &Arc<Mutex<Option<Child>>>) {
    if let Some(mut child) = slot.lock().expect("the child slot").take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(feature = "pipewire")]
fn recorder_loop(
    id: LinuxMeterId,
    node: u64,
    sink: bool,
    stop: Arc<AtomicBool>,
    slot: Arc<Mutex<Option<Child>>>,
    updates: Sender<Update>,
) {
    let mut backoff = Duration::from_secs(1);
    while !stop.load(Ordering::Relaxed) {
        let mut command = Command::new("pw-record");
        command
            .args(recorder_args(node, sink))
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                tracing::warn!("could not start pw-record for {id:?}: {err}");
                let _ = updates.send((id, None));
                sleep_or_stop(&stop, backoff);
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            let _ = updates.send((id, None));
            sleep_or_stop(&stop, backoff);
            continue;
        };
        *slot.lock().expect("the child slot") = Some(child);
        backoff = Duration::from_secs(1);

        let mut buffer = [0_u8; 4096];
        let mut peak = 0_u32;
        let mut window = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(len) => {
                    for sample in buffer[..len].chunks_exact(2) {
                        let value = i16::from_le_bytes([sample[0], sample[1]]);
                        peak = peak.max(value.unsigned_abs() as u32);
                    }
                    if window.elapsed() >= LINUX_METER_INTERVAL {
                        if updates.send((id, Some(level_from_peak(peak)))).is_err() {
                            stop_child(&slot);
                            return;
                        }
                        peak = 0;
                        window = Instant::now();
                    }
                }
                Err(_) => break,
            }
        }
        stop_child(&slot);
        let _ = updates.send((id, None));
        sleep_or_stop(&stop, backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

#[cfg(feature = "pipewire")]
fn sleep_or_stop(stop: &AtomicBool, delay: Duration) {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(
            Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

#[cfg(feature = "pipewire")]
pub fn spawn_linux_meters(
    generation: u64,
    interval: Duration,
    stop: Arc<AtomicBool>,
) -> Receiver<LinuxMetersSnapshot> {
    let (snapshots, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("zg01-pw-meters".to_string())
        .spawn(move || coordinator_loop(generation, interval, stop, snapshots))
        .expect("could not start the PipeWire meter poller");
    receiver
}

#[cfg(not(feature = "pipewire"))]
pub fn spawn_linux_meters(
    _generation: u64,
    _interval: Duration,
    _stop: Arc<AtomicBool>,
) -> Receiver<LinuxMetersSnapshot> {
    let (_sender, receiver) = mpsc::channel();
    receiver
}

#[cfg(feature = "pipewire")]
fn coordinator_loop(
    generation: u64,
    interval: Duration,
    stop: Arc<AtomicBool>,
    snapshots: Sender<LinuxMetersSnapshot>,
) {
    let (updates, receiver) = mpsc::channel::<Update>();
    let mut available = [false; 3];
    let mut values = [None; 3];
    let mut started = [false; 3];
    let mut recorders: Vec<Recorder> = Vec::new();
    let mut retry = Instant::now();
    let mut last_send: Option<Instant> = None;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if Instant::now() >= retry && !started.iter().all(|started| *started) {
            retry = Instant::now() + Duration::from_secs(2);
            match discover_nodes() {
                Ok(nodes) => {
                    for (id, node, sink) in [
                        (LinuxMeterId::MicIn, nodes.mic_in, false),
                        (LinuxMeterId::VoiceOut, nodes.voice_out, true),
                        (LinuxMeterId::GameOut, nodes.game_out, true),
                    ] {
                        let Some(node) = node else { continue };
                        available[id.index()] = true;
                        if !started[id.index()] {
                            started[id.index()] = true;
                            recorders.push(Recorder::start(id, node, sink, updates.clone()));
                        }
                    }
                }
                Err(err) => tracing::debug!("could not discover the PipeWire meters: {err}"),
            }
        }

        let wait = last_send
            .map(|sent| interval.saturating_sub(sent.elapsed()))
            .unwrap_or_default();
        match receiver.recv_timeout(wait) {
            Ok((id, value)) => {
                values[id.index()] = value;
                while let Ok((id, value)) = receiver.try_recv() {
                    values[id.index()] = value;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let due = last_send.is_none_or(|sent| sent.elapsed() >= interval);
        if due {
            let snapshot = LinuxMetersSnapshot {
                values,
                available,
                generation,
            };
            if snapshots.send(snapshot).is_err() {
                break;
            }
            last_send = Some(Instant::now());
        }
    }

    for recorder in recorders {
        recorder.stop();
    }
}
```

`Recorder::stop` takes `self`, so the coordinator owns the vector.

- [ ] **Step 4: Run the tests and the build**

Run:
```bash
cd gui
cargo test -p zg01-core stopping_a_recorder
cargo test -p zg01-core pipewire
cargo build -p zg01-core --no-default-features
```
Expected: the lifecycle test passes, the pure tests pass, and the no-feature build compiles.

- [ ] **Step 5: Commit**

```bash
git add gui/crates/zg01-core/src/pipewire.rs
git commit -m "feat(core): run the PipeWire recorders and publish peaks"
```

---

### Task 4: Update card and Linux bars independently in the meter widget

**Files:**
- Modify: `gui/crates/zg01-gui/src/widgets/meter.rs:61-235, 328-399`
- Test: `gui/crates/zg01-gui/src/widgets/meter.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `MeterSpec.card_present: bool` (a strip without a card meter, like GAME, has no card track).
- Produces: `Meter::set_card(&self, card: Option<f64>)` and `Meter::set_linux_source(&self, present: bool, level: Option<f64>)`.
- Produces: private `apply_card(&mut MeterSpec, Option<f64>)`, `apply_linux(&mut MeterSpec, bool, Option<f64>)` and `meter_width(card_present: bool, has_linux: bool) -> i32`.
- Keeps `Meter::set_levels` as a two-sided wrapper until Task 5 removes it.

- [ ] **Step 1: Write the failing tests**

Add to the widget test module:

```rust
#[test]
fn a_card_update_keeps_the_linux_bar() {
    let mut spec = MeterSpec {
        label: "MIC".to_string(),
        card: Some(0.4),
        card_present: true,
        linux: Some(0.2),
        peak: 0.0,
        linux_present: true,
    };
    super::apply_card(&mut spec, Some(0.6));
    assert_eq!(spec.card, Some(0.6));
    assert_eq!(spec.linux, Some(0.2));
    assert!(spec.linux_present);
}

#[test]
fn a_linux_update_keeps_the_card_bar_and_controls_presence() {
    let mut spec = MeterSpec {
        label: "MIC".to_string(),
        card: Some(0.4),
        card_present: true,
        linux: None,
        peak: 0.0,
        linux_present: false,
    };
    super::apply_linux(&mut spec, true, Some(0.2));
    assert_eq!(spec.card, Some(0.4));
    assert_eq!(spec.linux, Some(0.2));
    assert!(spec.linux_present);

    super::apply_linux(&mut spec, false, None);
    assert_eq!(spec.card, Some(0.4));
    assert_eq!(spec.linux, None);
    assert!(!spec.linux_present);
}
```

Update the existing width test:

```rust
#[test]
fn the_card_meter_is_narrower_than_the_pair() {
    assert_eq!(meter_width(true, false), 19);
    assert_eq!(meter_width(true, true), 26);
    assert_eq!(meter_width(false, true), 15);
    assert_eq!(meter_width(false, false), 12);
}
```

Add `card_present: true` to every existing `MeterSpec` literal in the test module.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd gui && cargo test -p zg01-gui a_card_update`
Expected: compile error, `apply_card` or `card_present` not found.

- [ ] **Step 3: Implement the independent updates**

Add `pub(crate) card_present: bool,` to `MeterSpec`, next to `card`, with the doc: whether the strip has a card meter at all (GAME has none).

Add next to `bar_state`:

```rust
/// Installs a card reading without touching the Linux source.
fn apply_card(spec: &mut MeterSpec, card: Option<f64>) {
    spec.card = normalise(card);
}

/// Installs a Linux source and its reading without touching the card one.
fn apply_linux(spec: &mut MeterSpec, present: bool, level: Option<f64>) {
    spec.linux = if present { normalise(level) } else { None };
    spec.linux_present = present || spec.linux.is_some();
}
```

Change `meter_width` to:

```rust
fn meter_width(card_present: bool, has_linux: bool) -> i32 {
    let mut bars = 0.0;
    if card_present {
        bars += CARD_WIDTH;
    }
    if card_present && has_linux {
        bars += METER_GAP;
    }
    if has_linux {
        bars += LINUX_WIDTH;
    }
    (bars + 2.0 * SIDE_MARGIN).round() as i32
}
```

Replace `Meter::set_levels` with the new methods and keep the wrapper:

```rust
/// Installs a card reading; the Linux bar is untouched.
pub(crate) fn set_card(&self, card: Option<f64>) {
    self.update(|spec| apply_card(spec, card));
}

/// Installs a Linux source and its reading; the card bar is untouched.
pub(crate) fn set_linux_source(&self, present: bool, level: Option<f64>) {
    self.update(|spec| apply_linux(spec, present, level));
}

/// Installs both sides, for callers not yet migrated to the split methods.
pub(crate) fn set_levels(&self, card: Option<f64>, linux: Option<f64>, _peak: f64) {
    self.set_card(card);
    self.set_linux_source(linux.is_some(), linux);
}

/// Applies one side of the meter, then repaints and updates the peak.
fn update(&self, apply: impl FnOnce(&mut MeterSpec)) {
    let had_linux = self.inner.spec.borrow().linux_present;
    let card_present = self.inner.spec.borrow().card_present;
    {
        let mut spec = self.inner.spec.borrow_mut();
        apply(&mut spec);
    }
    let has_linux = self.inner.spec.borrow().linux_present;
    if had_linux != has_linux {
        self.area.set_content_width(meter_width(card_present, has_linux));
    }
    refresh_accessible(&self.area, &self.inner.spec.borrow());
    self.area.queue_draw();

    let spec = self.inner.spec.borrow();
    let current = spec.card.into_iter().chain(spec.linux).fold(0.0, f64::max);
    let candidate = spec.peak.max(current);
    drop(spec);
    if candidate > self.inner.held_peak.get() {
        self.inner.held_peak.set(candidate);
        self.inner.peak_at.set(Some(Instant::now()));
        schedule_peak_clear(&self.inner, &self.area);
    }
}
```

In `Meter::new`, change the width call to `meter_width(spec.card_present, spec.linux_present)`.

In `draw`, honour `card_present`:

```rust
let has_card = spec.card_present;
let card_width = if has_card { CARD_WIDTH } else { 0.0 };
let gap = if has_card && has_linux { METER_GAP } else { 0.0 };
let bars = card_width + gap + if has_linux { LINUX_WIDTH } else { 0.0 };
let start = (f64::from(width) - bars) / 2.0;
let card_x = start;
let linux_x = start + card_width + gap;
let card_state = bar_state(has_card, spec.card);
```

Draw the card track only when `has_card`. The Linux track, fill, badge and peak line keep their current logic; the peak line uses `if has_linux { linux_x + LINUX_WIDTH } else { card_x + card_width }` as its right edge.

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
cd gui
cargo test -p zg01-gui a_card_update
cargo test -p zg01-gui a_linux_update
cargo test -p zg01-gui the_card_meter
cargo build -p zg01-gui
```
Expected: the new tests pass and the crate builds. The old `set_levels` callers still compile through the wrapper.

- [ ] **Step 5: Commit**

```bash
git add gui/crates/zg01-gui/src/widgets/meter.rs
git commit -m "feat(gui): update the card and Linux meter bars independently"
```

---

### Task 5: Wire the Linux snapshots into Mixage, Micro and the application

**Files:**
- Modify: `gui/crates/zg01-gui/src/ui/mixage.rs:49-52, 250-270, 300-460`
- Modify: `gui/crates/zg01-gui/src/ui/mic.rs:52-55, 315-325, 584-601`
- Modify: `gui/crates/zg01-gui/src/app.rs:20-30, 145-160, 260-270, 475-510, 540-560, 925-965, 1270-1310, 1320-1340`
- Test: build and existing tests.

**Interfaces:**
- Consumes: `LinuxMeterId` (Task 1), `LinuxMetersSnapshot`, `spawn_linux_meters`, `LINUX_METER_INTERVAL` (Tasks 2–3), `Meter::set_card`/`set_linux_source` (Task 4).
- Produces: `MixageScreen::set_linux_meter(LinuxMeterId, bool, Option<f64>)` and `MicScreen::set_linux_meter(LinuxMeterId, bool, Option<f64>)`.
- Produces: `App::start_linux_meters`, `App::stop_linux_meters`, `App::update_linux_meters`, `App::clear_linux_meters`.

- [ ] **Step 1: Update the Mixage screen**

In `mixage.rs`, import `LinuxMeterId`. Change `Metered` to:

```rust
struct Metered {
    card: Option<MeterId>,
    linux: Option<LinuxMeterId>,
    meter: Meter,
}
```

In `build_strip`, build the meter when `strip.meter.is_some() || strip.linux_meter.is_some()`:

```rust
let meter = (strip.meter.is_some() || strip.linux_meter.is_some()).then(|| {
    let meter = Meter::new(MeterSpec {
        label: if strip.meter.is_some() { "CARD" } else { "PW" }.to_string(),
        card: strip.meter.and_then(|id| meter_level(state, id)),
        card_present: strip.meter.is_some(),
        linux: None,
        peak: 0.0,
        linux_present: false,
    });
    body.append(meter.widget());
    Metered { card: strip.meter, linux: strip.linux_meter, meter }
});
```

Do the same in `build_output`, with `card: output.meter`, `card_present: output.meter.is_some()`, `linux: output.linux_meter`, and the same label rule.

Replace `set_meter` with:

```rust
pub(crate) fn set_meter(&self, id: MeterId, steps: Option<i64>) {
    let level = level_from_steps(steps);
    for metered in &self.strips {
        if metered.card == Some(id) {
            metered.meter.set_card(level);
        }
    }
    if let Some(metered) = &self.output.meter {
        if metered.card == Some(id) {
            metered.meter.set_card(level);
        }
    }
}

/// Installs one Linux reading.
pub(crate) fn set_linux_meter(
    &self,
    id: LinuxMeterId,
    available: bool,
    level: Option<f64>,
) {
    for metered in self.strips.iter().filter_map(|strip| strip.meter.as_ref()) {
        if metered.linux == Some(id) {
            metered.meter.set_linux_source(available, level);
        }
    }
}
```

Add a helper used by the application when the card leaves:

```rust
/// Hides every Linux bar and drops its last reading.
pub(crate) fn clear_linux_meters(&self) {
    for metered in self.strips.iter().filter_map(|strip| strip.meter.as_ref()) {
        if metered.linux.is_some() {
            metered.meter.set_linux_source(false, None);
        }
    }
    if let Some(metered) = &self.output.meter {
        if metered.linux.is_some() {
            metered.meter.set_linux_source(false, None);
        }
    }
}
```

The `StripWidgets.meter` and `StreamOutputWidgets.meter` types stay `Option<Metered>`.

- [ ] **Step 2: Update the Micro screen**

In `mic.rs`, import `LinuxMeterId`. Change `Metered` to:

```rust
struct Metered {
    id: MeterId,
    linux: Option<LinuxMeterId>,
    meter: Meter,
}
```

In `build_meter`, set `linux: (id == MeterId::MicIn).then_some(LinuxMeterId::MicIn)`, add `card_present: true` to the `MeterSpec`, and return `Metered { id, linux, meter }`.

Replace `set_meter`:

```rust
pub(crate) fn set_meter(&self, id: MeterId, steps: Option<i64>) {
    let level = level_from_steps(steps);
    for metered in &self.meters {
        if metered.id == id {
            metered.meter.set_card(level);
        }
    }
}

/// Installs the `MIC IN` Linux reading; `MIC OUT` has no Linux source.
pub(crate) fn set_linux_meter(
    &self,
    id: LinuxMeterId,
    available: bool,
    level: Option<f64>,
) {
    for metered in &self.meters {
        if metered.linux == Some(id) {
            metered.meter.set_linux_source(available, level);
        }
    }
}

/// Hides the Linux bars when the card leaves.
pub(crate) fn clear_linux_meters(&self) {
    for metered in &self.meters {
        if metered.linux.is_some() {
            metered.meter.set_linux_source(false, None);
        }
    }
}
```

- [ ] **Step 3: Update the application**

First delete the temporary `Meter::set_levels` wrapper from `meter.rs`; no caller remains after Steps 1 and 2.

In `app.rs`:
- Import `LinuxMeterId` from `zg01_core::mixage`.
- Import `spawn_linux_meters, LinuxMetersSnapshot, LINUX_METER_INTERVAL` from `zg01_core::pipewire`.
- Add field `linux_meters_stop: Option<Arc<AtomicBool>>` and initialize it to `None`.
- Add `Msg::LinuxMeters(LinuxMetersSnapshot)` and dispatch it to `self.update_linux_meters(snapshot)`.
- Change `Msg::Quit` to call `self.stop_meters()` and `self.stop_writer()` before `relm4::main_application().quit()`.
- In `start_meters`, after starting the card pump, call `self.start_linux_meters(sender);`.
- In `stop_meters`, also call `self.stop_linux_meters()`.
- In `disconnect`, after `self.stop_meters()`, call `self.clear_linux_meters()`.

Implement:

```rust
/// Starts the PipeWire source poller and its forwarding pump.
fn start_linux_meters(&mut self, sender: &ComponentSender<Self>) {
    self.stop_linux_meters();
    let stop = Arc::new(AtomicBool::new(false));
    self.linux_meters_stop = Some(stop.clone());
    let events = spawn_linux_meters(self.device_generation, LINUX_METER_INTERVAL, stop.clone());
    let sender = sender.clone();
    let spawned = std::thread::Builder::new()
        .name("zg01-linux-meter-events".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match events.recv_timeout(PUMP_TICK) {
                    Ok(snapshot) => sender.input(Msg::LinuxMeters(snapshot)),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("could not start the Linux meter event thread: {err}");
    }
}

/// Asks the PipeWire pump to stop; the worker reaps its children when
/// the receiver is dropped.
fn stop_linux_meters(&mut self) {
    if let Some(stop) = self.linux_meters_stop.take() {
        stop.store(true, Ordering::Relaxed);
    }
}

/// Installs the Linux readings on the screens.
fn update_linux_meters(&mut self, snapshot: LinuxMetersSnapshot) {
    if !self.state.device.is_connected() {
        return;
    }
    if !snapshot.belongs_to(self.device_generation) {
        tracing::debug!(
            "ignoring a Linux meter snapshot from generation {} (current is {})",
            snapshot.generation,
            self.device_generation
        );
        return;
    }
    for id in LinuxMeterId::ALL {
        let available = snapshot.is_available(id);
        let level = snapshot.level(id);
        if let Some(screen) = &self.mixage {
            screen.set_linux_meter(id, available, level);
        }
        if let Some(screen) = &self.mic {
            screen.set_linux_meter(id, available, level);
        }
    }
}

/// Hides the Linux bars without touching the card meters.
fn clear_linux_meters(&mut self) {
    if let Some(screen) = &self.mixage {
        screen.clear_linux_meters();
    }
    if let Some(screen) = &self.mic {
        screen.clear_linux_meters();
    }
}
```

- [ ] **Step 4: Build and test**

Run:
```bash
cd gui
cargo fmt
cargo test --workspace
cargo build --workspace
```
Expected: all tests pass, the workspace builds.

- [ ] **Step 5: Commit**

```bash
git add gui/crates/zg01-gui/src/ui/mixage.rs gui/crates/zg01-gui/src/ui/mic.rs gui/crates/zg01-gui/src/app.rs
git commit -m "feat(gui): show PipeWire VU meters beside the card meters"
```

---

### Task 6: Document the behavior and run the hardware acceptance

**Files:**
- Modify: `README.md:178-201, 274-281`
- Modify: `gui/README.md:108-128`
- Modify: `docs/GUI_TEST.md:39-61`

**Interfaces:**
- Consumes: everything from Tasks 1–5.

- [ ] **Step 1: Update the user docs**

In `README.md`, replace the "the Linux-side PipeWire bars are a v1.1 item and are not shown" sentences with: the GUI shows a thin `PW` bar beside the card bar for MIC (`Voice In`), VOICE (`Voice Out`) and GAME (`Game Out`); STREAM OUT has no Linux source; the bars use the card dB scale and are hidden when PipeWire or a node is absent.

In `gui/README.md`, replace the blocked-live-meter paragraph with: the card meters are still blocked, and the GUI now also shows PipeWire `PW` bars so the interface has live levels; the card bars stay dashed until EP `0x83` returns.

In `docs/GUI_TEST.md` section 2, add the expected PW behavior: MIC, VOICE and GAME `PW` bars move; STREAM OUT stays dashed; a source with no frames is dashed; closing the window leaves no `pw-record`.

- [ ] **Step 2: Run the full automated checks**

Run:
```bash
cd gui
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
```
Expected: every command exits 0. If `clippy` is not installed, record that and run the rest.

- [ ] **Step 3: Run the hardware acceptance**

Generate the test tone once:

```bash
python3 - <<'PY'
import math, struct, wave
sr, dur, amp = 48000, 6, int(32767 * 10 ** (-30 / 20))
with wave.open("/tmp/zg01-tone.wav", "wb") as out:
    out.setnchannels(2)
    out.setsampwidth(2)
    out.setframerate(sr)
    out.writeframes(b"".join(
        struct.pack("<hh", v, v)
        for i in range(sr * dur)
        for v in [int(amp * math.sin(2 * math.pi * 1000 * i / sr))]
    ))
PY
```

With PipeWire running and the ZG01 connected:

```bash
cd gui
cargo run -p zg01-gui
# while it runs, in another shell (69/70 are the serials from wpctl status):
pw-play --target 70 /tmp/zg01-tone.wav   # Game Out: GAME PW bar moves
pw-play --target 69 /tmp/zg01-tone.wav   # Voice Out: VOICE PW bar moves
# speak into the microphone: MIC PW bar moves
pgrep -af pw-record                      # three processes while running
# close the window
pgrep -af pw-record                      # empty
```

Expected: the three PW bars move, STREAM OUT is dashed, the card bars stay dashed, closing the window reaps every `pw-record`, and the app logs no panic.

- [ ] **Step 4: Commit**

```bash
git add README.md gui/README.md docs/GUI_TEST.md
git commit -m "docs(gui): document the PipeWire VU meters"
```
