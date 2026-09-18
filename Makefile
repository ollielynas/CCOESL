.PHONY: dev release

dev:
	cargo xtask dev

release:
	cargo xtask build-web
	cargo build --release -p ccosel-server
