//! `ccosel-server` binary.

use std::net::SocketAddr;
use std::path::PathBuf;

use ccosel_server::fs_api::Jail;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    ccosel_server::serve(addr, jail, web).await
}
