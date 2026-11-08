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

use std::process::Command;
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
    let output = Command::new("pw-dump").output().map_err(|source| PipewireError::Run {
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
