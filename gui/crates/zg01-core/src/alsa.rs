//! ALSA control backend and card discovery for the ZG01.
//!
//! The GUI drives the ZG01 through its ALSA *control* interface: the kernel
//! module registers one mixer element per device parameter (`Gate`, `Gate
//! Capture Switch`, the EQ bands, ...) plus the read-only meter levels, and
//! the card is the one the `zg01_usb` driver claims.
//!
//! The `alsa` crate wraps `snd_ctl_open()`, the element list and the element
//! values, but it does not wrap `snd_ctl_elem_info()`, which is where ALSA
//! publishes the element type, the value range and the read-only flag. The
//! [`ControlInfo`] the GUI needs cannot be built without it, so the backend
//! calls the `alsa-sys` bindings the `alsa` crate itself is built on and
//! keeps the crate's error type for the failures.

use std::collections::{hash_map::Entry, HashMap};
use std::ffi::{CStr, CString};
use std::os::raw::c_long;
use std::ptr;
use std::sync::Mutex;

use thiserror::Error;

/// Where the kernel lists the sound cards.
const CARDS_PATH: &str = "/proc/asound/cards";

/// Driver fragment used to pick the ZG01 when no card is named.
const ZG01_DRIVER: &str = "zg01";

/// `ENODATA`: the control has no value to report.
///
/// The meter controls return it while the device does not stream, so the GUI
/// can tell "no reading" from "the control failed".
const ENODATA: i32 = 61;

/// `EIO`: the device went away while an operation was in flight.
const EIO: i32 = 5;

/// `ENODEV`: the device is gone.
const ENODEV: i32 = 19;

/// A sound card of the ZG01 driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardInfo {
    /// The card index, as ALSA numbers them (`hw:<index>`).
    pub index: u32,
    /// The card identifier, e.g. `zg01`.
    pub id: String,
    /// The driver name, e.g. `zg01_usb`.
    pub driver: String,
    /// The USB bus and device number, e.g. `1:7`.
    ///
    /// A card that is unplugged and plugged back in gets the same ALSA index
    /// but a new USB device number. The watcher compares this field so a
    /// quick replug is not mistaken for an unchanged card.
    pub generation: Option<String>,
}

/// The kind of value an ALSA element carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    /// A switch: 0 or 1.
    Boolean,
    /// A number, possibly an enumeration index.
    Integer,
    /// A byte buffer, which the GUI cannot show as a number.
    Bytes,
}

/// What ALSA publishes about one control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlInfo {
    /// The element name, e.g. `Gate`.
    pub name: String,
    /// The kind of value the element carries.
    pub kind: ControlKind,
    /// The lowest value the element accepts.
    pub min: i64,
    /// The highest value the element accepts.
    pub max: i64,
    /// The step between two values, 0 when the element does not publish one.
    pub step: i64,
    /// How many values the element carries.
    pub count: u32,
    /// Whether the element accepts writes.
    pub writable: bool,
}

