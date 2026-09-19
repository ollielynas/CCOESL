.PHONY: dev release new-app

dev:
	cargo xtask dev

release:
	cargo xtask build-web
	cargo build --release -p ccosel-server

# make new-app NAME=my-app
new-app:
	cargo xtask new-app $(NAME)
