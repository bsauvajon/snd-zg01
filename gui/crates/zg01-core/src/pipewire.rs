//! The ZG01's Linux-side meter sources, read through PipeWire.
//!
//! The card meters on EP `0x83` are the device's own levels. When they are
//! unavailable, the GUI falls back to the PipeWire graph: `Voice In` for
//! the microphone, the monitor ports of `Voice Out` and `Game Out` for the
//! two playback paths, and `Streaming In` for the device's stream mix. One
//! `pw-record` process per source produces raw `f32` stereo, reduced to a
//! 50 ms peak and mapped onto the card meter's dB scale. Float samples are
//! required because the card scale reaches -127 dBFS, below the s16 LSB:
//! a quiet microphone captured as s16 reads exactly zero.
//!
//! A missing source has no bar. A source that exists but has no reading
//! is `None`, which the GUI draws dashed; only a captured silent buffer
//! reads zero.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::Value;

use crate::mixage::LinuxMeterId;

/// How often the Linux meters are published.
pub const LINUX_METER_INTERVAL: Duration = Duration::from_millis(50);

/// How long a recorder may stop delivering frames before its reading drops.
///
/// A `pw-record` that stays alive but stops producing data (paused stream,
/// suspended target) would otherwise freeze its bar at the last level.
pub const METER_STALE: Duration = Duration::from_secs(1);

/// The PipeWire object serials of the four ZG01 sources.
///
/// `pw-record --target` resolves these serials, not the global node ids:
/// the two values differ in a live graph, and targeting the id lands on
/// another node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nodes {
    /// The `Voice In` source.
    pub mic_in: Option<u64>,
    /// The `Voice Out` sink monitor.
    pub voice_out: Option<u64>,
    /// The `Game Out` sink monitor.
    pub game_out: Option<u64>,
    /// The `Streaming In` capture source.
    pub streaming_in: Option<u64>,
}

/// The PipeWire command-line tools the poller runs.
///
/// The defaults are the system tools; the tests substitute scripts so the
/// workers can be exercised without PipeWire or a ZG01.
#[derive(Debug, Clone)]
pub struct Tools {
    /// The `pw-dump` executable.
    pub pw_dump: PathBuf,
    /// The `pw-record` executable.
    pub pw_record: PathBuf,
}

impl Default for Tools {
    fn default() -> Self {
        Tools {
            pw_dump: PathBuf::from("pw-dump"),
            pw_record: PathBuf::from("pw-record"),
        }
    }
}

/// Why the PipeWire sources could not be discovered.
#[derive(Debug, thiserror::Error)]
pub enum PipewireError {
    /// The tool could not be started.
    #[error("could not run {tool}: {source}")]
    Run {
        /// The command that failed.
        tool: &'static str,
        /// The OS error.
        source: std::io::Error,
    },
    /// The tool ran but exited unsuccessfully.
    #[error("{tool} failed: {status}")]
    Status {
        /// The command that failed.
        tool: &'static str,
        /// Its exit status.
        status: std::process::ExitStatus,
    },
    /// `pw-dump` did not produce the expected JSON.
    #[error("could not parse pw-dump: {0}")]
    Json(#[from] serde_json::Error),
    /// The tool did not finish before its deadline or the stop flag.
    #[error("{tool} did not finish in time")]
    Timeout {
        /// The command that timed out.
        tool: &'static str,
    },
}

/// How many Linux meter sources the pump records.
pub const LINUX_METER_COUNT: usize = 4;

/// The four Linux meter readings at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinuxMetersSnapshot {
    /// The level of each source, in `LinuxMeterId` slot order.
    ///
    /// `None` means the source exists but has no reading yet.
    pub values: [Option<f64>; LINUX_METER_COUNT],
    /// Whether each source exists, independently of its reading.
    pub available: [bool; LINUX_METER_COUNT],
    /// The backend generation that produced the snapshot.
    pub generation: u64,
}

impl LinuxMetersSnapshot {
    /// The level of `id`, when it has a reading.
    pub fn level(self, id: LinuxMeterId) -> Option<f64> {
        self.values[id.index()]
    }

    /// Whether `id` exists, even without a reading.
    pub fn is_available(self, id: LinuxMeterId) -> bool {
        self.available[id.index()]
    }

