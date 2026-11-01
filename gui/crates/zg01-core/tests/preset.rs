//! The `.zgpreset` loader, the plan and the apply report.
//!
//! The MBDF fixtures are synthetic: they use the header, chunk and component
//! layout of `tools/zgpreset.c` and carry no Yamaha or user preset.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use zg01_core::alsa::{ControlInfo, ControlKind, FakeAlsa, FakeError};
use zg01_core::model::{Control, ControlValue, Unit, SAVE_ACTION};
use zg01_core::preset::{
    apply_plan, control_map, load_preset, plan_preset, Preset, PresetError, PresetParam, SkipReason,
};
use zg01_core::writer::ControlWriter;

/// Addresses of a few controls of the generated map.
const MIC_EQ_HIGH_SWITCH: u16 = 0x013;
const MIC_LEVEL_STATUS: u16 = 0x028;
const STREAMING_MIC_LEVEL: u16 = 0x10f;
const EQ_BAND_1_GAIN: u16 = 0x128;
const EQ_BAND_1_FREQUENCY: u16 = 0x12c;
const EQ_BAND_1_Q: u16 = 0x130;

fn info(name: &str, min: i64, max: i64, writable: bool) -> ControlInfo {
    ControlInfo {
        name: name.to_string(),
        kind: ControlKind::Integer,
        min,
        max,
        step: 0,
        writable,
    }
}

fn control(name: &str, min: i64, max: i64, writable: bool) -> Control {
    Control {
        info: info(name, min, max, writable),
        value: ControlValue::Known(0),
        unit: Unit::Steps,
    }
}

fn param(address: u16, width: u8, value: u32) -> PresetParam {
    PresetParam {
        address,
        width,
        value,
    }
}

fn preset(params: Vec<PresetParam>) -> Preset {
    Preset {
        block: "MIC INPUT".to_string(),
        name: "Test".to_string(),
        params,
    }
}

fn the_controls() -> Vec<Control> {
    vec![
        control("Mic EQ High Switch Steps", 0, 2097151, true),
        control("Mic Level Status", 0, 127, false),
        control("Monitor EQ Band 4 Switch Steps", 0, 2097151, true),
        control("Streaming Mic Level Steps", 0, 127, true),
        control("Mic EQ Band 1 Switch Steps", 0, 1, true),
        control("EQ Band 1 Gain", -180, 180, true),
        control("EQ Band 1 Frequency", 20, 1000, true),
        control("EQ Band 1 Q", 50, 1600, true),
    ]
}

#[test]
fn raw_addresses_are_skipped_without_opt_in() {
    let controls = the_controls();
    let preset = preset(vec![param(MIC_EQ_HIGH_SWITCH, 1, 1)]);

    let plan = plan_preset(&preset, &controls, false);
    assert!(plan.applies.is_empty());
    assert!(plan
        .skipped
        .iter()
        .any(|skip| skip.reason == SkipReason::Raw));

    let plan = plan_preset(&preset, &controls, true);
    assert_eq!(plan.skipped, Vec::new());
    assert_eq!(plan.applies.len(), 1);
    assert_eq!(plan.applies[0].name, "Mic EQ High Switch Steps");
}

#[test]
fn a_vendor_model_control_ending_in_steps_is_not_raw() {
    // `Streaming Mic Level Steps` is a vendor-model row, not a raw one.
    // Gating on the name would wrongly skip it.
    let controls = the_controls();
    let preset = preset(vec![param(STREAMING_MIC_LEVEL, 1, 64)]);

    let plan = plan_preset(&preset, &controls, false);
    assert_eq!(plan.skipped, Vec::new());
    assert_eq!(plan.applies.len(), 1);
    assert_eq!(plan.applies[0].value, 64);
}

#[test]
fn out_of_range_is_skipped_not_clamped() {
    let controls = the_controls();
    // 400 is +22.0 dB, past the +18.0 dB maximum.
    let over = preset(vec![param(EQ_BAND_1_GAIN, 2, 400)]);

    let plan = plan_preset(&over, &controls, false);
    assert!(plan.applies.is_empty());
    assert!(plan
        .skipped
        .iter()
        .any(|skip| skip.reason == SkipReason::OutOfRange));

    // The boundary itself applies, and is not clamped.
    let boundary = preset(vec![param(EQ_BAND_1_GAIN, 2, 360)]);
    let plan = plan_preset(&boundary, &controls, false);
    assert_eq!(plan.applies.len(), 1);
    assert_eq!(plan.applies[0].value, 180);
}

