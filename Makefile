.PHONY: build run install

build:
	cargo build --release

run:
	cargo run --release

install:
	sudo cp target/release/rgitk-gui /usr/local/bin/rgitk-gui
