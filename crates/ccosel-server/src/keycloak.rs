//! The Keycloak this server starts for itself when none is configured.
//!
//! It runs in Docker, listening on this machine only (`127.0.0.1`), under the `/idp` path so
//! [`crate::idp`] can forward its sign-in pages without rewriting anything. Browsers never
//! reach it directly; its admin console is only reachable from this machine, at
//! [`admin_console_url`], with the random password kept in [`ADMIN_PASSWORD_FILE`].
//!
//! Its data lives in a named volume, so accounts survive the container being replaced.

use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, bail};
use rand::Rng;
use rand::distributions::Alphanumeric;

use crate::idp;

pub const CONTAINER: &str = "ccosel-keycloak";
pub const VOLUME: &str = "ccosel-keycloak-data";
pub const PORT: u16 = 8080;
pub const REALM: &str = "ccosel";
pub const CLIENT_ID: &str = "ccosel";
const ADMIN_USER: &str = "admin";
const IMAGE: &str = "quay.io/keycloak/keycloak";

/// Stamped on the container. A container without the current value was made by an older
/// version of this server, with settings that are no longer safe (listening on every
/// interface, `admin`/`admin`), so it is replaced rather than reused. Bump the value whenever
/// [`run_args`] changes.
const CONFIG_LABEL: &str = "ccosel.config";
const CONFIG_VERSION: &str = "2";

/// Where the admin password is kept, relative to the directory the server runs in.
pub const ADMIN_PASSWORD_FILE: &str = "data/keycloak-admin-password";

/// What older versions of this server wrote into `.env` after starting Keycloak themselves. A
/// `.env` still holding it means "the managed Keycloak", not a Keycloak of the user's own.
const LEGACY_URL: &str = "http://localhost:8080/realms/ccosel";

pub fn is_legacy_url(url: &str) -> bool {
    url.trim_end_matches('/') == LEGACY_URL
}

/// Keycloak's origin, as this server reaches it.
pub fn upstream() -> String {
    format!("http://127.0.0.1:{PORT}")
}

/// The realm's base URL, as this server reaches it (token exchange, user info, sign-out).
pub fn realm_url() -> String {
    format!("{}{}/realms/{REALM}", upstream(), idp::PREFIX)
}

pub fn admin_console_url() -> String {
    format!("http://localhost:{PORT}{}/admin/", idp::PREFIX)
}