/// Everything that can go wrong while talking to the device.
#[derive(Debug, Error)]
pub enum AlsaError {
    /// ALSA refused the operation.
    #[error("ALSA error: {0}")]
    Alsa(#[from] alsa::Error),
    /// The card list could not be read.
    #[error("could not read {CARDS_PATH}: {0}")]
    Io(#[from] std::io::Error),
    /// No card matched.
    #[error("no ZG01 card found in {CARDS_PATH}")]
    CardNotFound,
    /// The control does not exist.
    #[error("no such control: {0}")]
    Unknown(String),
    /// The control exists but does not accept writes.
    #[error("control is read-only: {0}")]
    ReadOnly(String),
    /// The control kind cannot be handled as a number.
    #[error("unsupported control kind: {0}")]
    Unsupported(String),
    /// The control has no value to report.
    ///
    /// The meter controls return `-ENODATA` while the device does not stream.
    #[error("the control has no value: {0}")]
    NoData(String),
}

impl AlsaError {
    /// Whether the error means the card went away.
    ///
    /// `ENODEV` and `EIO` are what a read or a write reports after the USB
    /// device was unplugged (or replaced by a card with the same index).
    /// The GUI must then drop the backend and re-read the card instead of
    /// treating the value as a transient control failure.
    pub fn is_liveness(&self) -> bool {
        let errno = match self {
            AlsaError::Alsa(error) => error.errno(),
            AlsaError::Io(error) => error.raw_os_error().unwrap_or(0),
            _ => return false,
        };
        errno == ENODEV || errno == EIO
    }
}

/// The control surface the GUI talks to.
pub trait AlsaBackend: Send + Sync {
    /// Every control the device publishes.
    fn list_controls(&self) -> Result<Vec<ControlInfo>, AlsaError>;

    /// How many values a control declares, when the backend knows it.
    ///
    /// The meter poller uses this to tell the legacy four-value driver from
    /// the v2 arrays without reading a frame first. The default is unknown.
    fn control_count(&self, _name: &str) -> Option<u32> {
        None
    }

    /// The current value of a control.
    fn read(&self, name: &str) -> Result<i64, AlsaError>;

    /// Every value of a control.
    ///
    /// The default is the single value of [`AlsaBackend::read`], so a
    /// backend that cannot carry arrays keeps working.
    fn read_values(&self, name: &str) -> Result<Vec<i64>, AlsaError> {
        self.read(name).map(|value| vec![value])
    }

    /// Sets the value of a control.
    fn write(&self, name: &str, value: i64) -> Result<(), AlsaError>;
}

/// Finds the ZG01 card in the text of `/proc/asound/cards`.
///
/// The parsing follows `zgctl`: every card is a line of the form
/// `%d [id] : driver`, the identifier is trimmed, and the card matches when
/// its identifier or its driver equals `want`. Without a `want`, the first
/// card whose driver contains `zg01` is returned.
pub fn find_card_text(text: &str, want: Option<&str>) -> Option<CardInfo> {
    for line in text.lines() {
        let Some(card) = parse_card_line(line) else {
            continue;
        };
        match want {
            Some(want) => {
                if card.id == want || card.driver == want {
                    return Some(card);
                }
            }
            None => {
                if card.driver.contains(ZG01_DRIVER) {
                    return Some(card);
                }
            }
        }
    }
    None
}

/// Finds the ZG01 card on the running system.
pub fn find_card(want: Option<&str>) -> Result<CardInfo, AlsaError> {
    let text = std::fs::read_to_string(CARDS_PATH)?;
    let mut card = find_card_text(&text, want).ok_or(AlsaError::CardNotFound)?;
    card.generation = card_generation(card.index);
    Ok(card)
}

/// The USB bus and device numbers of a card, when sysfs exposes them.
///
/// `/proc/asound/cards` only carries the ALSA index, which a quick replug
/// keeps. The USB `devnum` changes, so it tells the watcher that the card
/// behind the same index is a new device.
fn card_generation(index: u32) -> Option<String> {
    let device = std::fs::canonicalize(format!("/sys/class/sound/card{index}/device")).ok()?;
    let usb = device
        .ancestors()
        .find(|path| path.join("devnum").exists() && path.join("busnum").exists())?;
    let bus = read_trimmed(usb.join("busnum"))?;
    let dev = read_trimmed(usb.join("devnum"))?;
    Some(format!("{bus}:{dev}"))
}

/// Reads a one-line sysfs attribute.
fn read_trimmed(path: std::path::PathBuf) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Parses one line of `/proc/asound/cards`.
fn parse_card_line(line: &str) -> Option<CardInfo> {
    let line = line.trim_start();
    let index_end = line.find(|c: char| !c.is_ascii_digit())?;
    if index_end == 0 {
        return None;
    }
    let index = line[..index_end].parse().ok()?;

    let rest = line[index_end..].trim_start();
    let rest = rest.strip_prefix('[')?;
    let close = rest.find(']')?;
    // `zgctl` only trims the trailing blanks of the identifier field.
    let id = rest[..close].trim_end().to_string();

    let rest = rest[close + 1..].trim_start();
    let rest = rest.strip_prefix(':')?;
    let driver = rest.split_whitespace().next()?.to_string();

    Some(CardInfo {
        index,
        id,
        driver,
        generation: None,
    })
}

/// The backend that talks to the real device.
pub struct RealAlsa {
    index: u32,
    ctl: Mutex<RawCtl>,
    /// The elements the backend already looked up.
    ///
    /// Finding an element means listing every element of the card and
    /// reading the metadata of each one. The meter poller reads four
    /// controls twenty times a second, so the lookup is done once and
    /// kept here.
    cache: Mutex<ElemCache>,
}

impl RealAlsa {
    /// Opens the control device of a card (`hw:<index>`).
    pub fn new(index: u32) -> Result<Self, AlsaError> {
        let ctl = RawCtl::open(&format!("hw:{index}"))?;
        Ok(RealAlsa {
            index,
            ctl: Mutex::new(ctl),
            cache: Mutex::new(ElemCache::default()),
        })
    }

    /// The card index this backend is bound to.
    pub fn index(&self) -> u32 {
        self.index
    }
}

impl AlsaBackend for RealAlsa {
    fn list_controls(&self) -> Result<Vec<ControlInfo>, AlsaError> {
        let ctl = self.lock();
        let mut cache = self.lock_cache();
        let mut controls = Vec::new();
        for id in ctl.elements()? {
            let Some(name) = element_name(&id) else {
                continue;
            };
            let meta = match ctl.meta(&id) {
                Ok(meta) => meta,
                Err(err) => {
                    tracing::warn!("ignoring control {name}: {err}");
                    continue;
                }
            };
            let Some(kind) = meta.kind else {
                tracing::warn!("ignoring control {name}: unsupported element type");
                continue;
            };
            controls.push(ControlInfo {
                name: name.clone(),
                kind,
                min: meta.min,
                max: meta.max,
                step: meta.step,
                count: meta.count as u32,
                writable: meta.writable,
            });
            cache.insert(name, id, meta);
        }
        Ok(controls)
    }

    fn read(&self, name: &str) -> Result<i64, AlsaError> {
        let ctl = self.lock();
        let mut cache = self.lock_cache();
        let (id, meta) = find_cached(&mut cache, name, || ctl.find(name))?;
        ctl.read(id, meta)
    }

    fn read_values(&self, name: &str) -> Result<Vec<i64>, AlsaError> {
        let ctl = self.lock();
        let mut cache = self.lock_cache();
        let (id, meta) = find_cached(&mut cache, name, || ctl.find(name))?;
        ctl.read_values(id, meta)
    }

    fn control_count(&self, name: &str) -> Option<u32> {
        let ctl = self.lock();
        let mut cache = self.lock_cache();
        let (_, meta) = find_cached(&mut cache, name, || ctl.find(name)).ok()?;
        Some(meta.count as u32)
    }

    fn write(&self, name: &str, value: i64) -> Result<(), AlsaError> {
        let ctl = self.lock();
        let mut cache = self.lock_cache();
        let (id, meta) = find_cached(&mut cache, name, || ctl.find(name))?;
        if !meta.writable {
            return Err(AlsaError::ReadOnly(name.to_string()));
        }
        ctl.write(id, meta, value)
    }
}

impl RealAlsa {
    /// Locks the control handle, ignoring a poisoned lock: the handle stays
    /// usable after a panic in another thread.
    fn lock(&self) -> std::sync::MutexGuard<'_, RawCtl> {
        self.ctl.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Locks the element cache, ignoring a poisoned lock.
    fn lock_cache(&self) -> std::sync::MutexGuard<'_, ElemCache> {
        self.cache.lock().unwrap_or_else(|err| err.into_inner())
    }
}

/// The elements the backend already looked up.
#[derive(Default)]
struct ElemCache {
    entries: HashMap<String, CachedElem>,
}

impl ElemCache {
    /// Keeps the element and its metadata.
    fn insert(&mut self, name: String, id: RawElemId, meta: ElemMeta) {
        self.entries.insert(name, CachedElem { id, meta });
    }
}

/// An element and the metadata the driver publishes for it.
struct CachedElem {
    /// The identifier of the element.
    id: RawElemId,
    /// What the driver publishes about it.
    meta: ElemMeta,
}

/// Finds the element named `name`, looking it up once.
///
/// The first call scans the elements of the card with `scan` and keeps
/// what it finds; the next calls are answered from the cache. A name the
/// scan does not know is not cached: the control may appear later.
fn find_cached<'a>(
    cache: &'a mut ElemCache,
    name: &str,
    scan: impl FnOnce() -> Result<(RawElemId, ElemMeta), AlsaError>,
) -> Result<(&'a RawElemId, &'a ElemMeta), AlsaError> {
    match cache.entries.entry(name.to_string()) {
        Entry::Occupied(entry) => {
            let entry = entry.into_mut();
            Ok((&entry.id, &entry.meta))
        }
        Entry::Vacant(entry) => {
            let (id, meta) = scan()?;
            let entry = entry.insert(CachedElem { id, meta });
            Ok((&entry.id, &entry.meta))
        }
    }
}

