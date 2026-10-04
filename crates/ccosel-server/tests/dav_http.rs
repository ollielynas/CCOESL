//! WebDAV end to end: signing in with app passwords, and every method under the same
//! permissions as the shell, over a real socket against the real router.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ccosel_server::app_passwords::THROTTLE_FAILURES;
use ccosel_server::auth::{AuthState, OAuthConfig};
use ccosel_server::fs_api::Jail;
use reqwest::{Method, StatusCode};

struct Site {
    root: PathBuf,
    /// Login on: callers sign in with an app password.
    addr: SocketAddr,
    /// Login off: everyone is anonymous.
    open: SocketAddr,
    auth: AuthState,
    alice: String,
    bob: String,
    client: reqwest::Client,
}

async fn serve(dir: &std::path::Path, auth: AuthState) -> SocketAddr {
    let app = ccosel_server::app(Jail::new(dir).unwrap(), dir.to_path_buf(), auth);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    addr
}

/// ```text
/// Docs/            read: *
///   guide.md
/// Shared/          (open)
///   notes.md
///   project/
///     src/main.rs
///     locked/      read: *
///       a.txt
///   mixed/
///     c.txt
///     hidden/      read: bob, write: bob
///       b.txt
///   peek -> ../home/bob
/// home/bob/secret.txt
/// ```
async fn site() -> Site {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-dav-http-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    for d in [
        "Docs",
        "Shared/project/src",
        "Shared/project/locked",
        "Shared/mixed/hidden",
        "home/bob",
    ] {
        fs::create_dir_all(dir.join(d)).unwrap();
    }
    for (f, text) in [
        ("Docs/.access", "read: *\n"),
        ("Docs/guide.md", "# Guide\n"),
        ("Shared/notes.md", "milk\n"),
        ("Shared/project/src/main.rs", "fn main() {}\n"),
        ("Shared/project/locked/.access", "read: *\n"),
        ("Shared/project/locked/a.txt", "a\n"),
        ("Shared/mixed/c.txt", "c\n"),
        ("Shared/mixed/hidden/.access", "read: bob\nwrite: bob\n"),
        ("Shared/mixed/hidden/b.txt", "b\n"),
        ("home/bob/secret.txt", "bob's\n"),
    ] {
        fs::write(dir.join(f), text).unwrap();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink("../home/bob", dir.join("Shared/peek")).unwrap();

    // Login on, against a provider nobody will ever reach: WebDAV never talks to it.
    let auth = AuthState::new(Some(OAuthConfig::keycloak(
        "http://127.0.0.1:9/realms/test".to_owned(),
        "test".to_owned(),
        None,
    )));
    let alice = auth
        .app_passwords()
        .create("alice", "Laptop")
        .unwrap()
        .password;
    let bob = auth
        .app_passwords()
        .create("bob", "Phone")
        .unwrap()
        .password;
    let addr = serve(&dir, auth.clone()).await;
    let open = serve(&dir, AuthState::default()).await;
    Site {
        root: Jail::new(&dir).unwrap().root().to_path_buf(),
        addr,
        open,
        auth,
        alice,
        bob,
        client: reqwest::Client::new(),
    }
}

fn method(name: &str) -> Method {
    Method::from_bytes(name.as_bytes()).unwrap()
}

impl Site {
    fn password(&self, who: &str) -> &str {
        match who {
            "alice" => &self.alice,
            "bob" => &self.bob,
            _ => panic!("no such test user {who}"),
        }
    }

    /// A request to `/dav{path}`, signed in as `who` (`None`: to the login-off server).
    fn req(&self, verb: &str, path: &str, who: Option<&str>) -> reqwest::RequestBuilder {
        match who {
            Some(who) => self
                .client
                .request(method(verb), format!("http://{}/dav{path}", self.addr))
                .basic_auth(who, Some(self.password(who))),
            None => self
                .client
                .request(method(verb), format!("http://{}/dav{path}", self.open)),
        }
    }

    async fn status(&self, verb: &str, path: &str, who: Option<&str>) -> StatusCode {
        self.req(verb, path, who).send().await.unwrap().status()
    }

    async fn get(&self, path: &str, who: Option<&str>) -> (StatusCode, String) {
        let resp = self.req("GET", path, who).send().await.unwrap();
        (resp.status(), resp.text().await.unwrap())
    }

    async fn put(&self, path: &str, who: Option<&str>, body: &str) -> StatusCode {
        self.req("PUT", path, who)
            .body(body.to_owned())
            .send()
            .await
            .unwrap()
            .status()
    }

    /// `MOVE` or `COPY` from `path` to `dest`, both under `/dav`.
    async fn transfer(
        &self,
        verb: &str,
        path: &str,
        dest: &str,
        who: Option<&str>,
        overwrite: bool,
    ) -> StatusCode {
        let addr = if who.is_some() { self.addr } else { self.open };
        self.req(verb, path, who)
            .header("destination", format!("http://{addr}/dav{dest}"))
            .header("overwrite", if overwrite { "T" } else { "F" })
            .send()
            .await
            .unwrap()
            .status()
    }

    /// The names a `Depth: 1` `PROPFIND` of `path` lists, not counting `path` itself.
    async fn names(&self, path: &str, who: Option<&str>) -> Result<Vec<String>, StatusCode> {
        let resp = self
            .req("PROPFIND", path, who)
            .header("depth", "1")
            .send()
            .await
            .unwrap();
        if resp.status() != StatusCode::MULTI_STATUS {
            return Err(resp.status());
        }
        let body = resp.text().await.unwrap();
        let own = format!("/dav{path}").trim_end_matches('/').to_owned();
        let mut names: Vec<String> = body
            .split("href>")
            .skip(1)
            .step_by(2)
            .filter_map(|s| s.split('<').next())
            .map(|href| href.trim_end_matches('/').to_owned())
            .filter(|href| *href != own)
            .map(|href| href.rsplit('/').next().unwrap().to_owned())
            .collect();
        names.sort();
        Ok(names)
    }

    fn exists(&self, path: &str) -> bool {
        self.root.join(path).symlink_metadata().is_ok()
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root.join(path)).unwrap()
    }
}

