//! `ccosel-server` binary.

use std::net::SocketAddr;
use std::path::PathBuf;

use ccosel_server::auth::{AuthState, OAuthConfig};
use ccosel_server::fs_api::Jail;
use ccosel_server::idp::IdpProxy;
use ccosel_server::keycloak;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let mut args = std::env::args().skip(1);
    let mut root = PathBuf::from("data/shared");
    let mut web = PathBuf::from("web");
    let mut port = 8777u16;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = PathBuf::from(args.next().unwrap_or_default()),
            "--web" => web = PathBuf::from(args.next().unwrap_or_default()),
            "--port" => port = args.next().unwrap_or_default().parse().unwrap_or(8777),
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    std::fs::create_dir_all(&root)?;
    let jail = Jail::new(&root)?;
    println!("serving files from {}", jail.root().display());

    let public_url = std::env::var("CCOSEL_PUBLIC_URL")
        .ok()
        .filter(|u| !u.is_empty());
    if let Some(url) = &public_url
        && !(url.starts_with("https://") || url.starts_with("http://"))
    {
        anyhow::bail!("CCOSEL_PUBLIC_URL must start with https:// or http://, got {url}");
    }

    let keycloak_url = std::env::var("CCOSEL_KEYCLOAK_URL")
        .ok()
        .filter(|u| !keycloak::is_legacy_url(u));
    let client_id = std::env::var("CCOSEL_KEYCLOAK_CLIENT_ID").ok();
    let client_secret = std::env::var("CCOSEL_KEYCLOAK_CLIENT_SECRET").ok();

    let auth = match (keycloak_url, client_id) {
        // A Keycloak of your own: browsers go to it directly, at whatever address you gave.
        (Some(url), Some(id)) => {
            AuthState::new(Some(OAuthConfig::keycloak(url, id, client_secret)))
        }
        _ => match keycloak::provision(public_url.as_deref(), port).await {
            Ok(()) => {
                println!(
                    "Keycloak admin console (this machine only): {} — user admin, password in {}",
                    keycloak::admin_console_url(),
                    keycloak::ADMIN_PASSWORD_FILE
                );
                AuthState::new(Some(OAuthConfig::proxied_keycloak(
                    keycloak::realm_url(),
                    keycloak::CLIENT_ID.to_owned(),
                )))
                .with_idp_proxy(IdpProxy::new(
                    keycloak::upstream(),
                    keycloak::REALM.to_owned(),
                ))
            }
            Err(e) => {
                println!("note: could not start Keycloak ({e:#}) — login is disabled");
                AuthState::default()
            }
        },
    }
    .with_public_url(public_url);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    ccosel_server::serve(addr, jail, web, auth).await
}