/// What ALSA publishes about one element.
#[derive(Clone)]
struct ElemMeta {
    /// `None` when the element type has no GUI equivalent.
    kind: Option<ControlKind>,
    min: i64,
    max: i64,
    step: i64,
    count: usize,
    writable: bool,
}

/// A control device handle.
struct RawCtl {
    handle: *mut alsa_sys::snd_ctl_t,
}

impl RawCtl {
    /// Opens a control device, e.g. `hw:2`.
    fn open(name: &str) -> Result<Self, AlsaError> {
        let name = CString::new(name).map_err(|_| {
            AlsaError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "control device name contains a NUL byte",
            ))
        })?;
        let mut handle: *mut alsa_sys::snd_ctl_t = ptr::null_mut();
        // SAFETY: `name` is a valid NUL-terminated string and `handle` is a
        // valid out pointer.
        let err = unsafe { alsa_sys::snd_ctl_open(&mut handle, name.as_ptr(), 0) };
        if err < 0 {
            return Err(alsa::Error::new("snd_ctl_open", -err).into());
        }
        Ok(RawCtl { handle })
    }

    /// The identifiers of every element of the card.
    fn elements(&self) -> Result<Vec<RawElemId>, AlsaError> {
        // SAFETY: every call below takes the live handle and freshly
        // allocated objects, and the wrappers free them on drop.
        unsafe {
            let mut list: *mut alsa_sys::snd_ctl_elem_list_t = ptr::null_mut();
            let err = alsa_sys::snd_ctl_elem_list_malloc(&mut list);
            if err < 0 {
                return Err(alsa::Error::new("snd_ctl_elem_list_malloc", -err).into());
            }
            let list = RawElemList(list);

            // The first pass fills the list descriptor with the number of
            // elements, the second one copies the elements into the space
            // reserved for them.
            let err = alsa_sys::snd_ctl_elem_list(self.handle, list.0);
            if err < 0 {
                return Err(alsa::Error::new("snd_ctl_elem_list", -err).into());
            }
            let count = alsa_sys::snd_ctl_elem_list_get_count(list.0);
            if count > 0 {
                let err = alsa_sys::snd_ctl_elem_list_alloc_space(list.0, count);
                if err < 0 {
                    return Err(alsa::Error::new("snd_ctl_elem_list_alloc_space", -err).into());
                }
                let err = alsa_sys::snd_ctl_elem_list(self.handle, list.0);
                if err < 0 {
                    return Err(alsa::Error::new("snd_ctl_elem_list", -err).into());
                }
            }
            let used = alsa_sys::snd_ctl_elem_list_get_used(list.0);

            let mut ids = Vec::with_capacity(used as usize);
            for index in 0..used {
                let mut id: *mut alsa_sys::snd_ctl_elem_id_t = ptr::null_mut();
                let err = alsa_sys::snd_ctl_elem_id_malloc(&mut id);
                if err < 0 {
                    return Err(alsa::Error::new("snd_ctl_elem_id_malloc", -err).into());
                }
                alsa_sys::snd_ctl_elem_list_get_id(list.0, index, id);
                ids.push(RawElemId(id));
            }
            Ok(ids)
        }
    }

    /// The metadata of one element.
    fn meta(&self, id: &RawElemId) -> Result<ElemMeta, AlsaError> {
        // SAFETY: the info object is freshly allocated and released on drop.
        unsafe {
            let mut info: *mut alsa_sys::snd_ctl_elem_info_t = ptr::null_mut();
            let err = alsa_sys::snd_ctl_elem_info_malloc(&mut info);
            if err < 0 {
                return Err(alsa::Error::new("snd_ctl_elem_info_malloc", -err).into());
            }
            let info = RawElemInfo(info);

            alsa_sys::snd_ctl_elem_info_set_id(info.0, id.0);
            let err = alsa_sys::snd_ctl_elem_info(self.handle, info.0);
            if err < 0 {
                return Err(alsa::Error::new("snd_ctl_elem_info", -err).into());
            }

            let elem_type = alsa_sys::snd_ctl_elem_info_get_type(info.0);
            let (min, max, step) = range_of(info.0, elem_type);
            Ok(ElemMeta {
                kind: kind_of(elem_type),
                min,
                max,
                step,
                count: (alsa_sys::snd_ctl_elem_info_get_count(info.0) as usize).max(1),
                writable: alsa_sys::snd_ctl_elem_info_is_writable(info.0) != 0,
            })
        }
    }

    /// The identifier and the metadata of the element named `name`.
    fn find(&self, name: &str) -> Result<(RawElemId, ElemMeta), AlsaError> {
        for id in self.elements()? {
            if element_name(&id).as_deref() == Some(name) {
                let meta = self.meta(&id)?;
                return Ok((id, meta));
            }
        }
        Err(AlsaError::Unknown(name.to_string()))
    }

    /// Reads every value of an element.
    fn read_values(&self, id: &RawElemId, meta: &ElemMeta) -> Result<Vec<i64>, AlsaError> {
        let kind = meta
            .kind
            .ok_or_else(|| AlsaError::Unsupported(element_name(id).unwrap_or_default()))?;
        // SAFETY: the value object is freshly allocated and released on drop;
        // the identifier outlives the read.
        unsafe {
            let mut value: *mut alsa_sys::snd_ctl_elem_value_t = ptr::null_mut();
            let err = alsa_sys::snd_ctl_elem_value_malloc(&mut value);
            if err < 0 {
                return Err(alsa::Error::new("snd_ctl_elem_value_malloc", -err).into());
            }
            let value = RawElemValue(value);

            alsa_sys::snd_ctl_elem_value_set_id(value.0, id.0);
            match kind {
                ControlKind::Boolean => {
                    alsa_sys::snd_ctl_elem_value_set_boolean(value.0, 0, 0);
                }
                ControlKind::Integer => {
                    alsa_sys::snd_ctl_elem_value_set_integer(value.0, 0, 0);
                }
                ControlKind::Bytes => {
                    return Err(AlsaError::Unsupported(element_name(id).unwrap_or_default()));
                }
            }

            let err = alsa_sys::snd_ctl_elem_read(self.handle, value.0);
            if err < 0 {
                let errno = -err;
                if errno == ENODATA {
                    return Err(AlsaError::NoData(element_name(id).unwrap_or_default()));
                }
                return Err(alsa::Error::new("snd_ctl_elem_read", errno).into());
            }

            let mut values = Vec::with_capacity(meta.count.max(1));
            for index in 0..meta.count.max(1) as u32 {
                values.push(match kind {
                    ControlKind::Boolean => {
                        alsa_sys::snd_ctl_elem_value_get_boolean(value.0, index) as i64
                    }
                    ControlKind::Integer => {
                        alsa_sys::snd_ctl_elem_value_get_integer(value.0, index) as i64
                    }
                    ControlKind::Bytes => {
                        return Err(AlsaError::Unsupported(element_name(id).unwrap_or_default()));
                    }
                });
            }
            Ok(values)
        }
    }

    /// Reads the first value of an element.
    fn read(&self, id: &RawElemId, meta: &ElemMeta) -> Result<i64, AlsaError> {
        self.read_values(id, meta)?
            .into_iter()
            .next()
            .ok_or_else(|| AlsaError::NoData(element_name(id).unwrap_or_default()))
    }

    /// Writes the value of an element.
    fn write(&self, id: &RawElemId, meta: &ElemMeta, value: i64) -> Result<(), AlsaError> {
        let kind = meta
            .kind
            .ok_or_else(|| AlsaError::Unsupported(element_name(id).unwrap_or_default()))?;
        // SAFETY: the value object is freshly allocated and released on drop;
        // the identifier outlives the write.
        unsafe {
            let mut raw: *mut alsa_sys::snd_ctl_elem_value_t = ptr::null_mut();
            let err = alsa_sys::snd_ctl_elem_value_malloc(&mut raw);
            if err < 0 {
                return Err(alsa::Error::new("snd_ctl_elem_value_malloc", -err).into());
            }
            let raw = RawElemValue(raw);

            alsa_sys::snd_ctl_elem_value_set_id(raw.0, id.0);
            match kind {
                ControlKind::Boolean => {
                    let on = if value != 0 { 1 } else { 0 };
                    alsa_sys::snd_ctl_elem_value_set_boolean(raw.0, 0, on);
                }
                ControlKind::Integer => {
                    alsa_sys::snd_ctl_elem_value_set_integer(raw.0, 0, value as c_long);
                }
                ControlKind::Bytes => {
                    return Err(AlsaError::Unsupported(element_name(id).unwrap_or_default()));
                }
            }

            let err = alsa_sys::snd_ctl_elem_write(self.handle, raw.0);
            if err < 0 {
                return Err(alsa::Error::new("snd_ctl_elem_write", -err).into());
            }
            Ok(())
        }
    }
}

