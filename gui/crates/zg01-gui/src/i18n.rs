//! gettext plumbing for the user interface.
//!
//! The source strings are English and live inline in the UI code. The French
//! catalogue is compiled from `po/fr.po` and installed under
//! `$(PREFIX)/share/locale`. `ZG01_LOCALEDIR` overrides the lookup directory,
//! which is how a source-tree run finds the catalogue built by
//! `make -C gui locale`.
//!
//! Messages that carry a value use named placeholders (`{error}`, `{name}`)
//! and are translated as one string, never assembled from translated
//! fragments: a translator can move a placeholder inside the sentence.

use std::ffi::{c_char, CString};

use gtk4::glib;

/// The gettext domain, matching the installed `zg01-control.mo` file.
pub(crate) const DOMAIN: &str = "zg01-control";

extern "C" {
    fn bindtextdomain(domainname: *const c_char, dirname: *const c_char) -> *mut c_char;
    fn textdomain(domainname: *const c_char) -> *mut c_char;
}

/// Binds the catalogue directory and selects the domain.
///
/// Call this once, after GTK has initialised the locale (GTK calls
/// `setlocale(LC_ALL, "")` during `gtk_init`).
pub(crate) fn init() {
    let Some(directory) = locale_dir() else {
        return;
    };
    let (Ok(domain), Ok(directory)) = (CString::new(DOMAIN), CString::new(directory)) else {
        return;
    };
    // SAFETY: both pointers are NUL-terminated and outlive the calls.
    unsafe {
        bindtextdomain(domain.as_ptr(), directory.as_ptr());
        textdomain(domain.as_ptr());
    }
}

/// The directory that holds `<lang>/LC_MESSAGES/<domain>.mo`.
fn locale_dir() -> Option<String> {
    if let Ok(directory) = std::env::var("ZG01_LOCALEDIR") {
        return Some(directory);
    }
    // An installed binary is `<prefix>/bin/zg01-control`; its catalogue is
    // `<prefix>/share/locale`, which keeps a custom PREFIX working.
    if let Ok(executable) = std::env::current_exe() {
        if let Some(prefix) = executable.parent().and_then(std::path::Path::parent) {
            let candidate = prefix.join("share/locale");
            if candidate.is_dir() {
                return Some(candidate.display().to_string());
            }
        }
    }
    // A source-tree run after `make -C gui locale`.
    for candidate in ["target/locale", "locale"] {
        let path = std::path::Path::new(candidate);
        if path.is_dir() {
            return Some(path.display().to_string());
        }
    }
    Some("/usr/share/locale".to_string())
}

/// Translates one message.
pub(crate) fn t(msgid: &str) -> String {
    glib::dgettext(Some(DOMAIN), msgid).to_string()
}

/// Translates one message and substitutes its named placeholders.
///
/// A placeholder is written `{name}` in the source string and kept verbatim
/// in the translation.
pub(crate) fn tf(msgid: &str, args: &[(&str, &str)]) -> String {
    let mut text = t(msgid);
    for (name, value) in args {
        text = text.replace(name, value);
    }
    text
}

/// Translates a plural message.
pub(crate) fn tn(singular: &str, plural: &str, count: usize) -> String {
    glib::dngettext(Some(DOMAIN), singular, plural, count as _).to_string()
}

/// Translates a plural message and substitutes its named placeholders.
pub(crate) fn tnf(singular: &str, plural: &str, count: usize, args: &[(&str, &str)]) -> String {
    let mut text = tn(singular, plural, count);
    for (name, value) in args {
        text = text.replace(name, value);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::tf;

    #[test]
    fn placeholders_are_substituted_after_translation() {
        assert_eq!(
            tf("Save failed: {error}", &[("{error}", "input/output error")]),
            "Save failed: input/output error"
        );
    }
}