    /// Whether this snapshot describes `generation`.
    pub fn belongs_to(self, generation: u64) -> bool {
        self.generation == generation
    }
}

/// The arguments of `pw-record` for one source.
///
/// Sinks capture their monitor port; the `Voice In` source is captured
/// directly, so it must not set `stream.capture.sink`.
///
/// `node.dont-reconnect` stops WirePlumber from moving the capture when it
/// changes the default node: without it, switching the default sink (or
/// source) mid-playback re-targets every monitor capture to the new default,
/// so the `GAME` bar would keep reading the `VOICE` audio.
/// `node.dont-fallback` covers the other case, a target that is missing: a
/// bar must never be fed by an unrelated node. A target that really
/// disappears still gets a fresh recorder, because discovery notices the new
/// serial.
pub fn recorder_args(node: u64, sink: bool) -> Vec<String> {
    let mut args = vec!["--target".to_string(), node.to_string()];
    if sink {
        args.push("-P".to_string());
        args.push("stream.capture.sink=true".to_string());
    }
    for pinned in ["node.dont-reconnect=true", "node.dont-fallback=true"] {
        args.push("-P".to_string());
        args.push(pinned.to_string());
    }
    args.extend([
        "--rate".to_string(),
        "48000".to_string(),
        "--channels".to_string(),
        "2".to_string(),
        "--format".to_string(),
        "f32".to_string(),
        "--latency".to_string(),
        "20ms".to_string(),
        "--raw".to_string(),
        "-".to_string(),
    ]);
    args
}

/// Maps a peak sample to the card meter's `0.0..=1.0` dB scale.
///
/// The card controls are `dB + 127` over `0..=127`; using the same scale
/// makes the two bars comparable. Silence is the bottom of the bar; a
/// non-finite sample (raw f32 can carry `NaN`/`inf`) also maps to the floor.
pub fn level_from_peak(peak: f32) -> f64 {
    if !peak.is_finite() || peak <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * f64::from(peak).log10();
    ((db.clamp(-127.0, 0.0) + 127.0) / 127.0).clamp(0.0, 1.0)
}

/// Finds the four ZG01 nodes in a `pw-dump` JSON document.
///
/// A node is matched by its UCM `node.name` suffix, with the human
/// description as a fallback. Unrelated nodes and missing sources are
/// left `None`, never guessed.
pub fn parse_pw_dump(json: &str) -> Result<Nodes, PipewireError> {
    let dump: Value = serde_json::from_str(json)?;
    let mut nodes = Nodes::default();
    let Some(objects) = dump.as_array() else {
        return Ok(nodes);
    };
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
        } else if name.ends_with("StreamingIn__source") || description == "Yamaha ZG01 Streaming In"
        {
            nodes.streaming_in = nodes.streaming_in.or(Some(serial));
        }
    }
    Ok(nodes)
}

/// Runs `pw-dump` and parses the ZG01 nodes.
pub fn discover_nodes() -> Result<Nodes, PipewireError> {
    let output = Command::new("pw-dump")
        .output()
        .map_err(|source| PipewireError::Run {
            tool: "pw-dump",
            source,
        })?;
    if !output.status.success() {
        return Err(PipewireError::Status {
            tool: "pw-dump",
            status: output.status,
        });
    }
    parse_pw_dump(&String::from_utf8_lossy(&output.stdout))
}

/// The stop flag and join handle of a running Linux meter poller.
///
/// Stopping the handle sets the flag and waits for the coordinator to kill
/// and reap every `pw-record` child, so no recorder survives a window
/// close, a disconnect or a quit.
pub struct LinuxMetersHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl LinuxMetersHandle {
    /// Stops the poller and waits until every recorder is reaped.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(feature = "pipewire")]
use std::io::Read;
#[cfg(feature = "pipewire")]
use std::process::{Child, Stdio};
#[cfg(feature = "pipewire")]
use std::sync::mpsc::Sender;
#[cfg(feature = "pipewire")]
use std::sync::Mutex;
#[cfg(feature = "pipewire")]
use std::time::Instant;

#[cfg(feature = "pipewire")]
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(2);
#[cfg(feature = "pipewire")]
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(feature = "pipewire")]
const RECORDER_HEALTHY: Duration = Duration::from_secs(5);
#[cfg(feature = "pipewire")]
const RECORDER_BACKOFF_START: Duration = Duration::from_secs(1);
#[cfg(feature = "pipewire")]
const RECORDER_BACKOFF_MAX: Duration = Duration::from_secs(5);

