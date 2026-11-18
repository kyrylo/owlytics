mod common;

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    response::Response,
};
use owlytics::{analytics, config::Config, db::Database};
use tower::ServiceExt;

const UA: &str = "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36";

async fn setup(config: Config) -> (Database, Router) {
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
    let app = owlytics::router(db.clone(), config);
    (db, app)
}

async fn send(
    app: &Router,
    endpoint: &str,
    body: serde_json::Value,
    ua: &str,
    peer: &str,
    forwarded: Option<&str>,
) -> Response {
    let mut request = Request::builder()
        .method("POST")
        .uri(endpoint)
        .header("origin", "https://example.com")
        .header("user-agent", ua)
        .header("content-type", "text/plain")
        .extension(ConnectInfo(peer.parse::<std::net::SocketAddr>().unwrap()));
    if let Some(ip) = forwarded {
        request = request.header("x-forwarded-for", ip);
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
    assert!(response.headers().get("retry-after").is_none());
    assert!(response.headers().get("set-cookie").is_none());
    response
}

fn page(id: usize, referrer: &str) -> serde_json::Value {
    serde_json::json!({"domain":"example.com", "path":"/new", "id":format!("{id:032x}"), "referrer":referrer})
}

fn engagement() -> serde_json::Value {
    serde_json::json!({"domain":"example.com", "id":format!("{:032x}", 1), "time_ms":5000, "scroll_depth":75})
}

fn outbound() -> serde_json::Value {
    serde_json::json!({"domain":"example.com", "id":format!("{:032x}", 3), "page_id":format!("{:032x}", 1), "url":"https://other.example/"})
}

async fn assert_all_collectors(
    config: Config,
    ua: &str,
    peer: &str,
    forwarded: Option<&str>,
    reason: Option<&str>,
) {
    let (db, app) = setup(config).await;
    for (endpoint, body) in [
        ("/api/event", page(2, "")),
        ("/api/engagement", engagement()),
        ("/api/outbound", outbound()),
    ] {
        let response = send(&app, endpoint, body, ua, peer, forwarded).await;
        assert_eq!(
            response
                .headers()
                .get("x-owlytics-dropped")
                .map(|v| v.to_str().unwrap()),
            reason,
            "{endpoint} {peer} {ua}"
        );
    }
    let counts = db
        .read(|c| {
            Ok((
                c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?,
                c.query_row("SELECT COUNT(*) FROM page_engagement", [], |r| {
                    r.get::<_, i64>(0)
                })?,
                c.query_row("SELECT COUNT(*) FROM outbound_clicks", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(
        counts,
        if reason.is_some() {
            (1, 0, 0)
        } else {
            (2, 1, 1)
        }
    );
}

#[tokio::test]
async fn bots_and_hosting_ips_are_dropped_before_any_collector_writes() {
    for ua in [
        "",
        "GPTBot/1.0",
        "Claude-User/1.0",
        "Go-http-client/1.1",
        "HeadlessChrome/130.0",
    ] {
        assert_all_collectors(
            common::config(),
            ua,
            "203.0.113.1:1234",
            None,
            Some("user-agent"),
        )
        .await;
    }
    for peer in [
        "165.227.0.1:1234",
        "[2604:a880::1]:1234",
        "[::ffff:165.227.0.1]:1234",
    ] {
        assert_all_collectors(common::config(), UA, peer, None, Some("datacenter")).await;
    }
    assert_all_collectors(common::config(), UA, "203.0.113.1:1234", None, None).await;
}

#[tokio::test]
async fn ip_exceptions_do_not_disable_other_bot_filters() {
    for enabled in [true, false] {
        let mut config = common::config();
        config.filter_datacenters = enabled;
        if enabled {
            config
                .bot_allowed_ips
                .push("165.227.0.1/32".parse().unwrap());
        }
        assert_all_collectors(config.clone(), UA, "165.227.0.1:1234", None, None).await;
        assert_all_collectors(
            config.clone(),
            "Googlebot/2.1",
            "165.227.0.1:1234",
            None,
            Some("user-agent"),
        )
        .await;
        if enabled {
            assert_all_collectors(config, UA, "165.227.0.2:1234", None, Some("datacenter")).await;
        }
    }
    let mut config = common::config();
    config
        .bot_allowed_ips
        .push("2604:a880::1/128".parse().unwrap());
    assert_all_collectors(config, UA, "[2604:a880::1]:1234", None, None).await;
}

#[tokio::test]
async fn forwarding_headers_only_work_through_trusted_proxies() {
    // A crawler on a hosting network cannot forge a residential address.
    assert_all_collectors(
        common::config(),
        UA,
        "165.227.0.1:1234",
        Some("203.0.113.1"),
        Some("datacenter"),
    )
    .await;
    let mut config = common::config();
    config.trusted_proxies.push("10.0.0.0/8".parse().unwrap());
    for forwarded in [
        "165.227.0.1",
        "203.0.113.1, 165.227.0.1, 10.0.0.2",
        "::ffff:165.227.0.1",
    ] {
        assert_all_collectors(
            config.clone(),
            UA,
            "10.0.0.1:1234",
            Some(forwarded),
            Some("datacenter"),
        )
        .await;
    }
    // A hosting-based reverse proxy must not cause real visitors to be dropped.
    config
        .trusted_proxies
        .push("165.227.0.1/32".parse().unwrap());
    assert_all_collectors(config, UA, "165.227.0.1:1234", Some("203.0.113.1"), None).await;
}

#[tokio::test]
async fn spam_referrers_are_dropped_but_people_referred_by_ai_are_kept() {
    let (db, app) = setup(common::config()).await;
    for (id, referrer, dropped) in [
        (2, "https://sub.SEMALT.com./x", true),
        (3, "https://notsemalt.com/", false),
        (4, "https://chatgpt.com/", false),
        (5, "https://claude.ai/", false),
        (6, "https://example.org/?x=semalt.com", false),
    ] {
        let response = send(
            &app,
            "/api/event",
            page(id, referrer),
            UA,
            "203.0.113.1:1234",
            None,
        )
        .await;
        assert_eq!(
            response.headers().get("x-owlytics-dropped").is_some(),
            dropped
        );
    }
    let count = db
        .read(|c| c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0)))
        .await
        .unwrap();
    assert_eq!(count, 5);
}

#[tokio::test]
async fn high_rate_pageviews_and_their_followup_events_stop_counting() {
    let (db, app) = setup(common::config()).await;
    // A plausible browser UA on a non-hosting address still faces burst limits.
    for id in 2..=122 {
        let response = send(
            &app,
            "/api/event",
            page(id, ""),
            UA,
            "203.0.113.1:1234",
            None,
        )
        .await;
        assert_eq!(
            response.headers().get("x-owlytics-dropped").is_some(),
            id == 122
        );
    }
    for (endpoint, body) in [
        ("/api/engagement", engagement()),
        ("/api/outbound", outbound()),
    ] {
        let response = send(&app, endpoint, body, UA, "203.0.113.1:1234", None).await;
        assert_eq!(response.headers()["x-owlytics-dropped"], "rate-limit");
    }
    let counts = db
        .read(|c| {
            Ok((
                c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?,
                c.query_row("SELECT COALESCE(SUM(pageviews),0) FROM sessions", [], |r| {
                    r.get::<_, i64>(0)
                })?,
                c.query_row("SELECT COUNT(*) FROM page_engagement", [], |r| {
                    r.get::<_, i64>(0)
                })?,
                c.query_row("SELECT COUNT(*) FROM outbound_clicks", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(counts, (121, 121, 0, 0));
}
