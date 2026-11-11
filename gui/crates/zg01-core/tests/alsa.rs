use zg01_core::alsa::{
    find_card_text, AlsaBackend, AlsaError, ControlInfo, ControlKind, FakeAlsa, FakeError,
};

const CARDS: &str = "\
 0 [zg01           ]: zg01_usb - ZG01
                      Yamaha ZG01 at usb-0000:00:14.0-1, high speed
 1 [Generic        ]: HDA-Intel - HD-Audio Generic
";

#[test]
fn finds_card_by_driver_and_id() {
    let card = find_card_text(CARDS, None).unwrap();
    assert_eq!(card.index, 0);
    assert_eq!(card.id, "zg01");
    assert_eq!(card.driver, "zg01_usb");
    assert_eq!(find_card_text(CARDS, Some("zg01")).unwrap().index, 0);
    assert_eq!(find_card_text(CARDS, Some("zg01_usb")).unwrap().index, 0);
    assert!(find_card_text(CARDS, Some("nope")).is_none());
}

#[test]
fn picks_the_zg01_driver_when_no_card_is_named() {
    let cards = "\
 0 [Generic        ]: HDA-Intel - HD-Audio Generic
 1 [zg01           ]: zg01_usb - ZG01
";
    let card = find_card_text(cards, None).unwrap();
    assert_eq!(card.index, 1);
    assert_eq!(card.id, "zg01");
    assert_eq!(card.driver, "zg01_usb");
}

#[test]
fn skips_the_description_lines() {
    let cards = "\
                      Yamaha ZG01 at usb-0000:00:14.0-1, high speed
";
    assert!(find_card_text(cards, None).is_none());
    assert!(find_card_text(cards, Some("zg01")).is_none());
}

#[test]
fn matches_a_card_by_id_only() {
    let cards = "\
 0 [C4K            ]: USB-Audio - Cisco Desk Camera 4K
 1 [zg01           ]: zg01_usb - ZG01
";
    let card = find_card_text(cards, Some("C4K")).unwrap();
    assert_eq!(card.index, 0);
    assert_eq!(card.driver, "USB-Audio");
}

fn gate() -> ControlInfo {
    ControlInfo {
        name: "Gate".to_string(),
        kind: ControlKind::Integer,
        min: 0,
        max: 100,
        step: 0,
        writable: true,
    }
}

fn meter() -> ControlInfo {
    ControlInfo {
        name: "Meter Mic In".to_string(),
        kind: ControlKind::Integer,
        min: 0,
        max: 127,
        step: 0,
        writable: false,
    }
}

#[test]
fn fake_backend_reads_the_values_it_was_given() {
    let backend = FakeAlsa::new(vec![gate()]);
    backend.set_value("Gate", 42);
    assert_eq!(backend.read("Gate").unwrap(), 42);
    assert!(matches!(backend.read("Nope"), Err(AlsaError::Unknown(_))));
}

#[test]
fn fake_backend_records_writes() {
    let backend = FakeAlsa::new(vec![gate()]);
    backend.write("Gate", 60).unwrap();
    assert_eq!(backend.writes(), vec![("Gate".to_string(), 60)]);
    assert_eq!(backend.read("Gate").unwrap(), 60);
}

#[test]
fn fake_backend_refuses_to_write_a_read_only_control() {
    let backend = FakeAlsa::new(vec![gate(), meter()]);
    assert!(matches!(
        backend.write("Meter Mic In", 3),
        Err(AlsaError::ReadOnly(_))
    ));
    assert!(backend.writes().is_empty());
}

#[test]
fn fake_backend_returns_the_programmed_error() {
    let backend = FakeAlsa::new(vec![gate()]);
    backend.set_error("Gate", FakeError::Unknown);
    assert!(matches!(backend.read("Gate"), Err(AlsaError::Unknown(_))));
    assert!(matches!(
        backend.write("Gate", 1),
        Err(AlsaError::Unknown(_))
    ));

    backend.clear_error("Gate");
    assert!(backend.write("Gate", 1).is_ok());
}

#[test]
fn fake_backend_lists_its_controls() {
    let backend = FakeAlsa::new(vec![gate(), meter()]);
    let listed = backend.list_controls().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].name, "Gate");
    assert_eq!(listed[1].name, "Meter Mic In");
    assert!(!listed[1].writable);
}

#[test]
fn fake_backend_reports_a_control_without_a_value() {
    let backend = FakeAlsa::new(vec![gate()]);
    assert!(matches!(backend.read("Gate"), Err(AlsaError::NoData(_))));
}

#[test]
fn the_backends_are_usable_as_trait_objects() {
    fn assert_backend<T: AlsaBackend + Send + Sync>() {}
    assert_backend::<zg01_core::alsa::RealAlsa>();
    assert_backend::<FakeAlsa>();

    let backend: Box<dyn AlsaBackend> = Box::new(FakeAlsa::new(vec![gate()]));
    assert_eq!(backend.list_controls().unwrap().len(), 1);
}

/// The hardware test needs a ZG01 with the `snd-zg01` module loaded.
///
/// Run it with `cargo test -p zg01-core --test alsa -- --ignored`.
#[test]
#[ignore]
fn lists_gate_on_hardware() {
    let card = zg01_core::alsa::find_card(None).unwrap();
    let backend = zg01_core::alsa::RealAlsa::new(card.index).unwrap();
    let names: Vec<_> = backend
        .list_controls()
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert!(names.iter().any(|n| n == "Gate"));
}

/// The meter levels are read-only, and they have no value while the device
/// does not stream.
#[test]
#[ignore]
fn refuses_to_write_a_meter_on_hardware() {
    let card = zg01_core::alsa::find_card(None).unwrap();
    let backend = zg01_core::alsa::RealAlsa::new(card.index).unwrap();
    let controls = backend.list_controls().unwrap();
    let Some(meter) = controls.iter().find(|c| c.name == "Meter Mic In") else {
        // A driver without the meter controls has nothing to check.
        return;
    };
    assert!(!meter.writable);
    assert!(matches!(
        backend.write("Meter Mic In", 1),
        Err(AlsaError::ReadOnly(_))
    ));
}