#[cfg(feature = "pipewire")]
type Update = (LinuxMeterId, Option<f64>);

#[cfg(feature = "pipewire")]
struct Recorder {
    node: u64,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    join: JoinHandle<()>,
}

#[cfg(feature = "pipewire")]
impl Recorder {
    fn start(
        id: LinuxMeterId,
        node: u64,
        sink: bool,
        record_tool: PathBuf,
        updates: Sender<Update>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let thread_stop = stop.clone();
        let thread_child = child.clone();
        let join = std::thread::Builder::new()
            .name(format!("zg01-pw-{}", id.index()))
            .spawn(move || {
                recorder_loop(
                    id,
                    node,
                    sink,
                    record_tool,
                    thread_stop,
                    thread_child,
                    updates,
                )
            })
            .expect("could not start a PipeWire recorder");
        Recorder {
            node,
            stop,
            child,
            join,
        }
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

/// The next retry delay: reset only after a healthy run, otherwise double
/// up to five seconds.
#[cfg(feature = "pipewire")]
fn next_backoff(current: Duration, healthy: bool) -> Duration {
    if healthy {
        RECORDER_BACKOFF_START
    } else {
        (current * 2).min(RECORDER_BACKOFF_MAX)
    }
}

#[cfg(feature = "pipewire")]
fn recorder_loop(
    id: LinuxMeterId,
    node: u64,
    sink: bool,
    record_tool: PathBuf,
    stop: Arc<AtomicBool>,
    slot: Arc<Mutex<Option<Child>>>,
    updates: Sender<Update>,
) {
    let mut backoff = RECORDER_BACKOFF_START;
    while !stop.load(Ordering::Relaxed) {
        let mut command = Command::new(&record_tool);
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
                backoff = next_backoff(backoff, false);
                continue;
            }
        };
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            let _ = updates.send((id, None));
            sleep_or_stop(&stop, backoff);
            backoff = next_backoff(backoff, false);
            continue;
        };
        *slot.lock().expect("the child slot") = Some(child);
        let started = Instant::now();

        let mut buffer = [0_u8; 4096];
        let mut peak = 0_f32;
        let mut carry = [0_u8; 4];
        let mut carry_len = 0_usize;
        let mut window = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(len) => {
                    let mut first = 0;
                    if carry_len > 0 {
                        let take = (4 - carry_len).min(len);
                        carry[carry_len..carry_len + take].copy_from_slice(&buffer[..take]);
                        carry_len += take;
                        first = take;
                        if carry_len == 4 {
                            peak = peak.max(f32::from_ne_bytes(carry).abs());
                            carry_len = 0;
                        }
                    }
                    let end = first + (len - first) / 4 * 4;
                    for sample in buffer[first..end].as_chunks::<4>().0 {
                        peak = peak.max(f32::from_ne_bytes(*sample).abs());
                    }
                    if end < len {
                        carry[..len - end].copy_from_slice(&buffer[end..]);
                        carry_len = len - end;
                    }
                    if window.elapsed() >= LINUX_METER_INTERVAL {
                        if updates.send((id, Some(level_from_peak(peak)))).is_err() {
                            stop_child(&slot);
                            return;
                        }
                        peak = 0.0;
                        window = Instant::now();
                    }
                }
                Err(_) => break,
            }
        }
        let healthy = started.elapsed() >= RECORDER_HEALTHY;
        stop_child(&slot);
        let _ = updates.send((id, None));
        sleep_or_stop(&stop, backoff);
        backoff = next_backoff(backoff, healthy);
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

