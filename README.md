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


## Apps

The user documentation, the same pages the Docs app shows, starts at
[data/shared/Docs/README.md](data/shared/Docs/README.md).

### Available

| App | What it does | Docs |
|---|---|---|
| **Files** | Browse the server's folders; upload, download and delete files | [file-browser.md](data/shared/Docs/Apps/file-browser.md) |
| **Docs** | Read, write and search documents, including the user documentation | [docs.md](data/shared/Docs/Apps/docs.md) |
| **Viewer** | Look at pictures, music, videos, PDFs and text files, and share them | [viewer.md](data/shared/Docs/Apps/viewer.md) |
| **Compiler** | Build a Rust project on the server and download the result | [rust-compiler.md](data/shared/Docs/Apps/rust-compiler.md) |
| **Spreadsheet** | Open, edit and save `.xlsx`, `.ods`, `.gnumeric` and `.csv` files, with formulas worked out by GNU Gnumeric | [spreadsheet.md](data/shared/Docs/Apps/spreadsheet.md) |
| **Server** | Live graphs of how the server is doing | [server-dashboard.md](data/shared/Docs/Apps/server-dashboard.md) |
| **Account** | See who you are signed in as, sign out, and make app passwords | [account.md](data/shared/Docs/Apps/account.md) |
| **Clock** | A clock and stopwatch | [clock.md](data/shared/Docs/Apps/clock.md) |

Your files can also be opened as a drive with [WebDAV](data/shared/Docs/WebDAV.md).

### In progress

These have an open pull request.

- **Settings**: theme, background, motion, data use, dock pinning, switching account (#4)
- **Octave**: a GNU Octave front end with command window, workspace, history, editor and plots (#56)
- **Modeller**: a SketchUp-style 3D modeller backed by FreeCAD (#47)

### Planned

These are open tickets.

- **Compiler**: C and C++ projects with GCC (#72)
- **Files**: compress and extract tar / gzip archives (#73)
- **Viewer**: server-side conversion of Apple formats such as HEIC and HEVC (#81)
- **Clock**: a sunlight map with timezones (#88)
- **Chess** against GNU Chess (#76), and **Go** against GNU Go (#77)
- **Music Score** backed by GNU LilyPond (#74)
- **Messages**: inbox, 1:1 and group chats, and file sharing (#49)
- **Admin console** (#65) and **NAS** storage (#66)
- **Fluid simulation** with OpenFOAM (#91)

## Auth

Sign-in goes through Keycloak: anyone with an account in its `ccosel` realm can get in. With
Docker installed, the server sets Keycloak up itself on first start; without Docker (and with
no Keycloak configured) login is off and everyone gets in.

**The Keycloak the server starts** runs in a Docker container (`ccosel-keycloak`) that only
this machine can reach. Browsers never talk to it directly: the server forwards its sign-in
pages under `/idp/`, so one address (or one tunnel) carries both CCOSEL and its login. The
admin console is never forwarded.

- **Admin console:** `http://localhost:8080/idp/admin/master/console/#/ccosel` (the server
  prints it at start-up), on the server machine only. The user
  is `admin`; the password is generated on first start and kept in
  `~/.local/share/ccosel/keycloak-admin-password` (`%APPDATA%\ccosel\` on Windows). It is per
  machine, like the container, so every checkout signs in to the same Keycloak with it.
- **Adding a user:** in the admin console, check the **ccosel** realm is selected (top left), then
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

Once login is on, `/rpc`, `/upload` and `/files/...` answer `401` without a signed-in session
(and `/dav/` without an app password, see [WebDAV](#webdav));
only the boot page, the shell and app modules, `/auth/*` and the forwarded `/idp/` pages stay
public.

The **Account** app shows who you are signed in as. **Manage account** opens Keycloak's
account page (profile, password, sessions) in a new tab. **Sign out** ends the session on this
server and in Keycloak, so signing in again asks for the password.

Sessions live in server memory only — a restart signs everyone out.

### WebDAV

The files are also served over WebDAV at `/dav/`, so people can mount them as a drive in
Finder, Windows Explorer, GNOME Files, davfs2 or rclone. The same `.access` permissions apply as
in the browser. [Docs/WebDAV.md](data/shared/Docs/WebDAV.md) is the user guide.

- **Signing in** is HTTP Basic: the user's login and an **app password**, which they make in
  the Account app. The session cookie is never accepted on `/dav/`. App password hashes are
  kept in `app-passwords.json` in the server's data folder (`~/.local/share/ccosel/`,
  `%APPDATA%\ccosel\`, or `/data/ccosel/` in Docker), never in the served folder. The server
  refuses to start if that data folder is inside the folder it serves.
- **It needs HTTPS in practice.** Basic auth over plain `http://` sends the app password
  readably across the LAN, and Windows refuses Basic auth over `http://` by default. Serve it
  behind an HTTPS tunnel or proxy (see above) and give people the `https://` address.
- **Failed sign-ins are rate-limited:** after ten in five minutes for one login, or from one
  address, `/dav/` answers `429` for the rest of the five minutes. Behind a tunnel running on
  this machine, the address is the one the tunnel puts last in `X-Forwarded-For`; that header
  is ignored from anywhere else.
- With login off, `/dav/` is open to everyone, anonymously, like the rest of the server.

## Running it in Docker

One container holds everything: the server, the shell and apps, and Keycloak. There is nothing
to configure; everything that must last is in one volume.

```sh
docker build -t ccosel .
docker run -d --name ccosel --restart unless-stopped \
  -p 8777:8777 -p 127.0.0.1:8080:8080 -v ccosel-data:/data ccosel
```

Then open `http://<this machine>:8777`. The first start takes a minute or so while Keycloak
sets itself up.

- **Adding accounts:** the admin console is at `http://localhost:8080/idp/admin/master/console/#/ccosel`, on this
  machine only (that is what the `127.0.0.1:` in `-p 127.0.0.1:8080:8080` does; leave that
  `-p` out to have no admin console at all). The user is `admin`, and the password is
  generated on first start:
  `docker exec ccosel cat /data/ccosel/keycloak-admin-password`. Then add users as described
  under [Auth](#auth).
- **Public, behind a tunnel,** on one domain: point the tunnel (for example
  `cloudflared tunnel --url http://localhost:8777`, or `ngrok http 8777`) at port 8777 and pass
  its address when starting the container. Sign-in goes through the same domain under `/idp/`,
  so there is nothing else to expose.

  ```sh
  docker run -d --name ccosel --restart unless-stopped \
    -p 8777:8777 -p 127.0.0.1:8080:8080 -v ccosel-data:/data \
    -e CCOSEL_PUBLIC_URL=https://ccosel.example.com ccosel
  ```

  A tunnel whose address changes on every run (a quick Cloudflare tunnel, free ngrok) needs
  the container recreated with the new address each time, since Keycloak only sends people
  back to addresses it was told about.
- **What's kept:** the `ccosel-data` volume holds people's files (`/data/files`, which starts
  with the app documentation), Keycloak's accounts, the admin password, and people's
  [app passwords](#webdav). Remove the
  container freely; remove the volume only to start over.
- **Smaller image:** `docker build --build-arg WITH_RUST=0 -t ccosel .` leaves out the Rust
  toolchain the Compiler app builds with, about 1 GB. Everything else still works.
