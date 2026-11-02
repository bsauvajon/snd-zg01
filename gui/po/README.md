# Translations

`fr.po` is the shipped French catalogue. The source strings are English in
`crates/zg01-gui/src`; `POTFILES` lists the files that carry a message.

`xgettext` does not understand Rust, so the catalogue is maintained directly
instead of regenerated blindly. When a user-facing string changes:

1. Update the `t()` / `tf()` / `tn()` / `tnf()` call in the source.
2. Update the matching `msgid` in `fr.po`, keeping its named placeholders
   (`{error}`, `{name}`, `{count}`, …) exactly as written.
3. Run `make -C gui check-po` (`msgfmt --check`) and `make -C gui locale`
   (`msgfmt --check --statistics`).

The UI never assembles a sentence from translated fragments: a message with a
value is one `msgid` with a placeholder, so a translator can move the value
inside the French sentence.

The technical labels (`MUTE`, `CARD`, `MIC IN`, `USB`, `GATE`/`COMP`/`LIMIT`,
the ALSA control names and the device section names) are identifiers, not
prose, and stay unchanged in both locales.
