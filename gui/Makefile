CARGO ?= cargo
XGETTEXT ?= xgettext
MSGMERGE ?= msgmerge
PREFIX ?= /usr/local
DESTDIR ?=
DOMAIN ?= zg01-control
POT = po/$(DOMAIN).pot

BINDIR = $(DESTDIR)$(PREFIX)/bin
APPSDIR = $(DESTDIR)$(PREFIX)/share/applications
METAINFODIR = $(DESTDIR)$(PREFIX)/share/metainfo
ICONDIR = $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps
LOCALEDIR = $(DESTDIR)$(PREFIX)/share/locale

.PHONY: build test check-po pot update-po install uninstall locale clean

build:
	$(CARGO) build --workspace --release

locale:
	mkdir -p target/locale/fr/LC_MESSAGES
	msgfmt --check --statistics -o target/locale/fr/LC_MESSAGES/$(DOMAIN).mo po/fr.po

check-po:
	msgfmt --check -o /dev/null po/fr.po

# GNU xgettext has no Rust parser. It extracts the `t("...")` / `tf("...")`
# / `tn*("...", "...")` calls with the C++ parser; the "unterminated
# character constant" warnings come from Rust lifetimes ('static) and are
# harmless. POTFILES lists every source file that carries a message.
pot:
	$(XGETTEXT) --language=C++ --from-code=UTF-8 --foreign-user \
		--copyright-holder="Bernard Sauvajon" \
		--package-name="ZG01 Control" \
		--msgid-bugs-address="https://github.com/bsauvajon/snd-zg01/issues" \
		--add-comments=TRANSLATORS: \
		--keyword=t:1 --keyword=tf:1 --keyword=tn:1,2 --keyword=tnf:1,2 \
		-o $(POT) -f po/POTFILES

# Merge the extracted template into the French catalogue. New msgids land as
# untranslated entries for the translator to fill; existing translations and
# their placeholders are kept.
update-po: pot
	$(MSGMERGE) --update --backup=none po/fr.po $(POT)
	$(MAKE) check-po

test: check-po
	$(CARGO) test --workspace
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) fmt --all -- --check

install: build locale
	install -Dm755 target/release/zg01-control $(BINDIR)/zg01-control
	install -Dm644 data/io.github.bsauvajon.Zg01Control.desktop \
		$(APPSDIR)/io.github.bsauvajon.Zg01Control.desktop
	install -Dm644 data/io.github.bsauvajon.Zg01Control.metainfo.xml \
		$(METAINFODIR)/io.github.bsauvajon.Zg01Control.metainfo.xml
	install -Dm644 data/icons/io.github.bsauvajon.Zg01Control.svg \
		$(ICONDIR)/io.github.bsauvajon.Zg01Control.svg
	install -Dm644 target/locale/fr/LC_MESSAGES/$(DOMAIN).mo \
		$(LOCALEDIR)/fr/LC_MESSAGES/$(DOMAIN).mo

uninstall:
	rm -f $(BINDIR)/zg01-control
	rm -f $(APPSDIR)/io.github.bsauvajon.Zg01Control.desktop
	rm -f $(METAINFODIR)/io.github.bsauvajon.Zg01Control.metainfo.xml
	rm -f $(ICONDIR)/io.github.bsauvajon.Zg01Control.svg
	rm -f $(LOCALEDIR)/fr/LC_MESSAGES/$(DOMAIN).mo

clean:
	$(CARGO) clean
	rm -rf target/locale
