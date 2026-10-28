mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

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
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(response.headers().get("etag").is_none());
    }
}
