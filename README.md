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

Sign-in goes through Keycloak: anyone with an account in its `ccosel` realm can get in. With
Docker installed, the server sets Keycloak up itself on first start; without Docker (and with
no Keycloak configured) login is off and everyone gets in.

**The Keycloak the server starts** runs in a Docker container (`ccosel-keycloak`) that only
this machine can reach. Browsers never talk to it directly: the server forwards its sign-in
pages under `/idp/`, so one address (or one tunnel) carries both CCOSEL and its login. The
admin console is never forwarded.

- **Admin console:** `http://localhost:8080/idp/admin/`, on the server machine only. The user
  is `admin`; the password is generated on first start and kept in
  `data/keycloak-admin-password`.
- **Adding a user:** in the admin console, switch to the **ccosel** realm (top left), then
  **Users → Add user**. Fill in email, first and last name too, or Keycloak asks for them at
  first sign-in. Set a password under **Credentials**.
- Accounts are kept in the `ccosel-keycloak-data` Docker volume, so they survive restarts and
  the container being replaced.

**Behind an HTTPS tunnel** (Cloudflare Tunnel or similar), point the tunnel at this server's
port (8777) and tell the server its public address:

```sh
CCOSEL_PUBLIC_URL=https://ccosel.example.com cargo xtask serve
```

It is then used for the sign-in callback, registered with Keycloak, and makes the session
cookie `Secure`. Put it in `.env` to keep it.

**Your own Keycloak** instead: set `CCOSEL_KEYCLOAK_URL` (the realm URL, e.g.
`https://sso.example.com/realms/ccosel`) and `CCOSEL_KEYCLOAK_CLIENT_ID`, plus
`CCOSEL_KEYCLOAK_CLIENT_SECRET` for a confidential client. Browsers then go to that Keycloak
directly, and its client needs `<this server>/auth/callback` as a valid redirect URI.

Once login is on, `/rpc`, `/upload` and `/files/...` answer `401` without a signed-in session;
only the boot page, the shell and app modules, `/auth/*` and the forwarded `/idp/` pages stay
public.

The **Account** app shows who you are signed in as. **Manage account** opens Keycloak's
account page (profile, password, sessions) in a new tab. **Sign out** ends the session on this
server and in Keycloak, so signing in again asks for the password.

Sessions live in server memory only — a restart signs everyone out.
