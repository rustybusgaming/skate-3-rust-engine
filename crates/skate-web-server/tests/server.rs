//! The server in-process: the package's headers and precompression, and the installation mount's
//! fence (host, proxy, token, paths).
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use skate_web_server::{Mount, router};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("skate-web-server-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(dir: &Path, path: &str, bytes: &[u8]) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn package(test: &str) -> PathBuf {
    let dist = folder(&format!("{test}-dist"));
    put(&dist, "index.html", b"<!doctype html>page");
    put(&dist, "skate3rust_bg.wasm", b"\0asm engine");
    put(&dist, "skate3rust_bg.wasm.gz", b"gzipped engine");
    put(&dist, "skate3rust_bg.wasm.br", b"brotli engine");
    dist
}

fn installation(test: &str) -> PathBuf {
    let root = folder(&format!("{test}-installation"));
    put(&root, "data/installation.json", br#"{"version":1}"#);
    put(&root, "data/installations/x/assets/private/game.json", b"{}");
    root
}

struct Sent {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

async fn send(app: &Router, request: Request<Body>) -> Sent {
    let response = app.clone().oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec();
    Sent { status, headers, body }
}

fn get(path: &str) -> axum::http::request::Builder {
    Request::builder().uri(path).header(header::HOST, "localhost:8080")
}

fn mounted(path: &str) -> axum::http::request::Builder {
    get(path).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
}

#[tokio::test]
async fn the_package_is_served_revalidated_and_precompressed() {
    let dist = package("package");
    let app = router(&dist, None);
    let page = send(&app, get("/").body(Body::empty()).unwrap()).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.body, b"<!doctype html>page");
    assert_eq!(page.headers[header::CACHE_CONTROL], "no-cache");
    assert_eq!(page.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(page.headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));

    let plain = send(&app, get("/skate3rust_bg.wasm").body(Body::empty()).unwrap()).await;
    assert_eq!((plain.status, plain.body.as_slice()), (StatusCode::OK, &b"\0asm engine"[..]));
    assert_eq!(plain.headers[header::CONTENT_TYPE], "application/wasm");
    // The smallest copy the browser accepts: brotli, else gzip.
    let brotli = send(&app, get("/skate3rust_bg.wasm").header(header::ACCEPT_ENCODING, "gzip, deflate, br").body(Body::empty()).unwrap()).await;
    assert_eq!(brotli.body, b"brotli engine");
    assert_eq!(brotli.headers[header::CONTENT_ENCODING], "br");
    assert_eq!(brotli.headers[header::CONTENT_TYPE], "application/wasm");
    let gzipped = send(&app, get("/skate3rust_bg.wasm").header(header::ACCEPT_ENCODING, "gzip").body(Body::empty()).unwrap()).await;
    assert_eq!(gzipped.body, b"gzipped engine");
    assert_eq!(gzipped.headers[header::CONTENT_ENCODING], "gzip");

    let modified = plain.headers[header::LAST_MODIFIED].clone();
    let again = send(&app, get("/skate3rust_bg.wasm").header(header::IF_MODIFIED_SINCE, modified).body(Body::empty()).unwrap()).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);

    assert_eq!(send(&app, get("/missing.js").body(Body::empty()).unwrap()).await.status, StatusCode::NOT_FOUND);
    let escape = send(&app, get("/..%2F..%2Fetc%2Fpasswd").body(Body::empty()).unwrap()).await;
    assert_ne!(escape.status, StatusCode::OK);
    // Without an installation there is no mount at all.
    assert_eq!(send(&app, mounted("/mount/files.json").body(Body::empty()).unwrap()).await.status, StatusCode::NOT_FOUND);
    let _ = std::fs::remove_dir_all(dist);
}

