//! `ccosel-server` binary.

use std::net::SocketAddr;
use std::path::PathBuf;

use ccosel_server::auth::{ApprovedUsers, AuthState, OAuthConfig};
use ccosel_server::fs_api::Jail;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
            // Missing on purpose while OAuth isn't set up yet: an empty jail is still a jail,
            // an empty allow-list is still a valid (if useless) allow-list, so this is a
            // warning rather than a reason to refuse to start.
            println!(
                "warning: could not read {} ({e}); no accounts are approved until it exists",
                approved_users_path.display()
            );
            ApprovedUsers::default()
        }
    };

    // Both unset (the default until an operator registers an OAuth app) means `/auth/login`
    // answers 503 instead of the server refusing to start — see `AuthState`'s doc comment.
    let client_id = std::env::var("CCOSEL_GITHUB_CLIENT_ID").ok();
    let client_secret = std::env::var("CCOSEL_GITHUB_CLIENT_SECRET").ok();
    let oauth = match (client_id, client_secret) {
        (Some(id), Some(secret)) => Some(OAuthConfig::github(id, secret)),
        _ => {
            println!(
                "note: CCOSEL_GITHUB_CLIENT_ID / CCOSEL_GITHUB_CLIENT_SECRET not set — login is disabled"
            );
            None
        }
    };
    let auth = AuthState::new(oauth, approved);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    ccosel_server::serve(addr, jail, web, auth).await
}
