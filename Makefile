.PHONY: dev release new-app

dev:
	cargo xtask dev

release:
	cargo xtask build-web
	cargo build --release -p ccosel-server

new-app:
	cargo xtask new-app $(NAME)
