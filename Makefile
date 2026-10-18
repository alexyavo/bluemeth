.PHONY: build release test lint bench icon bundle install login unlogin leaks run clean

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

icon:
	scripts/make-icon.sh

bundle:
	scripts/bundle.sh

install:
	scripts/bundle.sh --install

login:
	/Applications/bluemeth.app/Contents/MacOS/bluemeth-app --login-item enable

unlogin:
	/Applications/bluemeth.app/Contents/MacOS/bluemeth-app --login-item disable

leaks:
	scripts/leakcheck.sh

run:
	cargo run -p bluemeth-app

clean:
	cargo clean