impl Drop for RawCtl {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `RawCtl::open` and is not used
        // after the drop.
        unsafe { alsa_sys::snd_ctl_close(self.handle) };
    }
}

// SAFETY: the handle is only ever used behind the mutex of `RealAlsa`, so it
// is never touched by two threads at the same time.
unsafe impl Send for RawCtl {}

/// An element identifier owned by the caller.
struct RawElemId(*mut alsa_sys::snd_ctl_elem_id_t);

// SAFETY: the identifier is a plain descriptor allocated by ALSA, and it
// only travels between threads inside the mutex of `RealAlsa`.
unsafe impl Send for RawElemId {}

impl Drop for RawElemId {
    fn drop(&mut self) {
        // SAFETY: the identifier was allocated by `snd_ctl_elem_id_malloc`.
        unsafe { alsa_sys::snd_ctl_elem_id_free(self.0) };
    }
}

/// An element list owned by the caller.
struct RawElemList(*mut alsa_sys::snd_ctl_elem_list_t);

impl Drop for RawElemList {
    fn drop(&mut self) {
        // SAFETY: the list was allocated by `snd_ctl_elem_list_malloc`.
        unsafe {
            alsa_sys::snd_ctl_elem_list_free_space(self.0);
            alsa_sys::snd_ctl_elem_list_free(self.0);
        }
    }
}

