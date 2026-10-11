//! The browser build's web server (docs/web-port/server.md). It serves the package
//! `tools/web-port/build.sh` writes, precompressed and revalidated, and, only when the operator
//! asks, their own converted installation to their own browser on this machine.
//!
//! The installation mount is the one place game data leaves the disk, so it is fenced in:
//! - It is off unless an installation is given.
//! - The server must then listen on a loopback address (`main`).
//! - Every request needs the operator's token.
//! - A request is refused unless it names this machine as its host and passes through no proxy:
//!   a tunnel or reverse proxy in front of the server cannot carry it elsewhere.
#![forbid(unsafe_code)]
use axum::{
    Router,
    body::Body,
    extract::{Path as UrlPath, State},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tower::ServiceExt;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

/// The operator's own installation, served to their own browser (`#mount=<token>` on the page).
pub struct Mount {
    /// The folder the operator chose: the package folder, its `data` folder or an installation.
    /// The page finds the installation in it, as it does with a folder picked on the page.
    pub root: PathBuf,
    pub token: String,
}

/// Shortest token accepted from the operator: 128 bits of hex.
pub const MIN_TOKEN: usize = 32;

/// What can serve the mount: nothing else is checked against these.
pub fn mount_allowed(listen: SocketAddr) -> Result<(), String> {
    if listen.ip().is_loopback() {
        Ok(())
    } else {
        Err(format!(
            "The installation mount serves this machine only: listen on 127.0.0.1 or [::1], not {}",
            listen.ip()
        ))
    }
}

/// A fresh random token: 32 bytes as hex.
pub fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("No randomness for a token: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The server: the package in `dist` at `/`, and with `mount`, the installation under `/mount/`.
pub fn router(dist: &Path, mount: Option<Mount>) -> Router {
    let package = Router::new()
        .fallback_service(
            ServeDir::new(dist)
                .precompressed_br()
                .precompressed_gzip()
                .append_index_html_on_directories(true),
        )
        // Revalidate every load (cheap: Last-Modified, 304), so a rebuilt package is never
        // mixed with a cached engine of another commit.
        .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-cache")));
    let router = match mount {
        Some(mount) => Router::new()
            .route("/mount/files.json", get(files))
            .route("/mount/file/{*path}", get(file))
            .with_state(Arc::new(mount))
            .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
            .merge(package),
        None => package,
    };
    router.layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
}

/// Headers a proxy or tunnel adds: a mount request carrying one did not come from this machine's
/// own browser.
const PROXIED: [&str; 6] = ["forwarded", "x-forwarded-for", "x-forwarded-host", "x-real-ip", "cf-connecting-ip", "true-client-ip"];

/// A refused request: its status and why.
type Refusal = (StatusCode, &'static str);

/// The mount's fence: this machine's host name, no proxy, the operator's token.
fn admit(headers: &HeaderMap, mount: &Mount) -> Result<(), Refusal> {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default();
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    if !matches!(name.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1") {
        return Err((StatusCode::FORBIDDEN, "The installation mount answers only to this machine (localhost)."));
    }
    if PROXIED.iter().any(|name| headers.contains_key(*name)) {
        return Err((StatusCode::FORBIDDEN, "The installation mount is not served through a proxy."));
    }
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or_default();
    // blake3::Hash compares in constant time.
    if given.is_empty() || blake3::hash(given.as_bytes()) != blake3::hash(mount.token.as_bytes()) {
        return Err((StatusCode::UNAUTHORIZED, "Wrong or missing installation token."));
    }
    Ok(())
}

/// Every regular file under the mount, `/`-separated, with its size: what the page's folder
/// picker would list. Links are not followed, so nothing outside the folder is listed.
fn list(root: &Path) -> std::io::Result<Vec<(String, u64)>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, u64)>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let Ok(name) = entry.file_name().into_string() else { continue };
            let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(&entry.path(), &path, out)?;
            } else if kind.is_file() {
                out.push((path, entry.metadata()?.len()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, "", &mut out)?;
    out.sort();
    Ok(out)
}

async fn files(State(mount): State<Arc<Mount>>, headers: HeaderMap) -> Response {
    if let Err(refused) = admit(&headers, &mount) {
        return refused.into_response();
    }
    let root = mount.root.clone();
    match tokio::task::spawn_blocking(move || list(&root)).await {
        Ok(Ok(files)) => {
            let name = mount.root.file_name().unwrap_or_default().to_string_lossy();
            let body = serde_json::json!({ "name": name, "files": files });
            ([(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Could not list the installation.").into_response(),
    }
}

/// `relative` inside `root`, if it names a regular file there: plain path segments only, and the
/// resolved file (links followed) still under `root`.
fn inside(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty() || !relative.components().all(|c| matches!(c, Component::Normal(_))) || relative.to_str()?.contains('\\') {
        return None;
    }
    let path = root.join(relative).canonicalize().ok()?;
    (path.starts_with(root.canonicalize().ok()?) && path.is_file()).then_some(path)
}

async fn file(State(mount): State<Arc<Mount>>, UrlPath(relative): UrlPath<String>, request: Request<Body>) -> Response {
    if let Err(refused) = admit(request.headers(), &mount) {
        return refused.into_response();
    }
    match inside(&mount.root, &relative) {
        Some(path) => match ServeFile::new(path).oneshot(request).await {
            Ok(response) => response.into_response(),
            Err(error) => match error {},
        },
        None => (StatusCode::NOT_FOUND, "No such file in the installation.").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_listeners_may_serve_the_mount() {
        assert!(mount_allowed("127.0.0.1:8080".parse().unwrap()).is_ok());
        assert!(mount_allowed("[::1]:8080".parse().unwrap()).is_ok());
        assert!(mount_allowed("0.0.0.0:8080".parse().unwrap()).is_err());
        assert!(mount_allowed("192.168.1.20:8080".parse().unwrap()).is_err());
    }

    #[test]
    fn tokens_are_long_random_hex() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(a.len(), 64);
        assert!(a.len() >= MIN_TOKEN && a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn paths_stay_inside_the_mount() {
        let root = std::env::temp_dir().join(format!("skate-web-server-inside-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(root.join("data/a.json"), b"{}").unwrap();
        std::fs::write(root.parent().unwrap().join(format!("skate-web-server-outside-{}", std::process::id())), b"secret").unwrap();
        assert!(inside(&root, "data/a.json").is_some());
        for bad in ["", "data", "../x", "data/../../etc/passwd", "/etc/passwd", "data\\a.json", "./data/a.json", "data/missing"] {
            assert!(inside(&root, bad).is_none(), "{bad:?}");
        }
        assert!(inside(&root, &format!("../skate-web-server-outside-{}", std::process::id())).is_none());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(root.parent().unwrap().join(format!("skate-web-server-outside-{}", std::process::id())));
    }
}