#[test]
fn read_only_addresses_are_skipped() {
    let controls = the_controls();
    let preset = preset(vec![param(MIC_LEVEL_STATUS, 1, 5)]);

    let plan = plan_preset(&preset, &controls, true);
    assert!(plan.applies.is_empty());
    assert!(plan
        .skipped
        .iter()
        .any(|skip| skip.reason == SkipReason::ReadOnly));
}

#[test]
fn unknown_addresses_and_missing_controls_are_skipped() {
    let controls = the_controls();
    let preset = preset(vec![
        // Not in the generated control map at all.
        param(0x777, 1, 9),
        // In the map, but the card does not publish the control.
        param(0x003, 1, 42),
    ]);

    let plan = plan_preset(&preset, &controls, true);
    assert!(plan.applies.is_empty(), "nothing may be fabricated");
    assert_eq!(plan.skipped.len(), 2);
    assert!(plan
        .skipped
        .iter()
        .all(|skip| skip.reason == SkipReason::Missing));
    assert!(plan.skipped.iter().any(|skip| skip.address == 0x777));
    assert!(plan.skipped.iter().any(|skip| skip.address == 0x003));
}

#[test]
fn the_plan_converts_the_raw_values_to_user_units() {
    let controls = the_controls();
    let preset = preset(vec![
        param(EQ_BAND_1_FREQUENCY, 2, 100),
        param(EQ_BAND_1_Q, 1, 12),
        param(EQ_BAND_1_GAIN, 2, 150),
    ]);

    let plan = plan_preset(&preset, &controls, false);
    assert_eq!(plan.applies.len(), 3);
    let value = |name: &str| {
        plan.applies
            .iter()
            .find(|apply| apply.name == name)
            .map(|apply| apply.value)
    };
    assert_eq!(value("EQ Band 1 Frequency"), Some(100));
    assert_eq!(value("EQ Band 1 Q"), Some(100));
    assert_eq!(value("EQ Band 1 Gain"), Some(-30));
}

#[test]
fn a_clamped_device_value_is_out_of_range() {
    let controls = the_controls();
    // The Q conversion clamps 61 to the last device step; the raw value is
    // outside the device range and must not slip through as 1600.
    let preset = preset(vec![param(EQ_BAND_1_Q, 1, 61)]);

    let plan = plan_preset(&preset, &controls, false);
    assert!(plan.applies.is_empty());
    assert!(plan
        .skipped
        .iter()
        .any(|skip| skip.reason == SkipReason::OutOfRange));
}

#[test]
fn apply_plan_writes_every_value_and_reports_the_skips() {
    let infos = vec![
        info("EQ Band 1 Gain", -180, 180, true),
        info("EQ Band 1 Frequency", 20, 1000, true),
    ];
    let backend = Arc::new(FakeAlsa::new(infos));
    let (writer, _outcomes) = ControlWriter::spawn(backend.clone());
    let controls = the_controls();
    let preset = preset(vec![
        param(EQ_BAND_1_GAIN, 2, 150),
        param(EQ_BAND_1_FREQUENCY, 2, 100),
        param(0x777, 1, 9),
    ]);

    let plan = plan_preset(&preset, &controls, false);
    let report = apply_plan(&plan, &writer);

    assert_eq!(
        report.applied,
        vec!["EQ Band 1 Gain", "EQ Band 1 Frequency"]
    );
    assert!(report.failed.is_empty());
    assert_eq!(report.skipped, plan.skipped);
    assert_eq!(
        backend.writes(),
        vec![
            ("EQ Band 1 Gain".to_string(), -30),
            ("EQ Band 1 Frequency".to_string(), 100),
        ]
    );
    assert!(
        backend.writes().iter().all(|(name, _)| name != SAVE_ACTION),
        "applying a preset must not save it"
    );
}

#[test]
fn apply_plan_reports_the_failures() {
    let infos = vec![info("Gate", 0, 100, true)];
    let backend = Arc::new(FakeAlsa::new(infos));
    backend.set_error("Gate", FakeError::Unknown);
    let (writer, _outcomes) = ControlWriter::spawn(backend.clone());

    // Build the plan against a control the card claims to publish, then
    // make the write fail.
    let controls = vec![control("Gate", 0, 100, true)];
    let preset = preset(vec![param(0x003, 1, 42)]);
    let plan = plan_preset(&preset, &controls, false);
    assert_eq!(plan.applies.len(), 1, "Gate applies in the plan");

    let report = apply_plan(&plan, &writer);
    assert!(report.applied.is_empty());
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].0, "Gate");
    assert!(!report.failed[0].1.is_empty());
}

