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
