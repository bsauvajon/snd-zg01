CARGO ?= cargo

.PHONY: build test clean

build:
	$(CARGO) build --workspace --release

test:
	$(CARGO) test --workspace
	$(CARGO) clippy --workspace -- -D warnings
	$(CARGO) fmt --all -- --check

clean:
	$(CARGO) clean
