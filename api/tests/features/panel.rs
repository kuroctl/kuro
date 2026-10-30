//! Panel compilé servi par l'API (ADR-0016) : routes du navigateur, cache, en-têtes de sécurité.

use axum::http::StatusCode;
use tempfile::TempDir;

use crate::support::http::get_with_panel;

/// Panel compilé minimal, et un secret à côté de son dossier.
fn panel() -> (TempDir, std::path::PathBuf) {
    let root = TempDir::new().unwrap();
    let dir = root.path().join("panel");
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><div id=app></div>").unwrap();
    std::fs::write(dir.join("assets/app-3f2a.js"), "console.log('kuro')").unwrap();
    std::fs::write(root.path().join("secret.txt"), "do not serve").unwrap();
    (root, dir)
}

#[tokio::test]
async fn the_panel_is_served_with_strict_headers() {
    let (_root, dir) = panel();

    let response = get_with_panel(Some(&dir), "/").await;

    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("id=app"));
    let header = |name: &str| {
        response
            .headers
            .get(name)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    };
    let csp = header("content-security-policy");
    assert!(csp.contains("default-src 'self'"), "{csp}");
    assert!(csp.contains("script-src 'self';"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    // Carte du monde : seul OpenFreeMap est joignable, et le worker vient du panel.
    assert!(
        csp.contains("connect-src 'self' https://tiles.openfreemap.org;"),
        "{csp}"
    );
    assert!(csp.contains("worker-src 'self';"), "{csp}");
    assert_eq!(header("x-content-type-options"), "nosniff");
    assert_eq!(header("referrer-policy"), "no-referrer");
    assert_eq!(header("cache-control"), "no-cache");
}

#[tokio::test]
async fn browser_routes_get_the_panel() {
    let (_root, dir) = panel();

    let response = get_with_panel(Some(&dir), "/servers/0192f3a4/console").await;

    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("id=app"));
}

#[tokio::test]
async fn fingerprinted_assets_are_cached_for_good() {
    let (_root, dir) = panel();

    let response = get_with_panel(Some(&dir), "/assets/app-3f2a.js").await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, "console.log('kuro')");
    assert_eq!(
        response.headers.get("cache-control").unwrap(),
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn an_unknown_api_route_stays_a_json_error() {
    let (_root, dir) = panel();

    let response = get_with_panel(Some(&dir), "/api/admin/v1/nope").await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.json["code"], "not_found");
}

#[tokio::test]
async fn nothing_outside_the_panel_directory_is_served() {
    let (_root, dir) = panel();

    for path in [
        "/../secret.txt",
        "/%2e%2e/secret.txt",
        "/assets/..%2f..%2fsecret.txt",
    ] {
        let response = get_with_panel(Some(&dir), path).await;
        assert!(!response.body.contains("do not serve"), "{path}");
    }
}

#[tokio::test]
async fn without_a_panel_directory_only_the_api_answers() {
    let response = get_with_panel(None, "/servers").await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.json["code"], "not_found");
}
