PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin

.PHONY: build test install-local fmt

build:
	cargo build --release

test:
	cargo test

fmt:
	cargo fmt --check

install-local: build
	mkdir -p "$(BINDIR)"
	cp target/release/domainesia "$(BINDIR)/domainesia"
	chmod +x "$(BINDIR)/domainesia"
