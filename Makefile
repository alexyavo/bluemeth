.PHONY: build release test lint bench bundle install leaks run clean

build:
	cargo build --workspace

release:
	cargo build --release --workspace

test:
	cargo test --workspace

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

bench: release
	target/release/bluemeth-bench all

bundle:
	scripts/bundle.sh

install:
	scripts/bundle.sh --install

leaks:
	scripts/leakcheck.sh

run:
	cargo run -p bluemeth-app

clean:
	cargo clean
