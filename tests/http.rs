mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn gzip_preserves_html_assets_and_cache_policy() {
    use std::io::Read;
    let app = owlytics::router(
        owlytics::db::Database::open(":memory:").unwrap(),
        common::config(),
    );
    for path in [
        "/assets/app.css",
        "/assets/logo.svg",
        "/assets/country-shapes.svg",
        "/script.js",
        "/login",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("accept-encoding", "gzip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-encoding"], "gzip");
        assert_eq!(response.headers()["vary"], "accept-encoding");
        assert!(response.headers().get("content-length").is_none());
        let cache = response.headers()["cache-control"].clone();
        let compressed = response.into_body().collect().await.unwrap().to_bytes();
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(compressed.as_ref())
            .read_to_end(&mut decoded)
            .unwrap();
        assert!(compressed.len() < decoded.len());
        let plain = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("accept-encoding", "gzip;q=0, identity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(plain.headers().get("content-encoding").is_none());
        assert_eq!(plain.headers()["cache-control"], cache);
        let plain = plain.into_body().collect().await.unwrap().to_bytes();
        if path == "/login" {
            assert!(String::from_utf8(decoded).unwrap().contains("Sign in"));
            assert_eq!(cache, "no-store, no-transform");
        } else {
            assert_eq!(decoded, plain);
        }
    }
}

#[tokio::test]
async fn healthcheck_runs_a_real_database_query() {
    let app = owlytics::router(
        owlytics::db::Database::open(":memory:").unwrap(),
        common::config(),
    );
    let response = app
        .oneshot(Request::builder().uri("/up").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "ok"
    );
}

#[tokio::test]
async fn public_assets_revalidate_without_caching_private_pages() {
    let app = owlytics::router(
        owlytics::db::Database::open(":memory:").unwrap(),
        common::config(),
    );
    for path in [
        "/script.js",
        "/assets/app.css",
        "/assets/dashboard.js",
        "/assets/owl.svg",
        "/assets/logo.svg",
        "/assets/country-shapes.svg",
        "/assets/favicon.svg",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "public, no-cache");
        let etag = response.headers()["etag"].to_str().unwrap().to_owned();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(bytes.len() > 100);
        assert_eq!(
            etag,
            format!(
                "\"{}\"",
                owlytics::auth::digest(std::str::from_utf8(&bytes).unwrap())
            )
        );
        if path == "/assets/country-shapes.svg" {
            let versioned = format!("/assets/country-shapes.{}.svg", &etag[1..17]);
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(versioned)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()["cache-control"],
                "public, max-age=31536000, immutable"
            );
            assert_eq!(
                response.into_body().collect().await.unwrap().to_bytes(),
                bytes
            );
        }
        for condition in [
            etag.clone(),
            format!("W/{etag}"),
            format!("\"old\", {etag}"),
            "*".into(),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("if-none-match", condition)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
            assert_eq!(response.headers()["etag"], etag);
            assert_eq!(response.headers()["cache-control"], "public, no-cache");
            assert!(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty()
            );
        }
        let changed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("if-none-match", "\"old-version\"")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(changed.status(), StatusCode::OK);
        assert_eq!(
            changed.into_body().collect().await.unwrap().to_bytes(),
            bytes
        );
        let head = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()["etag"], etag);
        assert!(
            head.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
    }
    for path in ["/login", "/", "/sites/new", "/up"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("if-none-match", "*")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            response.headers()["cache-control"],
            if path == "/login" {
                "no-store, no-transform"
            } else {
                "no-store"
            }
        );
        assert!(response.headers().get("etag").is_none());
    }
}