/// The metadata of an element, owned by the caller.
struct RawElemInfo(*mut alsa_sys::snd_ctl_elem_info_t);

impl Drop for RawElemInfo {
    fn drop(&mut self) {
        // SAFETY: the info object was allocated by `snd_ctl_elem_info_malloc`.
        unsafe { alsa_sys::snd_ctl_elem_info_free(self.0) };
    }
}

/// An element value, owned by the caller.
struct RawElemValue(*mut alsa_sys::snd_ctl_elem_value_t);

impl Drop for RawElemValue {
    fn drop(&mut self) {
        // SAFETY: the value was allocated by `snd_ctl_elem_value_malloc`.
        unsafe { alsa_sys::snd_ctl_elem_value_free(self.0) };
    }
}

/// The name of an element.
fn element_name(id: &RawElemId) -> Option<String> {
    // SAFETY: the identifier is alive and ALSA returns a NUL-terminated
    // string that stays valid as long as the element exists.
    unsafe {
        let name = alsa_sys::snd_ctl_elem_id_get_name(id.0);
        if name.is_null() {
            return None;
        }
        CStr::from_ptr(name).to_str().ok().map(str::to_string)
    }
}

/// The value range of an element.
///
/// `snd_ctl_elem_info_get_min()` and its siblings abort the process when the
/// element is not an integer, so the range of every other kind is derived
/// from the element type instead.
///
/// # Safety
///
/// The info object must be alive and describe an element of `elem_type`.
unsafe fn range_of(info: *mut alsa_sys::snd_ctl_elem_info_t, elem_type: u32) -> (i64, i64, i64) {
    match elem_type {
        alsa_sys::SND_CTL_ELEM_TYPE_INTEGER => (
            alsa_sys::snd_ctl_elem_info_get_min(info) as i64,
            alsa_sys::snd_ctl_elem_info_get_max(info) as i64,
            alsa_sys::snd_ctl_elem_info_get_step(info) as i64,
        ),
        alsa_sys::SND_CTL_ELEM_TYPE_INTEGER64 => (
            alsa_sys::snd_ctl_elem_info_get_min64(info) as i64,
            alsa_sys::snd_ctl_elem_info_get_max64(info) as i64,
            alsa_sys::snd_ctl_elem_info_get_step64(info) as i64,
        ),
        alsa_sys::SND_CTL_ELEM_TYPE_BOOLEAN => (0, 1, 1),
        alsa_sys::SND_CTL_ELEM_TYPE_ENUMERATED => {
            let items = alsa_sys::snd_ctl_elem_info_get_items(info);
            (0, items.saturating_sub(1) as i64, 1)
        }
        alsa_sys::SND_CTL_ELEM_TYPE_BYTES | alsa_sys::SND_CTL_ELEM_TYPE_IEC958 => {
            let count = alsa_sys::snd_ctl_elem_info_get_count(info);
            (0, count as i64, 1)
        }
        _ => (0, 0, 0),
    }
}

