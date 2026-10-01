//! WebDAV at `/dav/`, so the server's files can be mounted as a drive (Finder, Windows
//! Explorer, davfs2, GNOME Files, rclone).
//!
//! `dav-server` speaks the protocol; this module only decides who the caller is and what they
//! may touch. Its own `LocalFs` is deliberately not used: it reads the disk directly and would
//! skip `.access`. [`JailFs`] instead sends every operation through [`Jail`], the same checks
//! `/rpc`, `/files` and `/upload` make, so a WebDAV client can never do more than the shell.
//!
//! **Signing in** is HTTP Basic with the caller's login and one of their app passwords (see
//! [`crate::app_passwords`]), checked by [`require_app_password`]. The session cookie is never
//! accepted here: `SameSite=Lax` is all that keeps another site from using a signed-in
//! browser's cookie, and it does not cover methods like `PROPFIND` or `DELETE` the way it
//! covers `/rpc`'s `POST`. With login turned off, every caller is anonymous, as everywhere else.
//!
//! **Operations the other routes lack** have these rules (see [`Jail::check_removable`]):
//!
//! - `DELETE` needs write on the entry, on the folder it is in, and on every folder inside it.
//! - `MOVE` needs what `DELETE` needs on the source, and write where it lands.
//! - `COPY` needs read on all of the source and write where it lands.
//! - Replacing something at the destination needs what removing it needs.
//!
//! `dav-server` carries out `DELETE` and `COPY` of a folder one entry at a time, and replaces a
//! destination before moving onto it. So each of those is checked in full by [`preflight`]
//! before it starts, and refused whole, rather than stopping halfway with part of it done.

use std::io::SeekFrom;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use bytes::{Buf, Bytes, BytesMut};
use ccosel_proto::server_error;
use dav_server::davpath::DavPath;
use dav_server::fakels::FakeLs;
use dav_server::fs::{
    DavDirEntry, DavFile, DavMetaData, FsError, FsFuture, FsResult, FsStream, GuardedFileSystem,
    OpenOptions, ReadDirMeta,
};
use dav_server::{DavConfig, DavHandler};
use futures_util::FutureExt;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::AppState;
use crate::access::User;
use crate::fs_api::{Jail, Need};
use crate::upload_api::MAX_UPLOAD_BYTES;

/// Where WebDAV is mounted.
pub const PREFIX: &str = "/dav";

/// The realm a client is asked to sign in to.
const REALM: &str = "CCOSEL";

/// Who is asking, as the file system sees it: a user name, or `None` for anonymous.
type Caller = Option<String>;

/// The handler for `/dav`, built once.
pub fn handler(jail: Arc<Jail>) -> DavHandler<Caller> {
    DavConfig::new()
        .strip_prefix(PREFIX)
        .filesystem(Box::new(JailFs { jail }))
        // macOS and Windows will only mount a share read-write if it says it supports locks.
        // A lock that locks nothing is what `dav-server` offers for exactly that.
        .locksystem(FakeLs::new())
        // Show a link the caller may follow as what it leads to, as the Files app does.
        // `JailFs::read_dir` has already left out any they may not.
        .hide_symlinks(false)
        .build_handler()
}

/// Asks a client to sign in.
fn challenge() -> Response {
    let mut resp = StatusCode::UNAUTHORIZED.into_response();
    let value = format!("Basic realm=\"{REALM}\", charset=\"UTF-8\"");
    if let Ok(value) = HeaderValue::from_str(&value) {
        resp.headers_mut().insert(header::WWW_AUTHENTICATE, value);
    }
    resp
}

/// The `login:password` of a `Basic` `Authorization` header.
fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, encoded) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (login, password) = text.split_once(':')?;
    Some((login.to_owned(), password.to_owned()))
}

