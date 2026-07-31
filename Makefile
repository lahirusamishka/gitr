.PHONY: build run install uninstall

build:
	cargo build --release

run:
	cargo run --release

install:
	sudo cp target/release/gitr /usr/local/bin/gitr

uninstall:
	sudo rm -f /usr/local/bin/gitr