#[tokio::test]
async fn the_mount_lists_and_serves_the_installation_to_this_machine() {
    let (dist, root) = (package("serves"), installation("serves"));
    let app = router(&dist, Some(Mount { root: root.clone(), token: TOKEN.into() }));
    let list = send(&app, mounted("/mount/files.json").body(Body::empty()).unwrap()).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.headers[header::CACHE_CONTROL], "no-store");
    let json: serde_json::Value = serde_json::from_slice(&list.body).unwrap();
    assert_eq!(
        json["files"],
        serde_json::json!([["data/installation.json", 13], ["data/installations/x/assets/private/game.json", 2]])
    );
    assert_eq!(json["name"], root.file_name().unwrap().to_string_lossy().as_ref());

    let file = send(&app, mounted("/mount/file/data/installations/x/assets/private/game.json").body(Body::empty()).unwrap()).await;
    assert_eq!((file.status, file.body.as_slice()), (StatusCode::OK, &b"{}"[..]));
    assert_eq!(file.headers[header::CACHE_CONTROL], "no-store");
    // Percent-encoded names (spaces) arrive decoded.
    put(&root, "texture-packs/My Pack/a b.png", b"png");
    let spaced = send(&app, mounted("/mount/file/texture-packs/My%20Pack/a%20b.png").body(Body::empty()).unwrap()).await;
    assert_eq!((spaced.status, spaced.body.as_slice()), (StatusCode::OK, &b"png"[..]));
    // The package is still served beside it.
    assert_eq!(send(&app, get("/").body(Body::empty()).unwrap()).await.status, StatusCode::OK);
    let _ = std::fs::remove_dir_all(dist);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn the_mount_refuses_other_hosts_proxies_bad_tokens_and_escapes() {
    let (dist, root) = (package("refuses"), installation("refuses"));
    let outside = root.parent().unwrap().join(format!("skate-web-server-secret-{}", std::process::id()));
    std::fs::write(&outside, b"secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("data/link")).unwrap();
    let app = router(&dist, Some(Mount { root: root.clone(), token: TOKEN.into() }));
    let status = |request: Request<Body>| {
        let app = app.clone();
        async move { send(&app, request).await.status }
    };
    let path = "/mount/files.json";
    // Another host name: DNS rebinding, or a tunnel that forwards the public name.
    let other_host = Request::builder().uri(path).header(header::HOST, "skate.example.com").header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    assert_eq!(status(other_host.body(Body::empty()).unwrap()).await, StatusCode::FORBIDDEN);
    for proxy in ["x-forwarded-for", "forwarded", "cf-connecting-ip"] {
        assert_eq!(status(mounted(path).header(proxy, "203.0.113.9").body(Body::empty()).unwrap()).await, StatusCode::FORBIDDEN, "{proxy}");
    }
    assert_eq!(status(get(path).body(Body::empty()).unwrap()).await, StatusCode::UNAUTHORIZED);
    assert_eq!(status(get(path).header(header::AUTHORIZATION, "Bearer wrong").body(Body::empty()).unwrap()).await, StatusCode::UNAUTHORIZED);
    assert_eq!(status(get(path).header(header::AUTHORIZATION, TOKEN).body(Body::empty()).unwrap()).await, StatusCode::UNAUTHORIZED);
    for host in ["127.0.0.1:8080", "[::1]:8080", "LOCALHOST"] {
        let request = Request::builder().uri(path).header(header::HOST, host).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
        assert_eq!(status(request.body(Body::empty()).unwrap()).await, StatusCode::OK, "{host}");
    }
    for escape in ["/mount/file/..%2F..%2Fetc%2Fpasswd", "/mount/file/data/..%2F..%2Fsecret", "/mount/file/data", "/mount/file/data/link"] {
        let sent = send(&app, mounted(escape).body(Body::empty()).unwrap()).await;
        assert_ne!(sent.status, StatusCode::OK, "{escape}");
        assert!(!sent.body.windows(6).any(|w| w == b"secret"), "{escape}");
    }
    // Links are not listed either.
    let list = send(&app, mounted(path).body(Body::empty()).unwrap()).await;
    assert!(!String::from_utf8_lossy(&list.body).contains("link"));
    let _ = std::fs::remove_file(outside);
    let _ = std::fs::remove_dir_all(dist);
    let _ = std::fs::remove_dir_all(root);
}