/// Guards `/dav`. With login on, a request needs `Basic` credentials naming a user and one of
/// their app passwords, or gets `401`; too many wrong ones, from one address or for one
/// login, get `429` for a while. With login off, everyone is let in, anonymous. A [`User`]
/// already present (see `crate::as_user`) is left alone, as `auth::require_session` does.
pub async fn require_app_password(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    if !state.auth.enabled() || req.extensions().get::<User>().is_some() {
        return next.run(req).await;
    }
    let Some((login, password)) = basic_credentials(req.headers()) else {
        return challenge();
    };
    let mut keys = vec![format!("login:{login}")];
    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        keys.push(format!("ip:{}", client_ip(addr.ip(), req.headers())));
    }
    let now = Instant::now();
    if let Some(wait) = state.auth.throttle.blocked(&keys, now) {
        let mut resp = (
            StatusCode::TOO_MANY_REQUESTS,
            "too many failed sign-ins, try again later",
        )
            .into_response();
        resp.headers_mut()
            .insert(header::RETRY_AFTER, (wait.as_secs() + 1).into());
        return resp;
    }
    if User::valid_name(&login) && state.auth.app_passwords.verify(&login, &password) {
        req.extensions_mut().insert(User(login));
        return next.run(req).await;
    }
    state.auth.throttle.fail(&keys, now);
    challenge()
}

/// Who to count a failed sign-in against. Usually the connection's own address, but behind a
/// tunnel (cloudflared, ngrok) every connection comes from this machine, and counting them all
/// as one would let anyone lock everyone out. So for a connection from this machine only, the
/// address the tunnel says it forwarded for is used: the last in `X-Forwarded-For`, which the
/// tunnel added itself. Anyone else could write anything there, so it is ignored for them.
fn client_ip(peer: IpAddr, headers: &HeaderMap) -> IpAddr {
    if !peer.is_loopback() {
        return peer;
    }
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .next_back()
        .and_then(|ip| ip.trim().parse().ok())
        .unwrap_or(peer)
}

/// `ANY /dav/...`.
pub async fn handle(State(state): State<AppState>, req: Request) -> Response {
    let user = crate::caller(req.extensions().get::<User>()).map(str::to_owned);
    if let Some(name) = &user
        && let Err(e) = crate::access::ensure_home(state.jail.root(), name)
    {
        eprintln!("dav: creating the home folder for {name}: {e}");
    }

    if req.method().as_str() == "PROPFIND" && is_infinite_depth(req.headers()) {
        // A listing of everything at once, which a client never needs to browse and which
        // could walk the whole jail. RFC 4918 9.1: refuse it, and say why.
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <D:error xmlns:D=\"DAV:\"><D:propfind-finite-depth/></D:error>\n",
        )
            .into_response();
    }
    if req.method() == Method::PUT && declared_length(req.headers()) > Some(MAX_UPLOAD_BYTES as u64)
    {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    if let Err(status) = preflight(&state.jail, &req, user.as_deref()) {
        return status.into_response();
    }

    let principal = user.clone().unwrap_or_else(|| "anonymous".to_owned());
    state
        .dav
        .handle_guarded(req, principal, user)
        .await
        .map(axum::body::Body::new)
}

fn is_infinite_depth(headers: &HeaderMap) -> bool {
    headers
        .get("depth")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|d| d.trim().eq_ignore_ascii_case("infinity"))
}

fn declared_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

/// A request path under [`PREFIX`] as a jail path (`/dav/a/b%20c` → `a/b c`). `None` for one
/// that is not under it, including `/davish`, which `DavPath::set_prefix` alone would accept.
fn jail_path(uri_path: &str) -> Option<String> {
    let mut path = DavPath::new(uri_path).ok()?;
    let full = path.as_bytes();
    let under = full
        .strip_prefix(PREFIX.as_bytes())
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(b"/"));
    if !under {
        return None;
    }
    path.set_prefix(PREFIX).ok()?;
    path.as_rel_ospath().to_str().map(str::to_owned)
}

