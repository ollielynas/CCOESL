//! `ccosel-server` binary.

use std::net::SocketAddr;
use std::path::PathBuf;

use ccosel_server::auth::{ApprovedUsers, AuthState, OAuthConfig};
use ccosel_server::fs_api::Jail;

const KEYCLOAK_REALM: &str = "ccosel";
const KEYCLOAK_CLIENT_ID: &str = "ccosel";
const KEYCLOAK_ADMIN_USER: &str = "admin";
const KEYCLOAK_ADMIN_PASS: &str = "admin";
const KEYCLOAK_CONTAINER_NAME: &str = "ccosel-keycloak";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let mut args = std::env::args().skip(1);
    let mut root = PathBuf::from("data/shared");
    let mut web = PathBuf::from("web");
    let mut port = 8777u16;
    let mut approved_users_path = PathBuf::from("data/approved_users.txt");

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = PathBuf::from(args.next().unwrap_or_default()),
            "--web" => web = PathBuf::from(args.next().unwrap_or_default()),
            "--port" => port = args.next().unwrap_or_default().parse().unwrap_or(8777),
            "--approved-users" => {
                approved_users_path = PathBuf::from(args.next().unwrap_or_default())
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    std::fs::create_dir_all(&root)?;
    let jail = Jail::new(&root)?;
    println!("serving files from {}", jail.root().display());

    let approved = match ApprovedUsers::load(&approved_users_path) {
        Ok(list) => {
            println!(
                "loaded {} approved account(s) from {}",
                list.len(),
                approved_users_path.display()
            );
            list
        }
        Err(e) => {
            println!(
                "warning: could not read {} ({e}); no accounts are approved until it exists",
                approved_users_path.display()
            );
            ApprovedUsers::default()
        }
    };

    let keycloak_url = std::env::var("CCOSEL_KEYCLOAK_URL").ok();
    let client_id = std::env::var("CCOSEL_KEYCLOAK_CLIENT_ID").ok();
    let client_secret = std::env::var("CCOSEL_KEYCLOAK_CLIENT_SECRET").ok();
    let oauth = match (keycloak_url, client_id) {
        (Some(url), Some(id)) => Some(OAuthConfig::keycloak(url, id, client_secret)),
        _ => {
            println!("Keycloak not configured — attempting auto-provision…");
            match auto_provision_keycloak().await {
                Ok((url, id)) => {
                    println!("keycloak ready at {url}");
                    Some(OAuthConfig::keycloak(url, id, None))
                }
                Err(e) => {
                    println!("note: could not auto-provision Keycloak ({e}) — login is disabled");
                    None
                }
            }
        }
    };
    let auth = AuthState::new(oauth, approved);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    ccosel_server::serve(addr, jail, web, auth).await
}

async fn auto_provision_keycloak() -> anyhow::Result<(String, String)> {
    ensure_docker()?;
    ensure_keycloak_running().await?;
    let base = "http://localhost:8080";
    let admin_token = keycloak_admin_token(base).await?;
    create_realm(base, &admin_token).await?;
    create_client(base, &admin_token).await?;
    let url = format!("{base}/realms/{KEYCLOAK_REALM}");
    write_dotenv(&url, KEYCLOAK_CLIENT_ID)?;
    Ok((url, KEYCLOAK_CLIENT_ID.to_string()))
}

fn ensure_docker() -> anyhow::Result<()> {
    let status = std::process::Command::new("docker")
        .args(["--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => anyhow::bail!(
            "docker is required for auto-provisioning — install it or set CCOSEL_KEYCLOAK_URL manually"
        ),
    }
}

async fn ensure_keycloak_running() -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;
    if client.get("http://localhost:8080").send().await.is_ok() {
        return Ok(());
    }

    // Remove a leftover stopped container with the same name.
    let _ = std::process::Command::new("docker")
        .args(["rm", "-f", KEYCLOAK_CONTAINER_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    println!("starting keycloak container…");
    let status = std::process::Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            KEYCLOAK_CONTAINER_NAME,
            "-p",
            "8080:8080",
            "-e",
            &format!("KEYCLOAK_ADMIN={KEYCLOAK_ADMIN_USER}"),
            "-e",
            &format!("KEYCLOAK_ADMIN_PASSWORD={KEYCLOAK_ADMIN_PASS}"),
            "quay.io/keycloak/keycloak",
            "start-dev",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()?;
    if !status.success() {
        anyhow::bail!("failed to start keycloak container (is docker running?)");
    }

    println!("waiting for keycloak to start…");
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if client.get("http://localhost:8080").send().await.is_ok() {
            return Ok(());
        }
    }
    anyhow::bail!("keycloak did not start within 60 seconds")
}

async fn keycloak_admin_token(base: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{base}/realms/master/protocol/openid-connect/token"
        ))
        .form(&[
            ("grant_type", "password"),
            ("client_id", "admin-cli"),
            ("username", KEYCLOAK_ADMIN_USER),
            ("password", KEYCLOAK_ADMIN_PASS),
        ])
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    resp["access_token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("could not get admin token: {resp}"))
}

async fn create_realm(base: &str, token: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/admin/realms"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "realm": KEYCLOAK_REALM,
            "enabled": true,
            "registrationAllowed": false,
            "loginWithEmailAllowed": true
        }))
        .send()
        .await?;
    if resp.status().is_success() || resp.status().as_u16() == 409 {
        return Ok(());
    }
    anyhow::bail!("failed to create realm: {}", resp.text().await?)
}

async fn create_client(base: &str, token: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/admin/realms/{KEYCLOAK_REALM}/clients"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "clientId": KEYCLOAK_CLIENT_ID,
            "protocol": "openid-connect",
            "publicClient": true,
            "redirectUris": [
                "http://localhost:8777/auth/callback",
                "http://127.0.0.1:8777/auth/callback",
                "http://localhost:8777/*",
                "http://127.0.0.1:8777/*"
            ],
            "webOrigins": [
                "http://localhost:8777",
                "http://127.0.0.1:8777"
            ],
        }))
        .send()
        .await?;
    if resp.status().is_success() || resp.status().as_u16() == 409 {
        return Ok(());
    }
    anyhow::bail!("failed to create client: {}", resp.text().await?)
}

fn write_dotenv(keycloak_url: &str, client_id: &str) -> anyhow::Result<()> {
    let content = format!(
        "CCOSEL_KEYCLOAK_URL={keycloak_url}\n\
         CCOSEL_KEYCLOAK_CLIENT_ID={client_id}\n"
    );
    std::fs::write(".env", &content)?;
    println!("wrote .env with auto-provisioned Keycloak credentials");
    Ok(())
}