/// Runs `pw-dump` with a deadline and a stop check.
///
/// The output is read on a helper thread so a full pipe never deadlocks the
/// wait. On timeout or stop, the child is killed and reaped; the caller
/// treats the error as "keep the current recorders".
#[cfg(feature = "pipewire")]
fn run_pw_dump(
    tools: &Tools,
    stop: &AtomicBool,
    timeout: Duration,
) -> Result<Nodes, PipewireError> {
    let mut child = Command::new(&tools.pw_dump)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| PipewireError::Run {
            tool: "pw-dump",
            source,
        })?;
    let reader = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = stdout.read_to_end(&mut buffer);
            buffer
        })
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PipewireError::Run {
                    tool: "pw-dump",
                    source,
                });
            }
        }
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    match status {
        Some(status) if status.success() => parse_pw_dump(&String::from_utf8_lossy(&output)),
        Some(status) => Err(PipewireError::Status {
            tool: "pw-dump",
            status,
        }),
        None => Err(PipewireError::Timeout { tool: "pw-dump" }),
    }
}

/// Starts the Linux meter poller with the system tools.
#[cfg(feature = "pipewire")]
pub fn spawn_linux_meters(
    generation: u64,
    interval: Duration,
    stop: Arc<AtomicBool>,
) -> (Receiver<LinuxMetersSnapshot>, LinuxMetersHandle) {
    spawn_linux_meters_with(
        generation,
        Timings {
            interval,
            discovery: DISCOVERY_INTERVAL,
            timeout: DISCOVERY_TIMEOUT,
            stale: METER_STALE,
        },
        stop,
        Tools::default(),
    )
}

/// Starts the poller with explicit tools and timing knobs.
#[cfg(feature = "pipewire")]
fn spawn_linux_meters_with(
    generation: u64,
    timings: Timings,
    stop: Arc<AtomicBool>,
    tools: Tools,
) -> (Receiver<LinuxMetersSnapshot>, LinuxMetersHandle) {
    let (snapshots, receiver) = mpsc::channel();
    let thread_stop = stop.clone();
    let join = std::thread::Builder::new()
        .name("zg01-pw-meters".to_string())
        .spawn(move || coordinator_loop(generation, timings, thread_stop, tools, snapshots))
        .expect("could not start the PipeWire meter poller");
    (
        receiver,
        LinuxMetersHandle {
            stop,
            join: Some(join),
        },
    )
}

/// The no-feature stub: a disconnected receiver and a no-op handle.
#[cfg(not(feature = "pipewire"))]
pub fn spawn_linux_meters(
    _generation: u64,
    _interval: Duration,
    stop: Arc<AtomicBool>,
) -> (Receiver<LinuxMetersSnapshot>, LinuxMetersHandle) {
    let (_sender, receiver) = mpsc::channel();
    (receiver, LinuxMetersHandle { stop, join: None })
}

/// The PipeWire node that feeds one Linux meter.
///
/// One arm per `LinuxMeterId`: adding a variant without a source is a
/// compile error here instead of a silently silent slot.
fn node_for(id: LinuxMeterId, nodes: &Nodes) -> Option<u64> {
    match id {
        LinuxMeterId::MicIn => nodes.mic_in,
        LinuxMeterId::VoiceOut => nodes.voice_out,
        LinuxMeterId::GameOut => nodes.game_out,
        LinuxMeterId::StreamingIn => nodes.streaming_in,
    }
}

/// Whether a meter reads a sink monitor rather than a capture source.
fn is_sink(id: LinuxMeterId) -> bool {
    match id {
        LinuxMeterId::MicIn | LinuxMeterId::StreamingIn => false,
        LinuxMeterId::VoiceOut | LinuxMeterId::GameOut => true,
    }
}

/// The timing knobs of the meter poller.
#[cfg(feature = "pipewire")]
#[derive(Debug, Clone, Copy)]
struct Timings {
    /// How often a snapshot is published.
    interval: Duration,
    /// How often the graph is rediscovered.
    discovery: Duration,
    /// The deadline of one `pw-dump` run.
    timeout: Duration,
    /// How long a recorder may stay silent before its reading is dropped.
    stale: Duration,
}

