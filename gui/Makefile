CARGO ?= cargo
PREFIX ?= /usr/local
DESTDIR ?=
DOMAIN ?= zg01-control

BINDIR = $(DESTDIR)$(PREFIX)/bin
APPSDIR = $(DESTDIR)$(PREFIX)/share/applications
METAINFODIR = $(DESTDIR)$(PREFIX)/share/metainfo
ICONDIR = $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps
LOCALEDIR = $(DESTDIR)$(PREFIX)/share/locale

.PHONY: build test check-po install uninstall locale clean

build:
	$(CARGO) build --workspace --release

locale:
	mkdir -p target/locale/fr/LC_MESSAGES
	msgfmt --check --statistics -o target/locale/fr/LC_MESSAGES/$(DOMAIN).mo po/fr.po

check-po:
	msgfmt --check -o /dev/null po/fr.po

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