/// Maps an ALSA element type to the kind the GUI handles.
fn kind_of(elem_type: u32) -> Option<ControlKind> {
    match elem_type {
        alsa_sys::SND_CTL_ELEM_TYPE_BOOLEAN => Some(ControlKind::Boolean),
        alsa_sys::SND_CTL_ELEM_TYPE_INTEGER | alsa_sys::SND_CTL_ELEM_TYPE_INTEGER64 => {
            Some(ControlKind::Integer)
        }
        // An enumerated element carries the index of the selected item.
        alsa_sys::SND_CTL_ELEM_TYPE_ENUMERATED => Some(ControlKind::Integer),
        alsa_sys::SND_CTL_ELEM_TYPE_BYTES | alsa_sys::SND_CTL_ELEM_TYPE_IEC958 => {
            Some(ControlKind::Bytes)
        }
        _ => None,
    }
}

/// A failure the [`FakeAlsa`] backend reports for one control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeError {
    /// The control does not exist.
    Unknown,
    /// The control exists but has no value.
    NoData,
    /// The control exists but does not accept writes.
    ReadOnly,
    /// The control kind cannot be handled as a number.
    Unsupported,
    /// The card went away (`ENODEV`).
    DeviceLost,
}

/// A test double for [`AlsaBackend`].
///
/// It keeps the controls it was built with, hands out the values it was
/// given, records every write and can be told to fail for a control.
pub struct FakeAlsa {
    state: Mutex<FakeState>,
}