/// `docker run` arguments for the container. The admin password is not among them: it is
/// passed through the environment, so it never shows up in a process listing.
pub fn run_args() -> Vec<String> {
    [
        "run",
        "-d",
        "--name",
        CONTAINER,
        "--label",
        &format!("{CONFIG_LABEL}={CONFIG_VERSION}"),
        "--restart",
        "unless-stopped",
        // This machine only. Browsers reach the sign-in pages through `/idp` on this server.
        "-p",
        &format!("127.0.0.1:{PORT}:8080"),
        "-v",
        &format!("{VOLUME}:/opt/keycloak/data"),
        // Keycloak 26 reads the first pair and older releases the second, so set both.
        "-e",
        &format!("KC_BOOTSTRAP_ADMIN_USERNAME={ADMIN_USER}"),
        "-e",
        "KC_BOOTSTRAP_ADMIN_PASSWORD",
        "-e",
        &format!("KEYCLOAK_ADMIN={ADMIN_USER}"),
        "-e",
        "KEYCLOAK_ADMIN_PASSWORD",
        IMAGE,
        "start-dev",
        &format!("--http-relative-path={}", idp::PREFIX),
        "--proxy-headers=xforwarded",
        "--hostname-strict=false",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Where Keycloak may send a browser back to after signing in: this server as reached on this
/// machine, and the public URL if there is one.
pub fn redirect_uris(public_url: Option<&str>, port: u16) -> Vec<String> {
    let mut origins = vec![
        format!("http://localhost:{port}"),
        format!("http://127.0.0.1:{port}"),
    ];
    if let Some(url) = public_url {
        origins.push(url.trim_end_matches('/').to_owned());
    }
    origins
        .into_iter()
        .map(|o| format!("{o}/auth/callback"))
        .collect()
}

/// Starts (or reuses) the container and makes sure the realm and client exist, with the client
/// allowed to redirect to [`redirect_uris`].
pub async fn provision(public_url: Option<&str>, port: u16) -> anyhow::Result<()> {
    let status = Command::new("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        bail!("docker is required to start Keycloak — install it, or set CCOSEL_KEYCLOAK_URL");
    }

    let password = admin_password()?;
    ensure_running(&password).await?;
    let token = admin_token(&password).await?;
    ensure_realm(&token).await?;
    ensure_client(&token, &redirect_uris(public_url, port)).await
}

/// Reads the admin password, creating a random one the first time.
fn admin_password() -> anyhow::Result<String> {
    if let Ok(existing) = std::fs::read_to_string(ADMIN_PASSWORD_FILE) {
        return Ok(existing.trim().to_owned());
    }
    let password: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(24)
        .map(char::from)
        .collect();
    if let Some(dir) = std::path::Path::new(ADMIN_PASSWORD_FILE).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(
        &mut options
            .open(ADMIN_PASSWORD_FILE)
            .with_context(|| format!("creating {ADMIN_PASSWORD_FILE}"))?,
        password.as_bytes(),
    )?;
    println!("created a Keycloak admin password in {ADMIN_PASSWORD_FILE}");
    Ok(password)
}

/// `(config label, running)` of the existing container, if there is one.
fn inspect() -> Option<(String, bool)> {
    let out = Command::new("docker")
        .args([
            "inspect",
            "--format",
            &format!("{{{{index .Config.Labels \"{CONFIG_LABEL}\"}}}}|{{{{.State.Running}}}}"),
            CONTAINER,
        ])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (label, running) = text.trim().split_once('|')?;
    Some((label.to_owned(), running == "true"))
}

fn docker(args: &[&str]) -> anyhow::Result<()> {
    let out = Command::new("docker")
        .args(args)
        .stdout(Stdio::null())
        .output()?;
    if !out.status.success() {
        bail!(
            "docker {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

async fn ensure_running(password: &str) -> anyhow::Result<()> {
    match inspect() {
        Some((label, running)) if label == CONFIG_VERSION => {
            if !running {
                println!("starting the Keycloak container…");
                docker(&["start", CONTAINER])?;
            }
        }
        existing => {
            if existing.is_some() {
                println!(
                    "replacing the {CONTAINER} container made by an older CCOSEL: it listened on \
                     every network interface with admin/admin. Accounts created in it are not \
                     carried over; add them again in the new admin console."
                );
                docker(&["rm", "-f", CONTAINER])?;
            }
            println!("starting Keycloak…");
            let out = Command::new("docker")
                .args(run_args())
                .env("KC_BOOTSTRAP_ADMIN_PASSWORD", password)
                .env("KEYCLOAK_ADMIN_PASSWORD", password)
                .stdout(Stdio::null())
                .output()?;
            if !out.status.success() {
                bail!(
                    "could not start Keycloak: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
        }
    }

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let probe = format!("{}{}/realms/master", upstream(), idp::PREFIX);
    for _ in 0..90 {
        if http
            .get(&probe)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    bail!("Keycloak did not start within 90 seconds (docker logs {CONTAINER})")
}

async fn admin_token(password: &str) -> anyhow::Result<String> {
    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!(
            "{}{}/realms/master/protocol/openid-connect/token",
            upstream(),
            idp::PREFIX
        ))
        .form(&[
            ("grant_type", "password"),
            ("client_id", "admin-cli"),
            ("username", ADMIN_USER),
            ("password", password),
        ])
        .send()
        .await?
        .json()
        .await?;
    match resp["access_token"].as_str() {
        Some(token) => Ok(token.to_owned()),
        None => bail!(
            "Keycloak refused the admin password in {ADMIN_PASSWORD_FILE}. If you changed it in \
             the admin console, put the new one in that file. To start over instead (this \
             deletes every account): docker rm -f {CONTAINER} && docker volume rm {VOLUME}"
        ),
    }
}

fn admin_api(path: &str) -> String {
    format!("{}{}/admin/realms{path}", upstream(), idp::PREFIX)
}

async fn ensure_realm(token: &str) -> anyhow::Result<()> {
    let resp = reqwest::Client::new()
        .post(admin_api(""))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "realm": REALM,
            "enabled": true,
            "registrationAllowed": false,
            "loginWithEmailAllowed": true,
        }))
        .send()
        .await?;
    if resp.status().is_success() || resp.status().as_u16() == 409 {
        return Ok(());
    }
    bail!("could not create the {REALM} realm: {}", resp.text().await?)
}

/// Creates the client, or updates its redirect URIs if it exists, so a changed public URL
/// takes effect on the next start.
async fn ensure_client(token: &str, redirect_uris: &[String]) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let client = serde_json::json!({
        "clientId": CLIENT_ID,
        "protocol": "openid-connect",
        "publicClient": true,
        "standardFlowEnabled": true,
        "redirectUris": redirect_uris,
        "webOrigins": [],
    });

    let created = http
        .post(admin_api(&format!("/{REALM}/clients")))
        .bearer_auth(token)
        .json(&client)
        .send()
        .await?;
    if created.status().is_success() {
        return Ok(());
    }
    if created.status().as_u16() != 409 {
        bail!(
            "could not create the {CLIENT_ID} client: {}",
            created.text().await?
        );
    }

    let found: serde_json::Value = http
        .get(admin_api(&format!("/{REALM}/clients?clientId={CLIENT_ID}")))
        .bearer_auth(token)
        .send()
        .await?
        .json()
        .await?;
    let Some(id) = found[0]["id"].as_str() else {
        bail!("the {CLIENT_ID} client exists but could not be looked up: {found}");
    };
    let updated = http
        .put(admin_api(&format!("/{REALM}/clients/{id}")))
        .bearer_auth(token)
        .json(&client)
        .send()
        .await?;
    if updated.status().is_success() {
        return Ok(());
    }
    bail!(
        "could not update the {CLIENT_ID} client: {}",
        updated.text().await?
    )
}

#[cfg(test)]
mod tests;