#[cfg(feature = "pipewire")]
fn coordinator_loop(
    generation: u64,
    timings: Timings,
    stop: Arc<AtomicBool>,
    tools: Tools,
    snapshots: Sender<LinuxMetersSnapshot>,
) {
    let Timings {
        interval,
        discovery,
        timeout,
        stale,
    } = timings;
    let (updates, receiver) = mpsc::channel::<Update>();
    let mut available = [false; LINUX_METER_COUNT];
    let mut values: [Option<f64>; LINUX_METER_COUNT] = [None; LINUX_METER_COUNT];
    let mut last_update: [Option<Instant>; LINUX_METER_COUNT] = [None; LINUX_METER_COUNT];
    let mut recorders: [Option<Recorder>; LINUX_METER_COUNT] = Default::default();
    let mut next_discovery = Instant::now();
    let mut last_send: Option<Instant> = None;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if Instant::now() >= next_discovery {
            next_discovery = Instant::now() + discovery;
            match run_pw_dump(&tools, &stop, timeout) {
                Ok(nodes) => {
                    for id in LinuxMeterId::ALL {
                        let index = id.index();
                        let node = node_for(id, &nodes);
                        let sink = is_sink(id);
                        let current = recorders[index].as_ref().map(|recorder| recorder.node);
                        if current == node {
                            if node.is_some() {
                                available[index] = true;
                            }
                            continue;
                        }
                        if let Some(recorder) = recorders[index].take() {
                            recorder.stop();
                        }
                        values[index] = None;
                        last_update[index] = None;
                        available[index] = node.is_some();
                        if let Some(node) = node {
                            recorders[index] = Some(Recorder::start(
                                id,
                                node,
                                sink,
                                tools.pw_record.clone(),
                                updates.clone(),
                            ));
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
                last_update[id.index()] = Some(Instant::now());
                while let Ok((id, value)) = receiver.try_recv() {
                    values[id.index()] = value;
                    last_update[id.index()] = Some(Instant::now());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let due = last_send.is_none_or(|sent| sent.elapsed() >= interval);
        if due {
            let now = Instant::now();
            for (index, since) in last_update.iter_mut().enumerate() {
                if since.is_some_and(|at| now.duration_since(at) > stale) {
                    values[index] = None;
                    *since = None;
                }
            }
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

    for recorder in recorders.into_iter().flatten() {
        recorder.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_sink, level_from_peak, node_for, parse_pw_dump, recorder_args, LinuxMetersSnapshot,
        LINUX_METER_COUNT,
    };
    use crate::mixage::LinuxMeterId;

    // Node ids and serials differ in a live PipeWire graph. `pw-record
    // --target` resolves the object serial, so the ids must not be used.
    const DUMP: &str = r#"[
      {"id": 71, "type": "PipeWire:Interface:Node",
       "info": {"props": {"node.name": "alsa_output.usb-Yamaha_Corporation_Yamaha_ZG01-01.HiFi__VoiceOut__sink",
                          "node.description": "Yamaha ZG01 Voice Out", "object.serial": "72"}}},
      {"id": 72, "type": "PipeWire:Interface:Node",
       "info": {"props": {"node.name": "alsa_output.usb-Yamaha_Corporation_Yamaha_ZG01-01.HiFi__Game__sink",
                          "node.description": "Yamaha ZG01 Game Out", "object.serial": "73"}}},
      {"id": 73, "type": "PipeWire:Interface:Node",
       "info": {"props": {"node.name": "alsa_input.usb-Yamaha_Corporation_Yamaha_ZG01-01.HiFi__VoiceIn__source",
                          "node.description": "Yamaha ZG01 Voice In", "object.serial": "74"}}},
      {"id": 74, "type": "PipeWire:Interface:Node",
       "info": {"props": {"node.name": "alsa_input.usb-Yamaha_Corporation_Yamaha_ZG01-01.HiFi__StreamingIn__source",
                          "node.description": "Yamaha ZG01 Streaming In", "object.serial": "75"}}},
      {"id": 12, "type": "PipeWire:Interface:Node",
       "info": {"props": {"node.name": "alsa_output.pci-0000_00_1f.3.analog-stereo",
                          "object.serial": "13"}}}
    ]"#;

    #[test]
    fn the_dump_selects_the_four_zg01_nodes() {
        let nodes = parse_pw_dump(DUMP).expect("a valid dump");
        assert_eq!(nodes.mic_in, Some(74));
        assert_eq!(nodes.voice_out, Some(72));
        assert_eq!(nodes.game_out, Some(73));
        assert_eq!(nodes.streaming_in, Some(75));
    }

    #[test]
    fn every_meter_reads_its_own_node_and_kind() {
        // A swapped arm in `node_for` feeds one bar with another source's
        // audio and nothing else fails, so the mapping is pinned here.
        let nodes = parse_pw_dump(DUMP).expect("a valid dump");
        for (id, serial, sink) in [
            (LinuxMeterId::MicIn, 74, false),
            (LinuxMeterId::VoiceOut, 72, true),
            (LinuxMeterId::GameOut, 73, true),
            (LinuxMeterId::StreamingIn, 75, false),
        ] {
            assert_eq!(node_for(id, &nodes), Some(serial), "{id:?} node");
            assert_eq!(is_sink(id), sink, "{id:?} kind");
        }
    }

    #[test]
    fn every_linux_meter_has_a_distinct_slot() {
        let mut seen = [false; LINUX_METER_COUNT];
        for id in LinuxMeterId::ALL {
            let index = id.index();
            assert!(index < seen.len(), "{id:?} indexes past the snapshot");
            assert!(!seen[index], "duplicate slot {index}");
            seen[index] = true;
        }
        assert!(
            seen.iter().all(|used| *used),
            "a snapshot slot has no meter"
        );
    }

    #[test]
    fn unrelated_nodes_and_missing_nodes_are_not_invented() {
        let nodes = parse_pw_dump("[]").expect("an empty dump");
        assert_eq!(nodes.mic_in, None);
        assert_eq!(nodes.voice_out, None);
        assert_eq!(nodes.game_out, None);
        assert_eq!(nodes.streaming_in, None);
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
        assert!(sink.iter().any(|arg| arg == "f32"));
        assert!(sink.iter().any(|arg| arg == "48000"));

        let source = recorder_args(71, false);
        assert!(!source.iter().any(|arg| arg == "stream.capture.sink=true"));
    }

    #[test]
    fn a_recorder_is_pinned_to_its_target() {
        // WirePlumber otherwise moves a monitor capture to the new default
        // node, so the GAME bar would show the VOICE audio after a switch.
        for sink in [true, false] {
            let args = recorder_args(70, sink);
            for pinned in ["node.dont-reconnect=true", "node.dont-fallback=true"] {
                assert!(
                    args.windows(2)
                        .any(|pair| pair[0] == "-P" && pair[1] == pinned),
                    "{pinned} missing from {args:?}"
                );
            }
        }
    }

    #[test]
    fn peaks_use_the_card_db_scale() {
        assert_eq!(level_from_peak(0.0), 0.0);
        assert_eq!(level_from_peak(f32::NAN), 0.0);
        assert!((level_from_peak(1.0) - 1.0).abs() < 1e-12);
        // -30 dBFS is 97 steps on the 0..127 dB scale.
        let minus_thirty = level_from_peak(10f32.powf(-30.0 / 20.0));
        assert!((minus_thirty * 127.0 - 97.0).abs() < 0.01);
        // The scale reaches below the s16 LSB; float keeps the reading.
        assert!(level_from_peak(1e-5) > 0.0);
    }

    #[test]
    fn a_snapshot_reads_by_id_and_generation() {
        let snapshot = LinuxMetersSnapshot {
            values: [Some(0.25), None, Some(0.75), None],
            available: [true, true, false, false],
            generation: 7,
        };
        assert_eq!(snapshot.level(LinuxMeterId::MicIn), Some(0.25));
        assert_eq!(snapshot.level(LinuxMeterId::VoiceOut), None);
        assert!(snapshot.is_available(LinuxMeterId::VoiceOut));
        assert!(!snapshot.is_available(LinuxMeterId::GameOut));
        assert!(!snapshot.is_available(LinuxMeterId::StreamingIn));
        assert!(snapshot.belongs_to(7));
        assert!(!snapshot.belongs_to(8));
    }

    #[cfg(feature = "pipewire")]
    #[test]
    fn stopping_a_recorder_kills_its_child() {
        use std::process::Command;
        use std::sync::{Arc, Mutex};
        let slot = Arc::new(Mutex::new(Some(
            Command::new("sh")
                .arg("-c")
                .arg("exec sleep 30")
                .spawn()
                .expect("a child"),
        )));
        super::stop_child(&slot);
        assert!(slot.lock().expect("the slot").is_none());
    }

    #[cfg(feature = "pipewire")]
    #[test]
    fn the_recorder_backoff_reaches_the_cap() {
        use std::time::Duration;
        assert_eq!(
            super::next_backoff(Duration::from_secs(1), false),
            Duration::from_secs(2)
        );
        assert_eq!(
            super::next_backoff(Duration::from_secs(2), false),
            Duration::from_secs(4)
        );
        assert_eq!(
            super::next_backoff(Duration::from_secs(4), false),
            Duration::from_secs(5)
        );
        assert_eq!(
            super::next_backoff(Duration::from_secs(5), false),
            Duration::from_secs(5)
        );
        assert_eq!(
            super::next_backoff(Duration::from_secs(5), true),
            Duration::from_secs(1)
        );
    }

    #[cfg(feature = "pipewire")]
    mod live {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use super::super::{spawn_linux_meters_with, Tools};

        static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

        fn temp_dir() -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "zg01-pw-test-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a test directory");
            dir
        }

        fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
            let path = dir.join(name);
            fs::write(&path, body).expect("a script");
            let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("script permissions");
            path
        }

        fn nodes_json(serial: u64) -> String {
            format!(
                r#"[{{"id":{},"info":{{"props":{{"node.name":"alsa_input.usb-ZG01-VoiceIn__source",
                    "node.description":"Yamaha ZG01 Voice In","object.serial":"{serial}"}}}}}}]"#,
                serial + 1
            )
        }

        /// A fake `pw-dump` that prints `nodes.json`, and a fake
        /// `pw-record` that records its target and then idles.
        fn tools(dir: &Path) -> Tools {
            tools_with(dir, "exec sleep 30")
        }

        /// The same tools, with a custom tail for the fake `pw-record`.
        fn tools_with(dir: &Path, tail: &str) -> Tools {
            let dump = script(
                dir,
                "pw-dump",
                &format!("#!/bin/sh\ncat \"{}/nodes.json\"\n", dir.display()),
            );
            let record = script(
                dir,
                "pw-record",
                &format!(
                    "#!/bin/sh\ntarget=\"$2\"\necho \"$target\" >> \"{}/targets.log\"\necho $$ > \"{}/pid-$target\"\n{tail}\n",
                    dir.display(),
                    dir.display()
                ),
            );
            Tools {
                pw_dump: dump,
                pw_record: record,
            }
        }

        fn wait_for_pid(dir: &Path, serial: u64) -> u32 {
            let path = dir.join(format!("pid-{serial}"));
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if let Ok(text) = fs::read_to_string(&path) {
                    if let Ok(pid) = text.trim().parse::<u32>() {
                        return pid;
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            panic!("the fake recorder {serial} never started");
        }

        fn alive(pid: u32) -> bool {
            Path::new("/proc").join(pid.to_string()).exists()
        }

        fn spawn(
            dir: &Path,
            serial: u64,
            discovery: Duration,
            timeout: Duration,
        ) -> (
            std::sync::mpsc::Receiver<super::super::LinuxMetersSnapshot>,
            super::super::LinuxMetersHandle,
            Arc<AtomicBool>,
        ) {
            spawn_with(
                dir,
                serial,
                super::super::Timings {
                    interval: Duration::from_millis(50),
                    discovery,
                    timeout,
                    stale: Duration::from_secs(30),
                },
                tools(dir),
            )
        }

        /// Spawns the poller with explicit tools and timing knobs.
        fn spawn_with(
            dir: &Path,
            serial: u64,
            timings: super::super::Timings,
            tools: Tools,
        ) -> (
            std::sync::mpsc::Receiver<super::super::LinuxMetersSnapshot>,
            super::super::LinuxMetersHandle,
            Arc<AtomicBool>,
        ) {
            fs::write(dir.join("nodes.json"), nodes_json(serial)).expect("nodes");
            let stop = Arc::new(AtomicBool::new(false));
            let (receiver, handle) = spawn_linux_meters_with(1, timings, stop.clone(), tools);
            (receiver, handle, stop)
        }

        #[test]
        fn stopping_the_poller_reaps_an_idle_recorder() {
            let dir = temp_dir();
            let (receiver, mut handle, _stop) = spawn(
                &dir,
                1,
                Duration::from_millis(50),
                Duration::from_millis(200),
            );
            let pid = wait_for_pid(&dir, 1);
            assert!(alive(pid));
            handle.stop();
            assert!(!alive(pid), "the idle recorder survived the stop");
            drop(receiver);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_removed_node_stops_its_recorder() {
            let dir = temp_dir();
            let (receiver, mut handle, _stop) = spawn(
                &dir,
                1,
                Duration::from_millis(100),
                Duration::from_millis(200),
            );
            let pid = wait_for_pid(&dir, 1);
            fs::write(dir.join("nodes.json"), "[]").expect("empty nodes");
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut removed = false;
            while Instant::now() < deadline {
                if let Ok(snapshot) = receiver.recv_timeout(Duration::from_millis(100)) {
                    if !snapshot.is_available(super::super::LinuxMeterId::MicIn) {
                        removed = true;
                        break;
                    }
                }
            }
            assert!(removed, "the removed node stayed available");
            assert!(!alive(pid), "the removed node kept its recorder");
            handle.stop();
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_recorder_that_stops_delivering_drops_its_reading() {
            let dir = temp_dir();
            // Two 8-byte f32 bursts ~200 ms apart publish one reading, then
            // the recorder stays alive without producing frames.
            let stall = "head -c 8 /dev/zero; sleep 0.2; head -c 8 /dev/zero; exec sleep 30";
            let (receiver, mut handle, _stop) = spawn_with(
                &dir,
                1,
                super::super::Timings {
                    interval: Duration::from_millis(50),
                    discovery: Duration::from_millis(50),
                    timeout: Duration::from_millis(200),
                    stale: Duration::from_millis(300),
                },
                tools_with(&dir, stall),
            );
            let deadline = Instant::now() + Duration::from_secs(4);
            let mut reading = false;
            let mut dropped = false;
            while Instant::now() < deadline {
                let Ok(snapshot) = receiver.recv_timeout(Duration::from_millis(100)) else {
                    continue;
                };
                if snapshot.is_available(super::super::LinuxMeterId::MicIn) {
                    match snapshot.level(super::super::LinuxMeterId::MicIn) {
                        Some(_) => reading = true,
                        None if reading => {
                            dropped = true;
                            break;
                        }
                        None => {}
                    }
                }
            }
            assert!(reading, "the stalled recorder never published a reading");
            assert!(dropped, "the stale reading stayed on the bar");
            handle.stop();
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_changed_serial_restarts_the_recorder() {
            let dir = temp_dir();
            let (receiver, mut handle, _stop) = spawn(
                &dir,
                1,
                Duration::from_millis(100),
                Duration::from_millis(200),
            );
            let first = wait_for_pid(&dir, 1);
            fs::write(dir.join("nodes.json"), nodes_json(2)).expect("new serial");
            let second = wait_for_pid(&dir, 2);
            assert_ne!(first, second);
            assert!(!alive(first), "the old serial's recorder survived");
            assert!(alive(second));
            handle.stop();
            drop(receiver);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_hung_pw_dump_does_not_block_stop() {
            let dir = temp_dir();
            let dump = script(&dir, "pw-dump", "#!/bin/sh\nexec sleep 30\n");
            let record = script(&dir, "pw-record", "#!/bin/sh\nexec sleep 30\n");
            let stop = Arc::new(AtomicBool::new(false));
            let (receiver, mut handle) = spawn_linux_meters_with(
                1,
                super::super::Timings {
                    interval: Duration::from_millis(50),
                    discovery: Duration::from_millis(50),
                    timeout: Duration::from_millis(100),
                    stale: Duration::from_secs(30),
                },
                stop,
                Tools {
                    pw_dump: dump,
                    pw_record: record,
                },
            );
            std::thread::sleep(Duration::from_millis(200));
            let started = Instant::now();
            handle.stop();
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "stopping waited for the hung pw-dump"
            );
            drop(receiver);
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