struct FakeState {
    controls: Vec<ControlInfo>,
    values: HashMap<String, Vec<i64>>,
    writes: Vec<(String, i64)>,
    errors: HashMap<String, FakeError>,
}

impl FakeAlsa {
    /// A backend that publishes `controls`.
    pub fn new(controls: Vec<ControlInfo>) -> Self {
        FakeAlsa {
            state: Mutex::new(FakeState {
                controls,
                values: HashMap::new(),
                writes: Vec::new(),
                errors: HashMap::new(),
            }),
        }
    }

    /// Sets the value a control hands out.
    pub fn set_value(&self, name: &str, value: i64) {
        self.lock().values.insert(name.to_string(), vec![value]);
    }

    /// Sets every value a multichannel control hands out.
    pub fn set_values(&self, name: &str, values: Vec<i64>) {
        self.lock().values.insert(name.to_string(), values);
    }

    /// The first value of a control.
    pub fn value(&self, name: &str) -> Option<i64> {
        self.lock()
            .values
            .get(name)
            .and_then(|values| values.first().copied())
    }

    /// Makes every access to a control fail.
    pub fn set_error(&self, name: &str, error: FakeError) {
        self.lock().errors.insert(name.to_string(), error);
    }

    /// Lets a control work again.
    pub fn clear_error(&self, name: &str) {
        self.lock().errors.remove(name);
    }

    /// The writes the backend received, in order.
    pub fn writes(&self) -> Vec<(String, i64)> {
        self.lock().writes.clone()
    }

    /// Forgets the recorded writes.
    pub fn clear_writes(&self) {
        self.lock().writes.clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }
}

impl AlsaBackend for FakeAlsa {
    fn list_controls(&self) -> Result<Vec<ControlInfo>, AlsaError> {
        Ok(self.lock().controls.clone())
    }

    fn control_count(&self, name: &str) -> Option<u32> {
        self.lock()
            .controls
            .iter()
            .find(|control| control.name == name)
            .map(|control| control.count)
    }

    fn read(&self, name: &str) -> Result<i64, AlsaError> {
        self.read_values(name)?
            .into_iter()
            .next()
            .ok_or_else(|| AlsaError::NoData(name.to_string()))
    }

    fn read_values(&self, name: &str) -> Result<Vec<i64>, AlsaError> {
        let state = self.lock();
        if let Some(error) = state.errors.get(name) {
            return Err(fake_error(error, name));
        }
        if !state.controls.iter().any(|control| control.name == name) {
            return Err(AlsaError::Unknown(name.to_string()));
        }
        state
            .values
            .get(name)
            .cloned()
            .ok_or_else(|| AlsaError::NoData(name.to_string()))
    }

