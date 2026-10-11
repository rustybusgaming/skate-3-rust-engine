//! `skate-web-server [--dist DIR] [--listen ADDRESS:PORT] [--installation DIR]`
//! (docs/web-port/server.md).
use skate_web_server::{MIN_TOKEN, Mount, mount_allowed, new_token, router};
use std::{net::SocketAddr, path::PathBuf};

const USAGE: &str = "Usage: skate-web-server [--dist DIR] [--listen ADDRESS:PORT] [--installation DIR]
  --dist DIR             the browser package (default target/web/dist, from tools/web-port/build.sh)
  --listen ADDRESS:PORT  where to listen (default 127.0.0.1:8080)
  --installation DIR     also serve your own converted installation to your own browser on this
                         machine: the package folder, its data folder or data/installations/<id>.
                         Needs a loopback --listen. The token comes from SKATE_WEB_TOKEN, else a
                         new one is made and printed with the address to open.";

struct Options {
    dist: PathBuf,
    listen: SocketAddr,
    installation: Option<PathBuf>,
}

fn options() -> Result<Options, String> {
    let mut options = Options { dist: "target/web/dist".into(), listen: ([127, 0, 0, 1], 8080).into(), installation: None };
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{} needs a value\n{USAGE}", arg.to_string_lossy()));
        match arg.to_str() {
            Some("--dist") => options.dist = value()?.into(),
            Some("--listen") => {
                options.listen = value()?.to_string_lossy().parse().map_err(|_| format!("--listen takes ADDRESS:PORT\n{USAGE}"))?
            }
            Some("--installation") => options.installation = Some(value()?.into()),
            Some("--help" | "-h") => return Err(USAGE.into()),
            _ => return Err(format!("Unknown argument {arg:?}\n{USAGE}")),
        }
    }
    Ok(options)
}

fn mount(options: &Options) -> Result<Option<Mount>, String> {
    let Some(root) = &options.installation else { return Ok(None) };
    mount_allowed(options.listen)?;
    if !root.is_dir() {
        return Err(format!("--installation {}: not a folder", root.display()));
    }
    let token = match std::env::var("SKATE_WEB_TOKEN") {
        Ok(token) if token.len() >= MIN_TOKEN => token,
        Ok(_) => return Err(format!("SKATE_WEB_TOKEN must be at least {MIN_TOKEN} characters")),
        Err(_) => new_token()?,
    };
    Ok(Some(Mount { root: root.clone(), token }))
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let options = match options() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::from(2);
        }
    };
    if !options.dist.join("index.html").is_file() {
        eprintln!("{}: no browser package here. Build it with tools/web-port/build.sh, or pass --dist.", options.dist.display());
        return std::process::ExitCode::FAILURE;
    }
    let mount = match mount(&options) {
        Ok(mount) => mount,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let listener = match tokio::net::TcpListener::bind(options.listen).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Cannot listen on {}: {error}", options.listen);
            return std::process::ExitCode::FAILURE;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(options.listen.port());
    println!("Serving {} at http://{}/", options.dist.display(), listener.local_addr().unwrap_or(options.listen));
    if let Some(mount) = &mount {
        // The token travels in the address's fragment, which the browser never sends to a server.
        println!("Your installation {} (this machine only): http://localhost:{port}/#mount={}", mount.root.display(), mount.token);
    }
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    match axum::serve(listener, router(&options.dist, mount)).with_graceful_shutdown(shutdown).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