#[test]
fn the_control_map_is_the_generated_one() {
    let map = control_map();
    assert_eq!(map.len(), 375, "the generated map has every control");
    assert_eq!(
        map.iter().filter(|entry| entry.provenance == "raw").count(),
        53,
        "only the raw provenance rows are gated"
    );
    let mut addresses: Vec<u16> = map.iter().map(|entry| entry.address).collect();
    addresses.sort_unstable();
    addresses.dedup();
    assert_eq!(addresses.len(), map.len(), "addresses are unique");
    assert!(
        map.iter()
            .any(|entry| entry.control_name == "Mic EQ High Switch Steps"
                && entry.provenance == "raw")
    );
    assert!(map
        .iter()
        .any(|entry| entry.control_name == "Streaming Mic Level Steps"
            && entry.provenance == "vendor-model"));
}

// ---------------------------------------------------------------- the FFI

/// A temporary directory removed when the fixture goes out of scope.
struct FixtureDir {
    path: PathBuf,
}

impl FixtureDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "zg01-preset-test-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("the fixture directory");
        FixtureDir { path }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(name);
        std::fs::write(&path, bytes).expect("the fixture file");
        path
    }
}

impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

const HEADER_SIZE: usize = 0x74;
const FILE_ID: &[u8] = b"79f82ba4-a076-47e8-887d-f34571f0fe49";
const ENVELOPE: &[u8] = b"Comp\x00\x00\x00\x00";

fn chunk(tag: &str, name: &str, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut tag = tag.as_bytes().to_vec();
    tag.resize(8, 0);
    bytes.extend(tag);
    bytes.extend([0; 4]);
    let mut name = name.as_bytes().to_vec();
    name.resize(12, 0);
    bytes.extend(name);
    bytes.extend((payload.len() as u64).to_be_bytes());
    bytes.extend([0; 4]);
    bytes.extend(payload);
    bytes
}

fn component(name: &str, payload: &[u8], magic: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut name = name.as_bytes().to_vec();
    name.resize(24, 0);
    bytes.extend(name);
    bytes.extend(magic.to_be_bytes());
    bytes.extend([0; 2]);
    bytes.extend((payload.len() as u16).to_le_bytes());
    bytes.extend([0; 2]);
    bytes.extend(payload);
    bytes
}

fn schema(entries: &[(u16, u8, u32)]) -> Vec<u8> {
    let mut bytes = b"\x80\x01\x00\x01\x00\x12\x41\x01\x00\x80".to_vec();
    for (address, width, _) in entries {
        bytes.extend(address.to_be_bytes());
        bytes.push(0x01);
        bytes.push(*width);
    }
    bytes
}

