//! `cargo xtask test-image`: build the Docker image and use it the way a person would.
//!
//! The image is how CCOSEL is actually run, so this is the check that it works, not that its
//! parts compile. "The page loads" proves little on its own: if Keycloak fails to start, the
//! server turns login off and keeps serving. So this signs in for real, through `/idp`, with an
//! account added through the admin console the README points people at, then does what the
//! apps do (RPC, upload, download, a build in the Compiler), and finally replaces the container
//! on the same volume to check that what people keep survives an upgrade.
//!
//! Everything goes over HTTP with `curl`, as a browser would. The container is published on
//! free loopback ports, so a dev server on 8777 or the dev Keycloak on 8080 doesn't get in the
//! way, and the container and volume it makes are removed however it ends.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ccosel_proto::account::WhoAmI;
use ccosel_proto::build::{Compile, CompileReq};
use ccosel_proto::fs::{ListDir, ListDirReq};
use ccosel_proto::{Rpc, WireReply, WireRequest, WireResult};

use crate::{GUESTS, root, run};

/// The tag the image is built as. Fixed, so `--no-build` can find the last one.
const IMAGE: &str = "ccosel:test-image";
/// The account the check signs in as.
const USER: &str = "tester";
/// How long a start may take. Keycloak's first start sets up its database, which is most of it.
const START_TIMEOUT: Duration = Duration::from_secs(300);
/// How long the Compiler may take to build a project with no dependencies.
const BUILD_TIMEOUT: Duration = Duration::from_secs(180);

const HELLO_MANIFEST: &str =
    "[package]\nname = \"hello\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n";
const HELLO_OUTPUT: &str = "hello from the ccosel image";

pub fn test_image(args: &[String]) -> Result<()> {
    let build = match args {
        [] => true,
        [flag] if flag == "--no-build" => false,
        _ => bail!("usage: cargo xtask test-image [--no-build]"),
    };
    if Command::new("docker").arg("version").output().is_err() {
        bail!("docker is required for test-image: https://docs.docker.com/engine/install/");
    }
    if Command::new("curl").arg("--version").output().is_err() {
        bail!("curl is required for test-image");
    }
    let root = root();
    if build {
        println!("\n==> docker build -t {IMAGE} .");
        run(&root, "docker", &["build", "-t", IMAGE, "."])?;
    }

    let run = Run::new()?;
    let result = check(&run);
    if result.is_err() {
        run.print_logs();
    }
    result?;
    println!("\ntest-image: the image works");
    Ok(())
}

/// Everything a person does with the image, in order. Each step says what it is checking, so a
/// failure in a long CI log says which part of the product broke.
fn check(run: &Run) -> Result<()> {
    let password = format!("Pw-{}-{}", std::process::id(), nanos());

    println!("\n==> first start, on a fresh volume");
    run.start()?;
    let browser = run.browser("first")?;
    run.wait_until_serving(&browser)?;

    println!("==> the boot page, the shell and every app are served");
    check_static(&browser)?;

    println!("==> login is on, and only the sign-in pages are forwarded");
    check_signed_out(&browser)?;

    println!("==> adding an account through the admin console");
    add_account(run, &password)?;

    println!("==> signing in through /idp");
    sign_in(&browser, &password)?;

    println!("==> the apps' calls work once signed in");
    check_signed_in(&browser)?;

    println!("==> uploading, downloading, and building in the Compiler");
    let project = format!("/home/{USER}/hello");
    upload(&browser, &project, "Cargo.toml", HELLO_MANIFEST.as_bytes())?;
    let main_rs = format!("fn main() {{\n    println!(\"{HELLO_OUTPUT}\");\n}}\n");
    upload(
        &browser,
        &format!("{project}/src"),
        "main.rs",
        main_rs.as_bytes(),
    )?;
    expect_file(&browser, &format!("{project}/Cargo.toml"), HELLO_MANIFEST)?;
    compile(run, &browser, &project)?;

    println!("\n==> replacing the container, keeping the volume");
    run.replace()?;
    let browser = run.browser("second")?;
    run.wait_until_serving(&browser)?;
    check_signed_out(&browser)?;
    println!("==> the account and the files survived");
    sign_in(&browser, &password)?;
    check_signed_in(&browser)?;
    expect_file(&browser, &format!("{project}/Cargo.toml"), HELLO_MANIFEST)?;
    Ok(())
}

