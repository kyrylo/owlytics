mod common;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use owlytics::{analytics, auth, db::Database};
use tower::ServiceExt;

const ORIGIN: &str = "https://stats.example.com";
const SITE_TABLES: &[(&str, &str)] = &[
    ("sites", "id"),
    ("site_salts", "site_id"),
    ("site_favicons", "site_id"),
    ("events", "site_id"),
    ("sessions", "site_id"),
    ("session_acquisition", "session_id"),
    ("page_engagement", "event_id"),
    ("daily_reports", "site_id"),
    ("daily_session_reports", "site_id"),
    ("daily_engagement", "site_id"),
    ("report_cross_day_dates", "site_id"),
    ("import_batches", "site_id"),
    ("import_daily", "site_id"),
    ("import_breakdowns", "site_id"),
    ("import_metrics", "site_id"),
    ("import_engagement", "site_id"),
    ("import_acquisition", "site_id"),
    ("import_groups", "site_id"),
    ("import_files", "batch_id"),
];

async fn fixture() -> (Router, Database, String, String) {
    let db = Database::open(":memory:").unwrap();
    let config = common::config();
    let token = auth::random_token().unwrap();
    let csrf = auth::random_token().unwrap();
    let token_hash = auth::digest(&token);
    let credential_hash = auth::digest(&config.password_hash);
    let saved_csrf = csrf.clone();
    db.call(move |c| {
        c.execute("INSERT INTO owner_sessions VALUES(?1,?2,?3,?4)",
            rusqlite::params![token_hash, saved_csrf, credential_hash, auth::now() + 3600])?;
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'other.example','UTC',0)", [])?;
        // Seed both sites with collected and imported data, including indirect children.
        for id in [1, 2] {
            for sql in [
                "INSERT INTO events(id,site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?1,?1,1793059200,'2026-10-27',zeroblob(16),zeroblob(16),'/','Google','US','Chrome','Desktop')",
                "INSERT INTO site_salts(site_id,day,secret,expires_at) VALUES(?1,'',zeroblob(32),1793145600)",
                "INSERT INTO site_favicons VALUES(?1,'image/png',X'00',1793059200)",
                "INSERT INTO session_acquisition VALUES(?1,'Organic Search','','','','','',NULL)",
                "INSERT INTO page_engagement VALUES(?1,1000,50)",
                "INSERT INTO daily_engagement VALUES(?1,'2026-10-27','/',1000,1,50)",
                "INSERT INTO report_cross_day_dates VALUES(?1,'2026-10-27')",
                "INSERT INTO import_batches VALUES(?1,?1,'2025-01-01','2025-01-01',0)",
                "INSERT INTO import_daily VALUES(?1,'2025-01-01',?1,2,3)",
                "INSERT INTO import_breakdowns VALUES(?1,'2025-01-01','page','/',?1,3)",
                "INSERT INTO import_metrics VALUES(?1,'2025-01-01','page','/',?1,2,2,1,10,1)",
                "INSERT INTO import_engagement VALUES(?1,'2025-01-01','/',?1,1000,2,50,2)",
                "INSERT INTO import_acquisition(site_id,day,batch_id,source,referrer,pageviews) VALUES(?1,'2025-01-01',?1,'Google','google.com',3)",
                "INSERT INTO import_groups(site_id,day,batch_id,kind,dimensions,pageviews) VALUES(?1,'2025-01-01',?1,'browser','{}',3)",
                "INSERT INTO import_files VALUES(?1,'visitors','[\"visitors\",\"pageviews\"]')",
            ] {
                c.execute(sql, [id])?;
            }
        }
        Ok(())
    }).await.unwrap();
    (
        owlytics::router(db.clone(), config),
        db,
        format!("__Host-owlytics_session={token}"),
        csrf,
    )
}

async fn request(
    app: &Router,
    path: &str,
    cookie: &str,
    form: Option<&[(&str, &str)]>,
    origin: Option<&str>,
) -> Response {
    let mut builder = Request::builder().uri(path).header("cookie", cookie);
    if let Some(origin) = origin {
        builder = builder.header("origin", origin);
    }
    let payload = if let Some(form) = form {
        builder = builder
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded");
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form.iter().copied())
            .finish()
    } else {
        String::new()
    };
    app.clone()
        .oneshot(builder.body(Body::from(payload)).unwrap())
        .await
        .unwrap()
}

