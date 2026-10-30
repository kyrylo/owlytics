mod common;

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use owlytics::{analytics, db::Database};
use tower::ServiceExt;

const UA: &str = "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36";

#[tokio::test]
async fn direct_requests_use_instance_gpc_policy_for_both_collectors() {
    for configured in [None, Some(true), Some(false)] {
        let mut config = common::config();
        assert!(
            config.respect_gpc,
            "Existing installations must keep suppression"
        );
        if let Some(value) = configured {
            config.respect_gpc = value;
        }
        for (gpc, dnt, ua) in [
            (None, None, UA),
            (Some("0"), None, UA),
            (Some("1"), None, UA),
            (None, Some("1"), UA),
            (Some("1"), Some("1"), UA),
            (Some("1"), None, "HeadlessChrome/130.0.0.0"),
        ] {
            // Seed a real pageview independently so engagement suppression is
            // tested even when the HTTP pageview request is itself suppressed.
            let db = Database::open(":memory:").unwrap();
            db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
                .await
                .unwrap();
            analytics::collect(
                &db,
                analytics::Event {
                    domain: "example.com".into(),
                    path: "/existing".into(),
                    referrer: String::new(),
                    id: format!("{:032x}", 1),
                    campaign: None,
                },
                "203.0.113.1".parse().unwrap(),
                UA.into(),
                || owlytics::auth::now() - 10,
            )
            .await
            .unwrap();
            let suppressed =
                (config.respect_gpc && gpc == Some("1")) || dnt == Some("1") || ua != UA;
            let app = owlytics::router(db.clone(), config.clone());
            for (path, body) in [
                (
                    "/api/event",
                    serde_json::json!({"domain":"example.com", "path":"/new", "id":format!("{:032x}", 2)}),
                ),
                (
                    "/api/engagement",
                    serde_json::json!({"domain":"example.com", "id":format!("{:032x}", 1), "time_ms":5000, "scroll_depth":75}),
                ),
            ] {
                let mut request = Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("origin", "https://example.com")
                    .header("user-agent", ua)
                    .header("content-type", "text/plain")
                    .extension(ConnectInfo(
                        "203.0.113.1:1234".parse::<std::net::SocketAddr>().unwrap(),
                    ));
                if let Some(value) = gpc {
                    request = request.header("sec-gpc", value);
                }
                if let Some(value) = dnt {
                    request = request.header("dnt", value);
                }
                let response = app
                    .clone()
                    .oneshot(request.body(Body::from(body.to_string())).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::NO_CONTENT);
                assert_eq!(
                    response.headers()["access-control-allow-origin"],
                    "https://example.com"
                );
                assert_eq!(response.headers()["cache-control"], "no-store");
            }
            let stored = db.read(|c| {
                Ok((
                    c.query_row("SELECT COUNT(*) FROM events WHERE path='/new'", [], |r| r.get::<_, i64>(0))?,
                    c.query_row("SELECT COUNT(*),COALESCE(SUM(time_ms),0),COALESCE(SUM(scroll_depth),0) FROM page_engagement", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?,
                ))
            }).await.unwrap();
            assert_eq!(
                stored,
                if suppressed {
                    (0, (0, 0, 0))
                } else {
                    (1, (1, 5000, 75))
                },
                "configured={configured:?}, GPC={gpc:?}, DNT={dnt:?}, UA={ua}"
            );
        }
    }
}

#[tokio::test]
async fn visitor_payloads_cannot_override_instance_gpc_policy() {
    for respect_gpc in [true, false] {
        let mut config = common::config();
        config.respect_gpc = respect_gpc;
        let db = Database::open(":memory:").unwrap();
        let app = owlytics::router(db, config);
        for (path, mut body) in [
            (
                "/api/event",
                serde_json::json!({"domain":"example.com", "path":"/", "id":format!("{:032x}", 1)}),
            ),
            (
                "/api/engagement",
                serde_json::json!({"domain":"example.com", "id":format!("{:032x}", 1), "time_ms":5000, "scroll_depth":75}),
            ),
        ] {
            body["respect_gpc"] = serde_json::json!(!respect_gpc);
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header("origin", "https://example.com")
                        .header("sec-gpc", "1")
                        .header("user-agent", UA)
                        .extension(ConnectInfo(
                            "203.0.113.1:1234".parse::<std::net::SocketAddr>().unwrap(),
                        ))
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        }
    }
}