fn check_static(browser: &Http) -> Result<()> {
    let page = browser.get("/")?.ok()?;
    if !String::from_utf8_lossy(&page.body).contains("<title>CCOSEL</title>") {
        bail!("GET / did not serve the boot page");
    }
    browser.get("/dist/ccosel-shell.js")?.ok()?;
    let mut modules = vec!["ccosel-shell_bg".to_owned()];
    modules.extend(GUESTS.iter().map(|(_, served)| (*served).to_owned()));
    for module in modules {
        let path = format!("/dist/{module}.wasm");
        let resp = browser.get(&path)?.ok()?;
        if !resp.body.starts_with(b"\0asm") {
            bail!("GET {path} is not a WebAssembly module");
        }
    }
    Ok(())
}

fn check_signed_out(browser: &Http) -> Result<()> {
    let me = browser.get("/auth/me")?.json()?;
    if me["configured"] != true {
        bail!(
            "login is off: the server could not start or set up Keycloak (see its log below), \
             so it is letting everyone in. /auth/me said {me}"
        );
    }
    if me["authenticated"] != false {
        bail!("a new browser is already signed in: /auth/me said {me}");
    }
    browser.post("/rpc", &[])?.expect(401)?;
    browser.get("/files/Docs/README.md")?.expect(401)?;
    // Through the public port, only the ccosel realm's pages. The admin console and the
    // master realm must never be reachable from there.
    browser.get("/idp/admin/")?.expect(404)?;
    browser.get("/idp/realms/master")?.expect(404)?;
    browser.get("/idp/realms/ccosel")?.ok()?;
    Ok(())
}

/// Adds [`USER`] the way the README says to: the admin password from the file in the volume,
/// and the admin console on its own port. Driven through the admin REST API rather than the
/// console's pages, which are a JavaScript app, but on the same port and credentials.
fn add_account(run: &Run, password: &str) -> Result<()> {
    let admin_password = capture(&[
        "exec",
        &run.name,
        "cat",
        "/data/ccosel/keycloak-admin-password",
    ])
    .context("reading the admin password where the README says it is")?;
    let admin = Http::new(
        format!("http://127.0.0.1:{}/idp", run.admin_port),
        run.dir.join("admin"),
    )?;
    admin
        .follow("/admin/", &[])?
        .ok()
        .context("the admin console")?;

    let token = admin
        .form(
            "/realms/master/protocol/openid-connect/token",
            &[
                ("grant_type", "password"),
                ("client_id", "admin-cli"),
                ("username", "admin"),
                ("password", &admin_password),
            ],
        )?
        .json()?;
    let Some(token) = token["access_token"].as_str() else {
        bail!("Keycloak refused the admin password from the volume: {token}");
    };
    // First and last name and email filled in, or Keycloak asks for them at first sign-in.
    let account = serde_json::json!({
        "username": USER,
        "enabled": true,
        "email": format!("{USER}@example.com"),
        "emailVerified": true,
        "firstName": "Test",
        "lastName": "User",
        "credentials": [{ "type": "password", "value": password, "temporary": false }],
    });
    admin
        .request(
            "/admin/realms/ccosel/users",
            &[
                "-H".into(),
                format!("authorization: Bearer {token}"),
                "-H".into(),
                "content-type: application/json".into(),
                "--data-binary".into(),
                account.to_string(),
            ],
        )?
        .expect(201)
        .context("creating the account")?;
    Ok(())
}

