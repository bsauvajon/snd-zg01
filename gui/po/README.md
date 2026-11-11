# Translations

`fr.po` is the shipped French catalogue. The source strings are English in
`crates/zg01-gui/src`; `POTFILES` lists the files that carry a message.

GNU `xgettext` has no Rust parser. The build uses its C++ parser with
`--keyword=t:1`, `tf:1`, `tn:1,2` and `tnf:1,2`, so every `t("...")` call is
extracted. The `unterminated character constant` messages on stderr come from
Rust lifetimes (`'static`) and are harmless; the extraction still succeeds.

Workflow when a user-facing string changes:

1. Update the `t()` / `tf()` / `tn()` / `tnf()` call in the source.
2. Run `make -C gui update-po`. This runs `xgettext` over `POTFILES` into
   `po/zg01-control.pot`, merges the template into `po/fr.po` with
   `msgmerge`, and runs `msgfmt --check`.
3. Fill the new `msgstr` entries in `fr.po`, keeping named placeholders
   (`{error}`, `{name}`, `{count}`, …) exactly as written. Remove a fuzzy
   match if `msgmerge` guessed one.
4. Run `make -C gui check-po` and `make -C gui locale`
   (`msgfmt --check --statistics`).

The UI never assembles a sentence from translated fragments: a message with a
value is one `msgid` with a placeholder, so a translator can move the value
inside the French sentence.

The technical labels (`MUTE`, `CARD`, `MIC IN`, `USB`, `GATE`/`COMP`/`LIMIT`,
the ALSA control names and the device section names) are identifiers, not
prose, and stay unchanged in both locales.