/// The status a refusal from [`Jail`] becomes.
fn status_of(code: u32) -> StatusCode {
    match code {
        server_error::NOT_FOUND => StatusCode::NOT_FOUND,
        server_error::DENIED => StatusCode::FORBIDDEN,
        server_error::EXISTS => StatusCode::PRECONDITION_FAILED,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Checks the whole of a `DELETE`, `MOVE` or `COPY` before any of it happens. See the module
/// documentation for why `dav-server`'s own per-entry checks are not enough on their own.
fn preflight(jail: &Jail, req: &Request, user: Option<&str>) -> Result<(), StatusCode> {
    let method = req.method().as_str();
    if !matches!(method, "DELETE" | "MOVE" | "COPY") {
        return Ok(());
    }
    let source = jail_path(req.uri().path()).ok_or(StatusCode::BAD_REQUEST)?;
    if method == "DELETE" {
        return jail
            .check_removable(&source, user)
            .map(drop)
            .map_err(status_of);
    }

    if method == "MOVE" {
        jail.check_removable(&source, user).map_err(status_of)?;
    } else {
        jail.check_readable_tree(&source, user).map_err(status_of)?;
    }
    let dest = req
        .headers()
        .get("destination")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<Uri>().ok())
        .and_then(|uri| jail_path(uri.path()))
        // A destination outside `/dav` is somewhere this server cannot put anything.
        .ok_or(StatusCode::BAD_GATEWAY)?;
    let overwrite = req
        .headers()
        .get("overwrite")
        .is_none_or(|v| !v.as_bytes().eq_ignore_ascii_case(b"F"));
    match jail.authorize_new(&dest, user) {
        // Something is there. `dav-server` refuses without `Overwrite`; with it, it removes
        // what is there first, which must be allowed too.
        Ok(target) if target.symlink_metadata().is_ok() => {
            if overwrite {
                jail.check_removable(&dest, user).map_err(status_of)?;
            }
            Ok(())
        }
        Ok(_) => Ok(()),
        // RFC 4918 9.8.5/9.9.4: no folder to put it in is a conflict.
        Err(server_error::NOT_FOUND) => Err(StatusCode::CONFLICT),
        Err(code) => Err(status_of(code)),
    }
}

/// The jail, as `dav-server` sees it. Every method goes through [`Jail`] with the caller.
#[derive(Clone)]
struct JailFs {
    jail: Arc<Jail>,
}

fn fs_error(code: u32) -> FsError {
    match code {
        server_error::NOT_FOUND => FsError::NotFound,
        server_error::DENIED | server_error::NOT_A_DIRECTORY => FsError::Forbidden,
        server_error::EXISTS => FsError::Exists,
        server_error::TOO_LARGE => FsError::TooLarge,
        _ => FsError::GeneralFailure,
    }
}

fn rel(path: &DavPath) -> FsResult<String> {
    path.as_rel_ospath()
        .to_str()
        .map(str::to_owned)
        .ok_or(FsError::NotFound)
}

fn io_error(e: std::io::Error) -> FsError {
    match e.kind() {
        std::io::ErrorKind::NotFound => FsError::NotFound,
        std::io::ErrorKind::PermissionDenied => FsError::Forbidden,
        std::io::ErrorKind::AlreadyExists => FsError::Exists,
        _ => FsError::GeneralFailure,
    }
}

impl JailFs {
    fn open_now(&self, path: &DavPath, options: OpenOptions, user: Option<&str>) -> FsResult<File> {
        let path = rel(path)?;
        let writing = options.write || options.append || options.create || options.create_new;
        if !writing {
            let real = self
                .jail
                .authorize(&path, user, Need::Read)
                .map_err(fs_error)?;
            if real.is_dir() {
                return Err(FsError::Forbidden);
            }
            let file = std::fs::File::open(&real).map_err(io_error)?;
            return Ok(File::new(file, 0));
        }

        if options.size.is_some_and(|n| n > MAX_UPLOAD_BYTES as u64) {
            return Err(FsError::TooLarge);
        }
        let target = self.jail.write_target(&path, user).map_err(fs_error)?;
        let existing = target.metadata().ok().map(|m| m.len());
        if options.create_new && existing.is_some() {
            return Err(FsError::Exists);
        }
        let file = std::fs::OpenOptions::new()
            .read(options.read)
            .write(true)
            .append(options.append)
            .truncate(options.truncate)
            .create(true)
            .open(&target)
            .map_err(io_error)?;
        // What is already there counts towards the cap only if it is being added to.
        let kept = if options.truncate {
            0
        } else {
            existing.unwrap_or(0)
        };
        Ok(File::new(file, kept))
    }

    fn read_dir_now(
        &self,
        path: &DavPath,
        meta: ReadDirMeta,
        user: Option<&str>,
    ) -> FsResult<Vec<Box<dyn DavDirEntry>>> {
        let path = rel(path)?;
        let at_root = path.is_empty();
        let visible = self
            .jail
            .visible_entries(&path, user, usize::MAX)
            .map_err(fs_error)?;
        let mut out: Vec<Box<dyn DavDirEntry>> = Vec::new();
        for entry in visible.entries {
            // Temporary project folders are the Compiler's business, as in the Files app.
            if at_root && entry.name == ccosel_proto::scratch::DIR {
                continue;
            }
            let meta = if meta == ReadDirMeta::Data && entry.meta.is_symlink() {
                // Already checked to lead somewhere visible inside the jail.
                match std::fs::metadata(visible.dir.join(&entry.name)) {
                    Ok(m) => m,
                    Err(_) => continue,
                }
            } else {
                entry.meta
            };
            out.push(Box::new(DirEntry {
                name: entry.name,
                meta: Meta(meta),
            }));
        }
        Ok(out)
    }
}

impl GuardedFileSystem<Caller> for JailFs {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
        user: &'a Caller,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        let result = self
            .open_now(path, options, user.as_deref())
            .map(|f| Box::new(f) as Box<dyn DavFile>);
        async move { result }.boxed()
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a DavPath,
        meta: ReadDirMeta,
        user: &'a Caller,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        let result = self
            .read_dir_now(path, meta, user.as_deref())
            .map(|entries| {
                Box::pin(futures_util::stream::iter(entries.into_iter().map(Ok)))
                    as FsStream<Box<dyn DavDirEntry>>
            });
        async move { result }.boxed()
    }

    fn metadata<'a>(
        &'a self,
        path: &'a DavPath,
        user: &'a Caller,
    ) -> FsFuture<'a, Box<dyn DavMetaData>> {
        let result = rel(path).and_then(|p| {
            let real = self
                .jail
                .authorize(&p, user.as_deref(), Need::Read)
                .map_err(fs_error)?;
            let meta = std::fs::metadata(real).map_err(io_error)?;
            Ok(Box::new(Meta(meta)) as Box<dyn DavMetaData>)
        });
        async move { result }.boxed()
    }

    fn symlink_metadata<'a>(
        &'a self,
        path: &'a DavPath,
        user: &'a Caller,
    ) -> FsFuture<'a, Box<dyn DavMetaData>> {
        let result = rel(path).and_then(|p| {
            let entry = self.jail.locate(&p, user.as_deref()).map_err(fs_error)?;
            let meta = std::fs::symlink_metadata(entry).map_err(io_error)?;
            Ok(Box::new(Meta(meta)) as Box<dyn DavMetaData>)
        });
        async move { result }.boxed()
    }

    fn create_dir<'a>(&'a self, path: &'a DavPath, user: &'a Caller) -> FsFuture<'a, ()> {
        let result =
            rel(path).and_then(|p| self.jail.create_dir(&p, user.as_deref()).map_err(fs_error));
        async move { result }.boxed()
    }

    fn remove_dir<'a>(&'a self, path: &'a DavPath, user: &'a Caller) -> FsFuture<'a, ()> {
        self.remove_file(path, user)
    }

    fn remove_file<'a>(&'a self, path: &'a DavPath, user: &'a Caller) -> FsFuture<'a, ()> {
        let result =
            rel(path).and_then(|p| self.jail.remove(&p, user.as_deref()).map_err(fs_error));
        async move { result }.boxed()
    }

    fn rename<'a>(
        &'a self,
        from: &'a DavPath,
        to: &'a DavPath,
        user: &'a Caller,
    ) -> FsFuture<'a, ()> {
        let result = rel(from).and_then(|from| {
            let to = rel(to)?;
            self.jail
                .rename(&from, &to, user.as_deref())
                .map_err(fs_error)
        });
        async move { result }.boxed()
    }

    fn copy<'a>(
        &'a self,
        from: &'a DavPath,
        to: &'a DavPath,
        user: &'a Caller,
    ) -> FsFuture<'a, ()> {
        let result = rel(from).and_then(|from| {
            let to = rel(to)?;
            self.jail
                .copy_file(&from, &to, user.as_deref())
                .map_err(fs_error)
        });
        async move { result }.boxed()
    }
}