/// What a browser does from the Sign in button: `/auth/login` to Keycloak's form (forwarded
/// under `/idp`), post the form, and follow the redirects back to the desktop.
fn sign_in(browser: &Http, password: &str) -> Result<()> {
    let form = browser.follow("/auth/login", &[])?.ok()?;
    let expected = format!("{}/idp/realms/ccosel/", browser.base);
    if !form.url.starts_with(&expected) {
        bail!(
            "/auth/login went to {}, not the sign-in page under {expected}",
            form.url
        );
    }
    let action = login_form_action(&String::from_utf8_lossy(&form.body))
        .context("the sign-in page has no login form")?;
    if !action.starts_with(&expected) {
        bail!("the login form posts to {action}, not through this server's /idp");
    }
    let path = &action[browser.base.len()..];
    let landed = browser
        .follow(
            path,
            &[
                "--data-urlencode".into(),
                format!("username={USER}"),
                "--data-urlencode".into(),
                format!("password={password}"),
            ],
        )?
        .ok()
        .context("posting the sign-in form")?;
    if landed.url != format!("{}/", browser.base) {
        bail!(
            "signing in ended at {}, not back on the desktop:\n{}",
            landed.url,
            snippet(&landed.body)
        );
    }
    let me = browser.get("/auth/me")?.json()?;
    if me["authenticated"] != true || me["login"] != USER {
        bail!("signed in, but /auth/me said {me}");
    }
    Ok(())
}

fn check_signed_in(browser: &Http) -> Result<()> {
    let account = browser.rpc::<WhoAmI>(&())?;
    if !account.login_enabled || account.name.as_deref() != Some(USER) {
        bail!("WhoAmI answered {account:?}");
    }
    let listing = browser.rpc::<ListDir>(&ListDirReq { path: "/" })?;
    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    for expected in ["Docs", "home"] {
        if !names.contains(&expected) {
            bail!("the root folder has no {expected}: {names:?}");
        }
    }
    expect_file(browser, "/Docs/README.md", "")?;
    Ok(())
}

fn upload(browser: &Http, dir: &str, filename: &str, bytes: &[u8]) -> Result<()> {
    browser
        .post(&format!("/upload?path={dir}&filename={filename}"), bytes)?
        .ok()
        .with_context(|| format!("uploading {dir}/{filename}"))?;
    Ok(())
}

/// Downloads `path` and, unless `expected` is empty, checks it holds exactly that.
fn expect_file(browser: &Http, path: &str, expected: &str) -> Result<()> {
    let resp = browser.get(&format!("/files{path}"))?.ok()?;
    if !expected.is_empty() && resp.body != expected.as_bytes() {
        bail!("{path} came back as {}", snippet(&resp.body));
    }
    if resp.body.is_empty() {
        bail!("{path} came back empty");
    }
    Ok(())
}

