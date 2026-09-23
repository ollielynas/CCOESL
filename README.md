# CCOESL

[![CI](https://github.com/ollielynas/CCOESL/actions/workflows/ci.yml/badge.svg)](https://github.com/ollielynas/CCOESL/actions/workflows/ci.yml)

Want to help? Read [CONTRIBUTING.md](CONTRIBUTING.md): setup, the checks every change must pass, and
how tickets and pull requests work.

The goal of this project is to provide a lightweight browser based desktop emviroment hosted from a single main computer. Lightweight apps are distrobuted as wasm files. A user can laod the webage hosted on thier local network. App packets are dynamically sent to them as needed allowing them to do lightweight work in the browser/dekstop enviroment. Heavy work is done by sending infomation back to the centerlised main computer which then does the compute and returns the result.

Rendering is done exclusivly with egui. each app renders its contents by calling egui redering calls.

Minimising web traffic is a priority

The web client handles
  - user accounts (minimal auth for now)
  - balancing memory usage by loading and offloading chunks of wasm
  - rendering

  One major goal is to have the UI be polished enough to look modern and be used by the layperson

The server application
  - runs in a docker container
  - hosts the web inetrface
  - runs heavy comutations
  - handles auth (eventually)
  - hosts shared and user space file systems


## apps

- [ ] **File Browser**
Users should be able to uplaod and download files from either a private or public file system location

- [ ] **Compiler**
The user can uplaod a project directory or select a project from the server file system and the server will compile it for their arcetecture.
Plan to support
- C / C++
- Rust

## Auth

The server can gate itself behind GitHub OAuth plus an approved-account list. It's off by
default — a checkout with nothing configured runs exactly as before.

1. Register a GitHub OAuth app (https://github.com/settings/developers). Its callback URL is
   `http://localhost:8777/auth/callback` (or whatever host:port the server is actually reached
   at — see `crates/ccosel-server/src/auth.rs` for why `http://`, not `https://`, is correct for
   now).
2. Copy `data/approved_users.example.txt` to `data/approved_users.txt` and list the GitHub
   usernames allowed to sign in, one per line.
3. Start the server with the app's credentials:

   ```sh
   CCOSEL_GITHUB_CLIENT_ID=... CCOSEL_GITHUB_CLIENT_SECRET=... cargo xtask serve
   ```

`--approved-users <path>` overrides the list's location. Sessions live in server memory only —
a restart signs everyone out.