    fn write(&self, name: &str, value: i64) -> Result<(), AlsaError> {
        let mut state = self.lock();
        if let Some(error) = state.errors.get(name) {
            return Err(fake_error(error, name));
        }
        let control = state
            .controls
            .iter()
            .find(|control| control.name == name)
            .ok_or_else(|| AlsaError::Unknown(name.to_string()))?;
        if !control.writable {
            return Err(AlsaError::ReadOnly(name.to_string()));
        }
        state.writes.push((name.to_string(), value));
        state.values.insert(name.to_string(), vec![value]);
        Ok(())
    }
}

/// Builds the error of a programmed failure.
fn fake_error(error: &FakeError, name: &str) -> AlsaError {
    match error {
        FakeError::Unknown => AlsaError::Unknown(name.to_string()),
        FakeError::NoData => AlsaError::NoData(name.to_string()),
        FakeError::ReadOnly => AlsaError::ReadOnly(name.to_string()),
        FakeError::Unsupported => AlsaError::Unsupported(name.to_string()),
        FakeError::DeviceLost => AlsaError::Alsa(alsa::Error::new("fake", ENODEV)),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::{
        element_name, find_cached, AlsaBackend, AlsaError, ControlInfo, ControlKind, ElemCache,
        ElemMeta, FakeAlsa, RawElemId,
    };

    #[test]
    fn a_missing_device_is_a_liveness_error() {
        let gone = AlsaError::Alsa(alsa::Error::new("snd_ctl_elem_read", 19));
        let io = AlsaError::Io(std::io::Error::from_raw_os_error(5));
        let unknown = AlsaError::Unknown("Gate".to_string());
        assert!(gone.is_liveness(), "ENODEV means the card went away");
        assert!(io.is_liveness(), "EIO means the card went away");
        assert!(
            !unknown.is_liveness(),
            "a missing control is not a disconnect"
        );
    }

    /// An element identifier that names `name`, without a card.
    fn element(name: &str) -> RawElemId {
        let mut id = std::ptr::null_mut();
        // SAFETY: the identifier is freshly allocated and freed on drop.
        unsafe {
            assert_eq!(alsa_sys::snd_ctl_elem_id_malloc(&mut id), 0);
        }
        let id = RawElemId(id);
        let name = CString::new(name).unwrap();
        // SAFETY: the identifier is alive and the name is NUL-terminated.
        unsafe {
            alsa_sys::snd_ctl_elem_id_set_name(id.0, name.as_ptr());
        }
        id
    }

    /// The metadata of a writable element.
    fn meta(writable: bool) -> ElemMeta {
        ElemMeta {
            kind: Some(ControlKind::Integer),
            min: 0,
            max: 100,
            step: 1,
            count: 1,
            writable,
        }
    }

    #[test]
    fn an_element_is_looked_up_once() {
        let mut cache = ElemCache::default();
        let mut scans = 0;
        for _ in 0..3 {
            let (id, meta) = find_cached(&mut cache, "Gate", || {
                scans += 1;
                Ok((element("Gate"), meta(true)))
            })
            .unwrap();
            assert_eq!(element_name(id).as_deref(), Some("Gate"));
            assert!(meta.writable);
        }
        assert_eq!(scans, 1, "the element is only looked up once");
    }

    #[test]
    fn a_missing_element_is_not_cached() {
        let mut cache = ElemCache::default();
        let mut scans = 0;
        for _ in 0..2 {
            let result = find_cached(&mut cache, "Nope", || {
                scans += 1;
                Err(AlsaError::Unknown("Nope".to_string()))
            });
            assert!(matches!(result, Err(AlsaError::Unknown(_))));
        }
        assert_eq!(scans, 2, "a control that does not exist is looked up again");
    }

    #[test]
    fn the_cached_metadata_is_the_one_of_the_element() {
        let mut cache = ElemCache::default();
        let (_, found) =
            find_cached(&mut cache, "Gate", || Ok((element("Gate"), meta(false)))).unwrap();
        assert!(!found.writable);
        let (_, found) =
            find_cached(&mut cache, "Gate", || Ok((element("Gate"), meta(true)))).unwrap();
        assert!(!found.writable, "the cache wins over the scan");
    }

    #[test]
    fn a_multichannel_control_reads_all_its_values() {
        let info = ControlInfo {
            name: "Meter PC In".to_string(),
            kind: ControlKind::Integer,
            min: 0,
            max: 127,
            step: 0,
            count: 8,
            writable: false,
        };
        let fake = FakeAlsa::new(vec![info]);
        fake.set_values("Meter PC In", vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(
            fake.read_values("Meter PC In").unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(fake.read("Meter PC In").unwrap(), 1);
    }
}