#[derive(Clone, Debug)]
struct Meta(std::fs::Metadata);

impl DavMetaData for Meta {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn modified(&self) -> FsResult<std::time::SystemTime> {
        self.0.modified().map_err(io_error)
    }

    fn is_dir(&self) -> bool {
        self.0.is_dir()
    }

    fn is_symlink(&self) -> bool {
        self.0.file_type().is_symlink()
    }

    fn created(&self) -> FsResult<std::time::SystemTime> {
        self.0.created().map_err(io_error)
    }
}

struct DirEntry {
    name: String,
    meta: Meta,
}

impl DavDirEntry for DirEntry {
    fn name(&self) -> Vec<u8> {
        self.name.as_bytes().to_vec()
    }

    fn metadata(&self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        let meta = Box::new(self.meta.clone()) as Box<dyn DavMetaData>;
        async move { Ok(meta) }.boxed()
    }
}

/// An open file. Writes stop at [`MAX_UPLOAD_BYTES`] however the body arrives, so a client
/// that sends no `Content-Length` (chunked) is held to the same cap as `/upload`.
#[derive(Debug)]
struct File {
    file: tokio::fs::File,
    /// How big the file would be after the writes so far, as far as the cap is concerned.
    size: u64,
}

impl File {
    fn new(file: std::fs::File, size: u64) -> Self {
        Self {
            file: tokio::fs::File::from_std(file),
            size,
        }
    }

