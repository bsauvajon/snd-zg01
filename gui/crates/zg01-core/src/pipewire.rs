//! The ZG01's Linux-side meter sources, read through PipeWire.
//!
//! The card meters on EP `0x83` are the device's own levels. When they are
//! unavailable, the GUI falls back to the PipeWire graph: `Voice In` for
//! the microphone, and the monitor ports of `Voice Out` and `Game Out` for
//! the two playback paths. One `pw-record` process per source produces raw
//! `s16` stereo, reduced to a 50 ms peak and mapped onto the card meter's
//! dB scale.
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

/// The PipeWire node serials of the three ZG01 sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nodes {
    /// The `Voice In` source.
    pub mic_in: Option<u64>,
    /// The `Voice Out` sink monitor.
    pub voice_out: Option<u64>,
    /// The `Game Out` sink monitor.
    pub game_out: Option<u64>,
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

/// The three Linux meter readings at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinuxMetersSnapshot {
    /// The level of each source, in `LinuxMeterId` slot order.
    ///
    /// `None` means the source exists but has no reading yet.
    pub values: [Option<f64>; 3],
    /// Whether each source exists, independently of its reading.
    pub available: [bool; 3],
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
pub fn recorder_args(node: u64, sink: bool) -> Vec<String> {
    let mut args = vec!["--target".to_string(), node.to_string()];
    if sink {
        args.push("-P".to_string());
        args.push("stream.capture.sink=true".to_string());
    }
    args.extend([
        "--rate".to_string(),
        "48000".to_string(),
        "--channels".to_string(),
        "2".to_string(),
        "--format".to_string(),
        "s16".to_string(),
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
/// non-finite level cannot occur because the input is a finite integer.
pub fn level_from_peak(peak: u32) -> f64 {
    if peak == 0 {
        return 0.0;
    }
    let db = 20.0 * (f64::from(peak) / 32768.0).log10();
    ((db.clamp(-127.0, 0.0) + 127.0) / 127.0).clamp(0.0, 1.0)
}

/// Finds the three ZG01 nodes in a `pw-dump` JSON document.
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
        let mut peak = 0_u32;
        let mut pending = None;
        let mut window = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(len) => {
                    let mut first = 0;
                    if let Some(byte) = pending.take() {
                        let value = i16::from_ne_bytes([byte, buffer[0]]);
                        peak = peak.max(value.unsigned_abs() as u32);
                        first = 1;
                    }
                    let end = first + (len - first) / 2 * 2;
                    for sample in buffer[first..end].as_chunks::<2>().0 {
                        let value = i16::from_ne_bytes(*sample);
                        peak = peak.max(value.unsigned_abs() as u32);
                    }
                    if end < len {
                        pending = Some(buffer[len - 1]);
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
        interval,
        DISCOVERY_INTERVAL,
        DISCOVERY_TIMEOUT,
        stop,
        Tools::default(),
    )
}

/// Starts the poller with explicit tools and discovery timings.
#[cfg(feature = "pipewire")]
fn spawn_linux_meters_with(
    generation: u64,
    interval: Duration,
    discovery: Duration,
    timeout: Duration,
    stop: Arc<AtomicBool>,
    tools: Tools,
) -> (Receiver<LinuxMetersSnapshot>, LinuxMetersHandle) {
    let (snapshots, receiver) = mpsc::channel();
    let thread_stop = stop.clone();
    let join = std::thread::Builder::new()
        .name("zg01-pw-meters".to_string())
        .spawn(move || {
            coordinator_loop(
                generation,
                interval,
                discovery,
                timeout,
                thread_stop,
                tools,
                snapshots,
            )
        })
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

#[cfg(feature = "pipewire")]
fn coordinator_loop(
    generation: u64,
    interval: Duration,
    discovery: Duration,
    timeout: Duration,
    stop: Arc<AtomicBool>,
    tools: Tools,
    snapshots: Sender<LinuxMetersSnapshot>,
) {
    let (updates, receiver) = mpsc::channel::<Update>();
    let mut available = [false; 3];
    let mut values: [Option<f64>; 3] = [None; 3];
    let mut recorders: [Option<Recorder>; 3] = [None, None, None];
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
                    for (index, (id, node, sink)) in [
                        (LinuxMeterId::MicIn, nodes.mic_in, false),
                        (LinuxMeterId::VoiceOut, nodes.voice_out, true),
                        (LinuxMeterId::GameOut, nodes.game_out, true),
                    ]
                    .into_iter()
                    .enumerate()
                    {
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

    for recorder in recorders.into_iter().flatten() {
        recorder.stop();
    }
}

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
                r#"[{{"info":{{"props":{{"node.name":"alsa_input.usb-ZG01-VoiceIn__source",
                    "node.description":"Yamaha ZG01 Voice In","object.serial":"{serial}"}}}}}}]"#
            )
        }

        /// A fake `pw-dump` that prints `nodes.json`, and a fake
        /// `pw-record` that records its target and then idles.
        fn tools(dir: &Path) -> Tools {
            let dump = script(
                dir,
                "pw-dump",
                &format!("#!/bin/sh\ncat \"{}/nodes.json\"\n", dir.display()),
            );
            let record = script(
                dir,
                "pw-record",
                &format!(
                    "#!/bin/sh\ntarget=\"$2\"\necho \"$target\" >> \"{}/targets.log\"\necho $$ > \"{}/pid-$target\"\nexec sleep 30\n",
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
            fs::write(dir.join("nodes.json"), nodes_json(serial)).expect("nodes");
            let stop = Arc::new(AtomicBool::new(false));
            let (receiver, handle) = spawn_linux_meters_with(
                1,
                Duration::from_millis(50),
                discovery,
                timeout,
                stop.clone(),
                tools(dir),
            );
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
                Duration::from_millis(50),
                Duration::from_millis(50),
                Duration::from_millis(100),
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