fn values(entries: &[(u16, u8, u32)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (_, width, value) in entries {
        bytes.extend(&value.to_le_bytes()[..usize::from(*width)]);
    }
    bytes
}

fn info_payload(block: &str, name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut block = block.as_bytes().to_vec();
    block.resize(64, 0);
    bytes.extend(block);
    bytes.extend([0; 16]);
    bytes.push(0x02);
    bytes.extend(name.as_bytes());
    bytes.extend([0; 2]);
    bytes
}

fn mbdf(magic: &str, chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![0; HEADER_SIZE];
    bytes[..magic.len()].copy_from_slice(magic.as_bytes());
    bytes.extend(FILE_ID);
    for chunk in chunks {
        bytes.extend(chunk);
    }
    bytes
}

fn preset_bytes(block: &str, name: &str, entries: &[(u16, u8, u32)]) -> Vec<u8> {
    let mut payload = ENVELOPE.to_vec();
    payload.extend(component("F_MicEq", &schema(entries), 0xda27));
    let mut data = vec![0x00, 0x02];
    let mut display = name.as_bytes().to_vec();
    display.resize(126, 0);
    data.extend(display);
    data.extend(values(entries));
    payload.extend(component("D_MicEq", &data, 0xd927));

    mbdf(
        "#YAMAHA MBDFPreset",
        &[
            chunk("#FILE", "Property", &[]),
            chunk("#PRESET ", "General", &info_payload(block, name)),
            chunk("#PRESET ", "ChunkData", &payload),
            chunk("#END", "", &[]),
        ],
    )
}

#[test]
fn load_preset_reads_a_synthetic_mbdf_file() {
    let dir = FixtureDir::new();
    let path = dir.write(
        "unit.zgpreset",
        &preset_bytes(
            "MIC INPUT",
            "Fixture",
            &[(EQ_BAND_1_GAIN, 2, 150), (EQ_BAND_1_Q, 1, 12)],
        ),
    );

    let preset = load_preset(&path).expect("the fixture parses");

    assert_eq!(preset.block, "MIC INPUT");
    assert_eq!(preset.name, "Fixture");
    assert_eq!(
        preset.params,
        vec![param(EQ_BAND_1_GAIN, 2, 150), param(EQ_BAND_1_Q, 1, 12)]
    );
}

#[test]
fn a_zg01_backup_is_rejected_with_a_clear_message() {
    let dir = FixtureDir::new();
    let path = dir.write(
        "backup.zg01",
        &mbdf(
            "#YAMAHA MBDFBackup",
            &[chunk("#CONSOLE", "ChunkData", ENVELOPE)],
        ),
    );

    let error = load_preset(&path).expect_err("a backup is not a preset");

    assert!(matches!(error, PresetError::BackupNotSupported));
    assert!(
        error.to_string().contains(".zgpreset"),
        "the message names the supported format: {error}"
    );
}

#[test]
fn a_malformed_file_is_rejected() {
    let dir = FixtureDir::new();
    let path = dir.write("broken.zgpreset", b"not an MBDF file");

    let error = load_preset(&path).expect_err("the fixture is not a preset");
    assert!(matches!(error, PresetError::Malformed));
}

#[test]
fn a_missing_file_reports_the_io_error() {
    let dir = FixtureDir::new();
    let error = load_preset(&dir.path.join("absent.zgpreset")).expect_err("no such file");
    assert!(matches!(error, PresetError::Io(_)), "got {error:?}");
}

/// The hardware check: apply a few known controls and read them back.
///
/// Run it with `cargo test -p zg01-core --test preset -- --ignored`.
/// The test writes the card, verifies the values with `amixer`, then puts
/// every control it touched back to the value it read first. It never calls
/// Save, so it does not change the persisted settings.
#[test]
#[ignore]
fn applies_a_synthetic_preset_on_hardware() {
    use std::process::Command;

    use zg01_core::alsa::{find_card, AlsaBackend, RealAlsa};

    let Ok(card) = find_card(None) else {
        eprintln!("skip: no ZG01 card");
        return;
    };
    let backend: Arc<dyn AlsaBackend> =
        Arc::new(RealAlsa::new(card.index).expect("the card opens"));
    let controls: Vec<Control> = backend
        .list_controls()
        .expect("the controls list")
        .into_iter()
        .map(|info| Control {
            info,
            value: ControlValue::Unknown,
            unit: Unit::Steps,
        })
        .collect();

    let touched = [
        "Gate",
        "Gate Threshold",
        "EQ Band 1 Gain",
        "EQ Band 1 Frequency",
        "EQ Band 1 Q",
    ];
    let previous: Vec<(String, i64)> = touched
        .iter()
        .filter_map(|name| {
            backend
                .read(name)
                .ok()
                .map(|value| ((*name).to_string(), value))
        })
        .collect();
    if previous.len() != touched.len() {
        eprintln!("skip: the card does not publish every control of the check");
        return;
    }

    let preset = preset(vec![
        param(0x003, 1, 25),  // Gate
        param(0x004, 1, 30),  // Gate Threshold
        param(0x128, 2, 200), // EQ Band 1 Gain: +2.0 dB
        param(0x12c, 2, 500), // EQ Band 1 Frequency
        param(0x130, 1, 20),  // EQ Band 1 Q
    ]);
    let plan = plan_preset(&preset, &controls, false);
    assert_eq!(plan.applies.len(), touched.len(), "the plan: {plan:?}");

    let (writer, _outcomes) = ControlWriter::spawn(Arc::clone(&backend));
    let report = apply_plan(&plan, &writer);
    assert!(report.failed.is_empty(), "the report: {report:?}");
    assert_eq!(report.applied.len(), touched.len());

    assert_eq!(backend.read("Gate").expect("Gate"), 25);
    assert_eq!(
        backend.read("EQ Band 1 Gain").expect("the gain"),
        20,
        "the user value of +2.0 dB"
    );

    let output = Command::new("amixer")
        .args(["-c", &card.index.to_string(), "cget", "name=Gate"])
        .output()
        .expect("amixer runs");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("values=25"),
        "amixer reports the applied value:\n{text}"
    );

    for (name, value) in &previous {
        writer
            .set_and_wait(name, *value)
            .unwrap_or_else(|err| panic!("could not restore {name}: {err}"));
    }
    for (name, value) in &previous {
        assert_eq!(
            backend.read(name).expect("the restored value"),
            *value,
            "{name} is back to its previous value"
        );
    }
}