// ---- signing in ----

#[tokio::test]
async fn signing_in_needs_a_login_and_one_of_its_app_passwords() {
    let s = site().await;
    let url = format!("http://{}/dav/", s.addr);

    let none = s.client.get(&url).send().await.unwrap();
    assert_eq!(none.status(), StatusCode::UNAUTHORIZED);
    let challenge = none.headers()["www-authenticate"].to_str().unwrap();
    assert!(
        challenge.starts_with("Basic realm=\"CCOSEL\""),
        "{challenge}"
    );

    let wrong = s.client.get(&url).basic_auth("alice", Some("guess"));
    assert_eq!(
        wrong.send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    // An app password works only for the login it belongs to.
    let swapped = s.client.get(&url).basic_auth("bob", Some(&s.alice));
    assert_eq!(
        swapped.send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let bearer = s.client.get(&url).bearer_auth(&s.alice);
    assert_eq!(
        bearer.send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(s.names("/", Some("alice")).await.unwrap().len(), 3);
}

#[tokio::test]
async fn a_revoked_app_password_stops_working_on_the_next_request() {
    let s = site().await;
    assert_eq!(
        s.status("PROPFIND", "/", Some("alice")).await,
        StatusCode::MULTI_STATUS
    );
    let id = s.auth.app_passwords().list("alice")[0].id.clone();
    s.auth.app_passwords().revoke("alice", &id).unwrap();
    assert_eq!(
        s.status("PROPFIND", "/", Some("alice")).await,
        StatusCode::UNAUTHORIZED
    );
    // Bob's is unaffected.
    assert_eq!(
        s.status("PROPFIND", "/", Some("bob")).await,
        StatusCode::MULTI_STATUS
    );
}

#[tokio::test]
async fn too_many_wrong_passwords_for_one_login_lock_it_for_a_while() {
    let s = site().await;
    let url = format!("http://{}/dav/", s.addr);
    for _ in 0..THROTTLE_FAILURES {
        let resp = s.client.get(&url).basic_auth("alice", Some("guess"));
        assert_eq!(
            resp.send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    // Now even the right password waits.
    let resp = s
        .client
        .get(&url)
        .basic_auth("alice", Some(&s.alice))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = resp.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry > 0 && retry <= 301, "{retry}");
}

#[tokio::test]
async fn too_many_wrong_passwords_from_one_address_lock_it_for_a_while() {
    let s = site().await;
    let url = format!("http://{}/dav/", s.addr);
    // One guess each for many logins, all from this test's address.
    for i in 0..THROTTLE_FAILURES {
        let resp = s
            .client
            .get(&url)
            .basic_auth(format!("user{i}"), Some("guess"));
        assert_eq!(
            resp.send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        s.status("PROPFIND", "/", Some("bob")).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn with_login_off_everyone_is_anonymous() {
    let s = site().await;
    assert_eq!(s.names("/", None).await.unwrap(), ["Docs", "Shared"]);
    assert_eq!(s.get("/Docs/guide.md", None).await.0, StatusCode::OK);
    assert_eq!(
        s.put("/Docs/new.md", None, "x").await,
        StatusCode::FORBIDDEN
    );
    // No home folder, and nobody else's.
    assert_eq!(s.names("/home", None).await, Err(StatusCode::NOT_FOUND));
    assert_eq!(
        s.get("/home/bob/secret.txt", None).await.0,
        StatusCode::NOT_FOUND
    );
    // Credentials change nothing: there is nobody to sign in as.
    let resp = s
        .client
        .get(format!("http://{}/dav/home/bob/secret.txt", s.open))
        .basic_auth("bob", Some(&s.bob))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---- listing ----

#[tokio::test]
async fn a_listing_shows_only_what_the_caller_may_read() {
    let s = site().await;
    // `.scratch` (the Compiler's) and `.access` are never listed.
    assert!(s.exists(ccosel_proto::scratch::DIR));
    assert_eq!(
        s.names("/", Some("alice")).await.unwrap(),
        ["Docs", "Shared", "home"]
    );
    assert_eq!(s.names("/Docs", Some("alice")).await.unwrap(), ["guide.md"]);
    // Each user sees their own home folder, made on their first request, and nobody else's.
    assert_eq!(s.names("/home", Some("alice")).await.unwrap(), ["alice"]);
    assert_eq!(s.names("/home", Some("bob")).await.unwrap(), ["bob"]);
    assert_eq!(
        s.names("/Shared/mixed", Some("alice")).await.unwrap(),
        ["c.txt"]
    );
    assert_eq!(
        s.names("/Shared/mixed", Some("bob")).await.unwrap(),
        ["c.txt", "hidden"]
    );
    assert_eq!(
        s.names("/home/bob", Some("alice")).await,
        Err(StatusCode::NOT_FOUND)
    );
}

#[tokio::test]
async fn an_infinite_depth_listing_is_refused() {
    let s = site().await;
    let resp = s
        .req("PROPFIND", "/", Some("alice"))
        .header("depth", "infinity")
        // `dav-server` would allow it for the litmus test suite; this server does not.
        .header("x-litmus", "props: 1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(resp.text().await.unwrap().contains("propfind-finite-depth"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_cannot_lend_a_private_folder_public_permissions() {
    let s = site().await;
    assert!(
        !s.names("/Shared", Some("alice"))
            .await
            .unwrap()
            .contains(&"peek".to_owned())
    );
    assert_eq!(
        s.get("/Shared/peek/secret.txt", Some("alice")).await.0,
        StatusCode::NOT_FOUND
    );
    assert_ne!(
        s.put("/Shared/peek/new.txt", Some("alice"), "x").await,
        StatusCode::CREATED
    );
    assert!(!s.exists("home/bob/new.txt"));
    assert_eq!(
        s.transfer(
            "COPY",
            "/Shared/peek/secret.txt",
            "/Shared/got.txt",
            Some("alice"),
            false
        )
        .await,
        StatusCode::NOT_FOUND
    );

    // Bob may follow it: it leads to his own folder.
    assert!(
        s.names("/Shared", Some("bob"))
            .await
            .unwrap()
            .contains(&"peek".to_owned())
    );
    assert_eq!(
        s.get("/Shared/peek/secret.txt", Some("bob")).await,
        (StatusCode::OK, "bob's\n".to_owned())
    );
    // Removing a link removes the link, not what it leads to.
    assert_eq!(
        s.status("DELETE", "/Shared/peek", Some("bob")).await,
        StatusCode::NO_CONTENT
    );
    assert!(!s.exists("Shared/peek"));
    assert!(s.exists("home/bob/secret.txt"));
}

// ---- reading and writing ----

#[tokio::test]
async fn files_are_read_and_written_under_the_folders_rules() {
    let s = site().await;
    assert_eq!(
        s.get("/Docs/guide.md", Some("alice")).await,
        (StatusCode::OK, "# Guide\n".to_owned())
    );
    assert_eq!(
        s.put("/Docs/guide.md", Some("alice"), "vandalised").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(s.read("Docs/guide.md"), "# Guide\n");
    assert_eq!(
        s.status("MKCOL", "/Docs/new", Some("alice")).await,
        StatusCode::FORBIDDEN
    );

    assert_eq!(
        s.put("/Shared/new.txt", Some("alice"), "hello").await,
        StatusCode::CREATED
    );
    assert_eq!(s.read("Shared/new.txt"), "hello");
    assert_eq!(
        s.put("/Shared/new.txt", Some("bob"), "replaced").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        s.get("/Shared/new.txt", Some("alice")).await,
        (StatusCode::OK, "replaced".to_owned())
    );
    assert_eq!(
        s.status("MKCOL", "/Shared/dir", Some("alice")).await,
        StatusCode::CREATED
    );
    assert!(s.root.join("Shared/dir").is_dir());

    assert_eq!(
        s.put("/home/alice/mine.txt", Some("alice"), "private")
            .await,
        StatusCode::CREATED
    );
    assert_eq!(
        s.get("/home/alice/mine.txt", Some("bob")).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn another_users_home_is_as_absent_as_a_missing_folder() {
    let s = site().await;
    assert_eq!(
        s.get("/home/bob/secret.txt", Some("alice")).await.0,
        StatusCode::NOT_FOUND
    );
    assert_ne!(
        s.put("/home/bob/new.txt", Some("alice"), "x").await,
        StatusCode::CREATED
    );
    assert!(!s.exists("home/bob/new.txt"));
    assert_ne!(
        s.status("MKCOL", "/home/bob/dir", Some("alice")).await,
        StatusCode::CREATED
    );
    assert!(!s.exists("home/bob/dir"));
    assert_eq!(
        s.status("DELETE", "/home/bob/secret.txt", Some("alice"))
            .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.status("DELETE", "/home/bob", Some("alice")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.transfer(
            "MOVE",
            "/home/bob/secret.txt",
            "/Shared/s.txt",
            Some("alice"),
            false
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.transfer(
            "COPY",
            "/home/bob/secret.txt",
            "/Shared/s.txt",
            Some("alice"),
            false
        )
        .await,
        StatusCode::NOT_FOUND
    );
    // Somewhere to put it that does not exist, for all alice can tell.
    assert_eq!(
        s.transfer(
            "COPY",
            "/Shared/notes.md",
            "/home/bob/notes.md",
            Some("alice"),
            false
        )
        .await,
        StatusCode::CONFLICT
    );
    assert!(s.exists("home/bob/secret.txt"));
    assert!(!s.exists("Shared/s.txt"));
    assert!(!s.exists("home/bob/notes.md"));
}

#[tokio::test]
async fn access_files_are_invisible_and_untouchable() {
    let s = site().await;
    for status in [
        s.get("/Docs/.access", Some("alice")).await.0,
        s.get("/Shared/mixed/hidden/.access", Some("bob")).await.0,
        s.put("/Shared/.access", Some("alice"), "write: *").await,
        s.put("/Docs/.access", Some("alice"), "write: *").await,
        s.status("MKCOL", "/Shared/.access", Some("alice")).await,
        s.status("DELETE", "/Shared/mixed/hidden/.access", Some("bob"))
            .await,
        s.status("PROPFIND", "/Docs/.access", Some("alice")).await,
        s.transfer(
            "COPY",
            "/Docs/.access",
            "/Shared/copied",
            Some("alice"),
            false,
        )
        .await,
        s.transfer(
            "MOVE",
            "/Shared/notes.md",
            "/Shared/.access",
            Some("alice"),
            true,
        )
        .await,
        s.transfer(
            "COPY",
            "/Shared/notes.md",
            "/Shared/.access",
            Some("alice"),
            true,
        )
        .await,
    ] {
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    assert!(!s.exists("Shared/.access"));
    assert!(!s.exists("Shared/copied"));
    assert_eq!(s.read("Docs/.access"), "read: *\n");
    assert_eq!(
        s.read("Shared/mixed/hidden/.access"),
        "read: bob\nwrite: bob\n"
    );
    assert!(s.exists("Shared/notes.md"));
}

#[tokio::test]
async fn an_upload_over_the_cap_is_refused_before_it_is_sent() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let s = site().await;
    let mut stream = tokio::net::TcpStream::connect(s.open).await.unwrap();
    let head = format!(
        "PUT /dav/Shared/big.bin HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        s.open,
        ccosel_server::upload_api::MAX_UPLOAD_BYTES + 1
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.starts_with("HTTP/1.1 413"), "{text}");
    assert!(!s.exists("Shared/big.bin"));
}

#[tokio::test]
async fn locking_is_offered_so_clients_mount_read_write() {
    let s = site().await;
    let options = s.req("OPTIONS", "/", Some("alice")).send().await.unwrap();
    let dav = options.headers()["dav"].to_str().unwrap();
    assert!(dav.contains('2'), "{dav}");

    let lock = s
        .req("LOCK", "/Shared/notes.md", Some("alice"))
        .header("timeout", "Second-60")
        .body(
            "<?xml version=\"1.0\"?><D:lockinfo xmlns:D=\"DAV:\"><D:lockscope><D:exclusive/>\
             </D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(lock.status(), StatusCode::OK);
    let token = lock.headers()["lock-token"].to_str().unwrap().to_owned();
    let unlock = s
        .req("UNLOCK", "/Shared/notes.md", Some("alice"))
        .header("lock-token", token)
        .send()
        .await
        .unwrap();
    assert_eq!(unlock.status(), StatusCode::NO_CONTENT);
}

// ---- delete, move, copy ----

#[tokio::test]
async fn deleting_a_folder_needs_write_on_everything_in_it() {
    let s = site().await;
    // `locked` is read-only, so its parent cannot go either, and nothing is removed.
    assert_eq!(
        s.status("DELETE", "/Shared/project", Some("alice")).await,
        StatusCode::FORBIDDEN
    );
    assert!(s.exists("Shared/project/src/main.rs"));
    assert!(s.exists("Shared/project/locked/a.txt"));
    // Neither can a folder holding one alice cannot even see.
    assert_eq!(
        s.status("DELETE", "/Shared/mixed", Some("alice")).await,
        StatusCode::FORBIDDEN
    );
    assert!(s.exists("Shared/mixed/c.txt"));
    assert!(s.exists("Shared/mixed/hidden/b.txt"));
    // Nor anything in a read-only folder.
    assert_eq!(
        s.status("DELETE", "/Shared/project/locked/a.txt", Some("alice"))
            .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        s.status("DELETE", "/Docs/guide.md", Some("alice")).await,
        StatusCode::FORBIDDEN
    );

    // What is all writable goes.
    assert_eq!(
        s.status("DELETE", "/Shared/project/src", Some("alice"))
            .await,
        StatusCode::NO_CONTENT
    );
    assert!(!s.exists("Shared/project/src"));
    assert_eq!(
        s.status("DELETE", "/Shared/notes.md", Some("alice")).await,
        StatusCode::NO_CONTENT
    );
    assert!(!s.exists("Shared/notes.md"));
    // Bob may write in `hidden`, so for him `mixed` goes, `hidden` and all.
    assert_eq!(
        s.status("DELETE", "/Shared/mixed", Some("bob")).await,
        StatusCode::NO_CONTENT
    );
    assert!(!s.exists("Shared/mixed"));
}

#[tokio::test]
async fn nobody_can_delete_their_home_folder_or_the_root() {
    let s = site().await;
    assert_eq!(
        s.put("/home/alice/a.txt", Some("alice"), "a").await,
        StatusCode::CREATED
    );
    assert_eq!(
        s.status("DELETE", "/home/alice", Some("alice")).await,
        StatusCode::FORBIDDEN
    );
    assert!(s.exists("home/alice/a.txt"));
    assert_eq!(s.status("DELETE", "/", None).await, StatusCode::FORBIDDEN);
    assert!(s.exists("Shared/notes.md"));
    // But everything in it is hers to remove.
    assert_eq!(
        s.status("DELETE", "/home/alice/a.txt", Some("alice")).await,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn moving_needs_removal_from_where_it_was_and_write_where_it_goes() {
    let s = site().await;
    assert_eq!(
        s.transfer(
            "MOVE",
            "/Shared/notes.md",
            "/home/alice/notes.md",
            Some("alice"),
            false
        )
        .await,
        StatusCode::CREATED
    );
    assert!(!s.exists("Shared/notes.md"));
    assert_eq!(s.read("home/alice/notes.md"), "milk\n");

    // Out of a read-only folder.
    assert_eq!(
        s.transfer(
            "MOVE",
            "/Docs/guide.md",
            "/home/alice/guide.md",
            Some("alice"),
            false
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert!(s.exists("Docs/guide.md"));
    // Into one.
    assert_eq!(
        s.transfer(
            "MOVE",
            "/home/alice/notes.md",
            "/Docs/notes.md",
            Some("alice"),
            false
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert!(s.exists("home/alice/notes.md"));
    // A folder with something read-only inside.
    assert_eq!(
        s.transfer(
            "MOVE",
            "/Shared/project",
            "/home/alice/project",
            Some("alice"),
            false
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert!(s.exists("Shared/project/locked/a.txt"));
    // A whole writable folder.
    assert_eq!(
        s.transfer(
            "MOVE",
            "/Shared/project/src",
            "/home/alice/src",
            Some("alice"),
            false
        )
        .await,
        StatusCode::CREATED
    );
    assert_eq!(s.read("home/alice/src/main.rs"), "fn main() {}\n");
    // Into itself.
    assert!(
        !s.transfer(
            "MOVE",
            "/home/alice/src",
            "/home/alice/src/inner",
            Some("alice"),
            false
        )
        .await
        .is_success()
    );
    assert!(s.exists("home/alice/src/main.rs"));
}

#[tokio::test]
async fn replacing_what_is_at_the_destination_needs_permission_to_remove_it() {
    let s = site().await;
    s.put("/Shared/draft.md", Some("alice"), "draft").await;
    // Over a read-only file: refused before anything is removed.
    assert_eq!(
        s.transfer(
            "MOVE",
            "/Shared/draft.md",
            "/Docs/guide.md",
            Some("alice"),
            true
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(s.read("Docs/guide.md"), "# Guide\n");
    assert_eq!(s.read("Shared/draft.md"), "draft");
    // Over a folder with something read-only inside.
    assert_eq!(
        s.transfer(
            "COPY",
            "/Shared/draft.md",
            "/Shared/project",
            Some("alice"),
            true
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert!(s.exists("Shared/project/locked/a.txt"));
    // Without `Overwrite`, nothing is replaced at all.
    assert_eq!(
        s.transfer(
            "MOVE",
            "/Shared/draft.md",
            "/Shared/notes.md",
            Some("alice"),
            false
        )
        .await,
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(s.read("Shared/notes.md"), "milk\n");
    // With it, and permission, it is.
    assert_eq!(
        s.transfer(
            "MOVE",
            "/Shared/draft.md",
            "/Shared/notes.md",
            Some("alice"),
            true
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(s.read("Shared/notes.md"), "draft");
}

#[tokio::test]
async fn copying_needs_read_on_all_of_the_source_and_write_where_it_goes() {
    let s = site().await;
    // Out of a read-only folder is fine: it only needs reading.
    assert_eq!(
        s.transfer(
            "COPY",
            "/Docs/guide.md",
            "/home/alice/guide.md",
            Some("alice"),
            false
        )
        .await,
        StatusCode::CREATED
    );
    assert_eq!(s.read("home/alice/guide.md"), "# Guide\n");
    assert_eq!(s.read("Docs/guide.md"), "# Guide\n");
    // Into one is not.
    assert_eq!(
        s.transfer(
            "COPY",
            "/Shared/notes.md",
            "/Docs/notes.md",
            Some("alice"),
            false
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert!(!s.exists("Docs/notes.md"));
    // A folder holding something alice cannot see is refused whole, rather than copied
    // without it.
    assert_eq!(
        s.transfer(
            "COPY",
            "/Shared/mixed",
            "/home/alice/mixed",
            Some("alice"),
            false
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert!(!s.exists("home/alice/mixed"));
    // A folder alice can read all of is copied, read-only parts included. The copy takes the
    // rules of where it lands; the `.access` file is not copied.
    assert_eq!(
        s.transfer(
            "COPY",
            "/Shared/project",
            "/home/alice/project",
            Some("alice"),
            false
        )
        .await,
        StatusCode::CREATED
    );
    assert_eq!(s.read("home/alice/project/src/main.rs"), "fn main() {}\n");
    assert_eq!(s.read("home/alice/project/locked/a.txt"), "a\n");
    assert!(!s.exists("home/alice/project/locked/.access"));
}

#[tokio::test]
async fn a_destination_outside_dav_is_refused() {
    let s = site().await;
    let resp = s
        .req("COPY", "/Shared/notes.md", Some("alice"))
        .header("destination", format!("http://{}/davish/notes.md", s.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    assert!(!s.exists("sh/notes.md"));
}