async fn text(response: Response) -> String {
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

async fn counts(db: &Database, id: i64) -> Vec<i64> {
    db.read(move |c| {
        SITE_TABLES
            .iter()
            .map(|(table, key)| {
                c.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {key}=?1"),
                    [id],
                    |r| r.get(0),
                )
            })
            .collect()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn deletion_requires_owner_post_origin_csrf_and_matching_domain() {
    let (app, db, cookie, csrf) = fixture().await;
    let before = counts(&db, 1).await;
    let valid = [("csrf", csrf.as_str()), ("domain", "example.com")];
    let signed_out = request(&app, "/sites/1/delete", "", Some(&valid), Some(ORIGIN)).await;
    assert_eq!(signed_out.status(), StatusCode::SEE_OTHER);
    assert_eq!(signed_out.headers()["location"], "/login");
    assert_eq!(
        request(&app, "/sites/1/delete", &cookie, None, None)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    for origin in [None, Some("https://evil.example")] {
        assert_eq!(
            request(&app, "/sites/1/delete", &cookie, Some(&valid), origin)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        request(
            &app,
            "/sites/1/delete",
            &cookie,
            Some(&[("csrf", "invalid"), ("domain", "example.com")]),
            Some(ORIGIN)
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "/sites/1/delete",
            &cookie,
            Some(&[("csrf", &csrf)]),
            Some(ORIGIN)
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for domain in ["", "other.example", "<script>alert(1)</script>"] {
        let response = request(
            &app,
            "/sites/1/delete",
            &cookie,
            Some(&[("csrf", &csrf), ("domain", domain)]),
            Some(ORIGIN),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let html = text(response).await;
        assert!(html.contains("class=\"delete-confirmation\" open"));
        assert!(html.contains("aria-invalid=\"true\""));
        assert!(html.contains("The domain doesn’t match."));
        assert!(!html.contains("<script>alert(1)</script>"));
    }
    assert_eq!(counts(&db, 1).await, before);
    assert_eq!(
        request(
            &app,
            "/sites/999/delete",
            &cookie,
            Some(&valid),
            Some(ORIGIN)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    db.call(|c| c.execute("UPDATE owner_sessions SET expires_at=0", []))
        .await
        .unwrap();
    assert_eq!(
        request(&app, "/sites/1/delete", &cookie, Some(&valid), Some(ORIGIN))
            .await
            .headers()["location"],
        "/login"
    );
    assert_eq!(counts(&db, 1).await, before);
}

#[tokio::test]
async fn deleting_a_site_cleans_all_data_preserves_other_sites_and_stops_collection() {
    let (app, db, cookie, csrf) = fixture().await;
    assert!(
        counts(&db, 1).await.iter().all(|count| *count > 0),
        "Every child table must be exercised"
    );
    let other = counts(&db, 2).await;
    let html = text(request(&app, "/sites/1/settings", &cookie, None, None).await).await;
    assert!(html.contains("<h1>Website settings</h1>"));
    assert!(html.contains("This cannot be undone."));
    assert!(html.contains("action=\"/sites/1/delete\""));
    assert!(html.contains("name=\"csrf\""));
    assert!(html.contains("class=\"delete-confirmation\"><summary"));
    let response = request(
        &app,
        "/sites/1/delete",
        &cookie,
        Some(&[("csrf", &csrf), ("domain", " example.com ")]),
        Some(ORIGIN),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/?deleted=true");
    assert_eq!(counts(&db, 1).await, vec![0; SITE_TABLES.len()]);
    assert_eq!(counts(&db, 2).await, other);
    for path in ["/sites/1", "/sites/1/settings", "/sites/1/export"] {
        assert_eq!(
            request(&app, path, &cookie, None, None).await.status(),
            StatusCode::NOT_FOUND
        );
    }
    let html = text(request(&app, "/?deleted=true", &cookie, None, None).await).await;
    assert!(html.contains("Website and its analytics data deleted."));
    assert!(html.contains("other.example"));
    assert!(!html.contains("href=\"/sites/1\""));
    assert_eq!(
        request(
            &app,
            "/sites/1/delete",
            &cookie,
            Some(&[("csrf", &csrf), ("domain", "example.com")]),
            Some(ORIGIN)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let result = analytics::collect(
        &db,
        analytics::Event {
            domain: "example.com".into(),
            path: "/".into(),
            referrer: String::new(),
            id: "11111111111111111111111111111111".into(),
            campaign: None,
        },
        "203.0.113.1".parse().unwrap(),
        "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36".into(),
        || 1793059200,
    )
    .await
    .unwrap();
    assert_eq!(result, analytics::Collected::UnknownSite);
    assert_eq!(counts(&db, 1).await, vec![0; SITE_TABLES.len()]);
    request(
        &app,
        "/sites/2/delete",
        &cookie,
        Some(&[("csrf", &csrf), ("domain", "other.example")]),
        Some(ORIGIN),
    )
    .await;
    assert!(
        text(request(&app, "/", &cookie, None, None).await)
            .await
            .contains("No websites yet")
    );
    let added = request(
        &app,
        "/sites",
        &cookie,
        Some(&[
            ("csrf", &csrf),
            ("domain", "example.com"),
            ("timezone", "UTC"),
        ]),
        Some(ORIGIN),
    )
    .await;
    assert_eq!(added.status(), StatusCode::SEE_OTHER);
    assert_eq!(counts(&db, 1).await[0], 1);
    assert!(counts(&db, 1).await[1..].iter().all(|count| *count == 0));
}

#[tokio::test]
async fn a_failed_cascade_rolls_back_the_entire_deletion() {
    let (app, db, cookie, csrf) = fixture().await;
    let before = counts(&db, 1).await;
    db.call(|c| c.execute_batch("CREATE TRIGGER block_delete BEFORE DELETE ON events BEGIN SELECT RAISE(ABORT,'blocked'); END;")).await.unwrap();
    let response = request(
        &app,
        "/sites/1/delete",
        &cookie,
        Some(&[("csrf", &csrf), ("domain", "example.com")]),
        Some(ORIGIN),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(counts(&db, 1).await, before);
}