/// Builds `project` the way the Compiler app does (start, then poll the same generation), and
/// runs what it built inside the container.
fn compile(run: &Run, browser: &Http, project: &str) -> Result<()> {
    let req = CompileReq {
        path: project,
        generation: 1,
    };
    let started = Instant::now();
    let result = loop {
        let status = browser.rpc::<Compile>(&req)?;
        if let Some(result) = status.result {
            break result;
        }
        if started.elapsed() > BUILD_TIMEOUT {
            bail!("the Compiler's build did not finish in {BUILD_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    if !result.success {
        bail!("the Compiler's build failed:\n{}", result.output);
    }
    let Some(binary) = result.binaries.first() else {
        bail!("the Compiler's build succeeded but produced no binary");
    };
    expect_file(
        browser,
        &format!("/{}", binary.path.trim_start_matches('/')),
        "",
    )?;
    let path = format!("/data/files/{}", binary.path.trim_start_matches('/'));
    let said = capture(&["exec", &run.name, &path])?;
    if said != HELLO_OUTPUT {
        bail!("the built program printed {said:?}");
    }
    Ok(())
}

/// The `action` of Keycloak's login form, with its HTML escaping undone.
fn login_form_action(html: &str) -> Option<String> {
    let form = &html[html.find("id=\"kc-form-login\"")?..];
    let form = &form[..form.find('>')?];
    let start = form.find("action=\"")? + "action=\"".len();
    let end = start + form[start..].find('"')?;
    Some(form[start..end].replace("&amp;", "&"))
}

/// The start of a body, for an error message.
fn snippet(body: &[u8]) -> String {
    String::from_utf8_lossy(&body[..body.len().min(600)]).into_owned()
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

/// `docker <args>`, returning what it printed.
fn capture(args: &[&str]) -> Result<String> {
    crate::capture(&root(), "docker", args)
}

/// The container under test, its volume and ports, and a scratch folder for `curl`. Dropping it
/// removes all of them, whether the check passed or not.
struct Run {
    name: String,
    volume: String,
    port: u16,
    admin_port: u16,
    dir: PathBuf,
}

impl Run {
    fn new() -> Result<Self> {
        let name = format!("ccosel-test-image-{}", std::process::id());
        let dir = std::env::temp_dir().join(&name);
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            volume: format!("{name}-data"),
            name,
            port: free_port()?,
            admin_port: free_port()?,
            dir,
        })
    }

    /// The command the README gives, on this run's ports. `CCOSEL_PUBLIC_URL` is what lets
    /// Keycloak send the browser back to a port other than 8777, as it does behind a tunnel.
    fn start(&self) -> Result<()> {
        let url = format!("http://localhost:{}", self.port);
        capture(&[
            "run",
            "-d",
            "--name",
            &self.name,
            "-p",
            &format!("127.0.0.1:{}:8777", self.port),
            "-p",
            &format!("127.0.0.1:{}:8080", self.admin_port),
            "-v",
            &format!("{}:/data", self.volume),
            "-e",
            &format!("CCOSEL_PUBLIC_URL={url}"),
            IMAGE,
        ])?;
        Ok(())
    }

    /// What upgrading looks like: the container goes, the volume stays.
    fn replace(&self) -> Result<()> {
        capture(&["rm", "-f", &self.name])?;
        self.start()
    }

    /// A browser with no cookies, pointed at this run.
    fn browser(&self, label: &str) -> Result<Http> {
        Http::new(
            format!("http://localhost:{}", self.port),
            self.dir.join(label),
        )
    }

    fn wait_until_serving(&self, browser: &Http) -> Result<()> {
        let started = Instant::now();
        loop {
            if browser.get("/auth/me").is_ok_and(|r| r.status == 200) {
                println!("    serving after {}s", started.elapsed().as_secs());
                return Ok(());
            }
            let running = capture(&["inspect", "-f", "{{.State.Running}}", &self.name])?;
            if running != "true" {
                bail!("the container stopped before it served anything");
            }
            if started.elapsed() > START_TIMEOUT {
                bail!("the container was not serving within {START_TIMEOUT:?}");
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    fn print_logs(&self) {
        println!("\n==> the container's log (last 200 lines)");
        let _ = Command::new("docker")
            .args(["logs", "--tail", "200", &self.name])
            .status();
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let quiet = |args: &[&str]| {
            let _ = Command::new("docker").args(args).output();
        };
        quiet(&["rm", "-f", &self.name]);
        quiet(&["volume", "rm", "-f", &self.volume]);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A browser, as far as this check needs one: `curl` with its own cookie jar.
struct Http {
    base: String,
    dir: PathBuf,
}

struct Response {
    status: u16,
    /// Where the request ended up, after any redirects it followed.
    url: String,
    body: Vec<u8>,
}

impl Response {
    fn expect(self, status: u16) -> Result<Self> {
        if self.status != status {
            bail!(
                "{} answered {} where {status} was expected:\n{}",
                self.url,
                self.status,
                snippet(&self.body)
            );
        }
        Ok(self)
    }

    fn ok(self) -> Result<Self> {
        self.expect(200)
    }

    fn json(self) -> Result<serde_json::Value> {
        let resp = self.ok()?;
        serde_json::from_slice(&resp.body)
            .with_context(|| format!("{} is not JSON: {}", resp.url, snippet(&resp.body)))
    }
}

impl Http {
    fn new(base: String, dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("cookies"), "")?;
        Ok(Self { base, dir })
    }

    fn get(&self, path: &str) -> Result<Response> {
        self.request(path, &[])
    }

    /// A GET, or with `form` a form POST, following redirects the way a browser does.
    fn follow(&self, path: &str, form: &[String]) -> Result<Response> {
        let mut args = vec!["-L".to_owned()];
        args.extend_from_slice(form);
        self.request(path, &args)
    }

    fn post(&self, path: &str, body: &[u8]) -> Result<Response> {
        let file = self.dir.join("request");
        std::fs::write(&file, body)?;
        self.request(
            path,
            &[
                "-H".into(),
                "content-type: application/octet-stream".into(),
                "--data-binary".into(),
                format!("@{}", file.display()),
            ],
        )
    }

    fn form(&self, path: &str, fields: &[(&str, &str)]) -> Result<Response> {
        let mut args = Vec::new();
        for (k, v) in fields {
            args.push("--data-urlencode".to_owned());
            args.push(format!("{k}={v}"));
        }
        self.request(path, &args)
    }

    /// One call, as the shell makes it: a batch of one on `POST /rpc`.
    fn rpc<M: Rpc>(&self, req: &M::Req<'_>) -> Result<M::Reply> {
        let args = postcard::to_allocvec(req)?;
        // A `Vec`, not an array: postcard writes a sequence's length, but not an array's.
        let batch = postcard::to_allocvec(&vec![WireRequest {
            seq: 1,
            method: M::METHOD as u16,
            args: &args,
        }])?;
        let resp = self.post("/rpc", &batch)?.ok()?;
        let replies: Vec<WireReply> =
            postcard::from_bytes(&resp.body).context("the /rpc reply is not a batch")?;
        match replies.first().map(|r| &r.result) {
            Some(WireResult::Ok(bytes)) => Ok(postcard::from_bytes(bytes)?),
            Some(WireResult::Err { code, detail }) => {
                bail!("{:?} failed with code {code}: {detail}", M::METHOD)
            }
            None => bail!("/rpc answered an empty batch"),
        }
    }

    fn request(&self, path: &str, args: &[String]) -> Result<Response> {
        let body = self.dir.join("body");
        let jar = self.dir.join("cookies");
        let url = format!("{}{path}", self.base);
        let out = Command::new("curl")
            .args([
                "-sS",
                "--max-time",
                "60",
                "-w",
                "%{http_code} %{url_effective}",
            ])
            .args(cookie_args(&jar))
            .arg("-o")
            .arg(&body)
            .args(args)
            .arg(&url)
            .output()
            .context("running curl")?;
        if !out.status.success() {
            bail!("{url}: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        let written = String::from_utf8_lossy(&out.stdout).into_owned();
        let (status, url) = written
            .split_once(' ')
            .with_context(|| format!("curl printed {written:?}"))?;
        Ok(Response {
            status: status.parse()?,
            url: url.to_owned(),
            body: std::fs::read(&body).unwrap_or_default(),
        })
    }
}

/// Read and write the same jar, so cookies set during a redirect chain are sent on later hops
/// and later requests.
fn cookie_args(jar: &Path) -> [std::ffi::OsString; 4] {
    [
        "-b".into(),
        jar.as_os_str().to_owned(),
        "-c".into(),
        jar.as_os_str().to_owned(),
    ]
}

#[cfg(test)]
mod tests;