    async fn write(&mut self, bytes: &[u8]) -> FsResult<()> {
        self.size += bytes.len() as u64;
        if self.size > MAX_UPLOAD_BYTES as u64 {
            return Err(FsError::TooLarge);
        }
        self.file.write_all(bytes).await.map_err(io_error)
    }
}

impl DavFile for File {
    fn metadata(&mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        async move {
            let meta = self.file.metadata().await.map_err(io_error)?;
            Ok(Box::new(Meta(meta)) as Box<dyn DavMetaData>)
        }
        .boxed()
    }

    fn write_buf(&mut self, mut buf: Box<dyn Buf + Send>) -> FsFuture<'_, ()> {
        async move {
            while buf.has_remaining() {
                let chunk = Bytes::copy_from_slice(buf.chunk());
                buf.advance(chunk.len());
                self.write(&chunk).await?;
            }
            Ok(())
        }
        .boxed()
    }

    fn write_bytes(&mut self, buf: Bytes) -> FsFuture<'_, ()> {
        async move { self.write(&buf).await }.boxed()
    }

    fn read_bytes(&mut self, count: usize) -> FsFuture<'_, Bytes> {
        async move {
            let mut buf = BytesMut::zeroed(count);
            let mut filled = 0;
            while filled < count {
                let n = self.file.read(&mut buf[filled..]).await.map_err(io_error)?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            buf.truncate(filled);
            Ok(buf.freeze())
        }
        .boxed()
    }

    fn seek(&mut self, pos: SeekFrom) -> FsFuture<'_, u64> {
        async move { self.file.seek(pos).await.map_err(io_error) }.boxed()
    }

    fn flush(&mut self) -> FsFuture<'_, ()> {
        async move { self.file.flush().await.map_err(io_error) }.boxed()
    }
}

#[cfg(test)]
mod tests;
