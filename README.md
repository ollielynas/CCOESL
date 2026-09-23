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

The server can gate itself behind Keycloak OAuth plus an approved-account list. It's off by
default — a checkout with nothing configured runs exactly as before.

1. Start Keycloak locally:

   ```sh
   docker run -d -p 8080:8080 -e KEYCLOAK_ADMIN=admin -e KEYCLOAK_ADMIN_PASSWORD=admin \
     quay.io/keycloak/keycloak start-dev
   ```

2. Open the Keycloak admin console at `http://localhost:8080` (admin/admin). Create a realm
   (e.g. `ccosel`), then create a client:
   - Client type: `OpenID Connect`
   - Valid redirect URIs: `http://localhost:8777/auth/callback`
   - Web origins: `http://localhost:8777`
   - Client authentication: On (this generates the client secret)
   Note the **Client ID** and **Client Secret** from the Credentials tab.

3. Copy `data/approved_users.example.txt` to `data/approved_users.txt` and list the Keycloak
   usernames allowed to sign in, one per line.

4. Start the server with the Keycloak credentials:

   ```sh
   CCOSEL_KEYCLOAK_URL=http://localhost:8080/realms/ccosel \
   CCOSEL_KEYCLOAK_CLIENT_ID=ccosel \
   CCOSEL_KEYCLOAK_CLIENT_SECRET=... \
   cargo xtask serve
   ```

`--approved-users <path>` overrides the list's location. Sessions live in server memory only —
a restart signs everyone out.
