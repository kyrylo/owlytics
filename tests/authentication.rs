mod common;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use http_body_util::BodyExt;
use owlytics::{auth, db::Database};
use tower::ServiceExt;

async fn request(
    app: &Router,
    path: &str,
    cookie: &str,
    form: Option<&[(&str, &str)]>,
    origin: Option<&str>,
) -> Response {
    let mut builder = Request::builder().uri(path).header(header::COOKIE, cookie);
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let body = if let Some(fields) = form {
        builder = builder
            .method("POST")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        Body::from(
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(fields.iter().copied())
                .finish(),
        )
    } else {
        Body::empty()
    };
    app.clone()
        .oneshot(builder.body(body).unwrap())
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

fn cookie(response: &Response) -> String {
    response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into()
}

fn csrf(html: &str) -> String {
    html.split("name=\"csrf\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .into()
}

async fn login(app: &Router) -> String {
    let page = request(app, "/login", "", None, None).await;
    let pre_cookie = cookie(&page);
    let token = csrf(&text(page).await);
    let result = request(
        app,
        "/login",
        &pre_cookie,
        Some(&[("password", common::PASSWORD), ("csrf", &token)]),
        Some("https://stats.example.com"),
    )
    .await;
    assert_eq!(result.status(), StatusCode::SEE_OTHER);
    assert_eq!(result.headers()[header::LOCATION], "/");
    let set_cookie = result.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(set_cookie.contains("Secure"));
    assert!(set_cookie.starts_with("__Host-"));
    cookie(&result)
}

#[tokio::test]
async fn owner_can_sign_in_and_log_out_and_old_cookie_is_revoked() {
    let db = Database::open(":memory:").unwrap();
    let app = owlytics::router(db.clone(), common::config());
    let anonymous = request(&app, "/", "", None, None).await;
    assert_eq!(anonymous.status(), StatusCode::SEE_OTHER);
    assert_eq!(anonymous.headers()[header::LOCATION], "/login");
    let owner_cookie = login(&app).await;
    let home = request(&app, "/", &owner_cookie, None, None).await;
    assert_eq!(home.status(), StatusCode::OK);
    assert_eq!(
        home.headers()[header::CACHE_CONTROL],
        "no-store, no-transform"
    );
    assert_eq!(home.headers()["x-frame-options"], "DENY");
    let token = csrf(&text(home).await);
    let raw = owner_cookie.split('=').nth(1).unwrap();
    let stored: String = db
        .call(|c| c.query_row("SELECT token_hash FROM owner_sessions", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_ne!(stored, raw);
    assert_eq!(stored, auth::digest(raw));
    let forbidden = request(
        &app,
        "/logout",
        &owner_cookie,
        Some(&[("csrf", &token)]),
        Some("https://evil.example"),
    )
    .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let result = request(
        &app,
        "/logout",
        &owner_cookie,
        Some(&[("csrf", &token)]),
        Some("https://stats.example.com"),
    )
    .await;
    assert_eq!(result.status(), StatusCode::SEE_OTHER);
    assert!(
        result.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(
        request(&app, "/", &owner_cookie, None, None).await.status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn rejects_wrong_password_csrf_cross_origin_and_oversized_requests() {
    let db = Database::open(":memory:").unwrap();
    let app = owlytics::router(db.clone(), common::config());
    let page = request(&app, "/login", "", None, None).await;
    let pre_cookie = cookie(&page);
    let token = csrf(&text(page).await);
    for origin in [Some("https://evil.example"), Some("null"), None] {
        let result = request(
            &app,
            "/login",
            &pre_cookie,
            Some(&[("csrf", &token), ("password", common::PASSWORD)]),
            origin,
        )
        .await;
        assert_eq!(result.status(), StatusCode::FORBIDDEN);
    }
    let invalid = request(
        &app,
        "/login",
        &pre_cookie,
        Some(&[("csrf", &"b".repeat(64)), ("password", common::PASSWORD)]),
        Some("https://stats.example.com"),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::FORBIDDEN);
    let wrong = request(
        &app,
        "/login",
        &pre_cookie,
        Some(&[("csrf", &token), ("password", "incorrect password")]),
        Some("https://stats.example.com"),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let oversized = request(
        &app,
        "/login",
        &pre_cookie,
        Some(&[("csrf", &token), ("password", &"x".repeat(9000))]),
        Some("https://stats.example.com"),
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let sessions: i64 = db
        .call(|c| c.query_row("SELECT COUNT(*) FROM owner_sessions", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(sessions, 0);
}

#[tokio::test]
async fn expired_sessions_and_password_changes_require_sign_in() {
    let db = Database::open(":memory:").unwrap();
    let app = owlytics::router(db.clone(), common::config());
    let owner_cookie = login(&app).await;
    let mut changed = common::config();
    let duplicate = format!("{owner_cookie}; {owner_cookie}");
    assert_eq!(
        request(&app, "/", &duplicate, None, None).await.status(),
        StatusCode::SEE_OTHER
    );
    changed.password_hash = auth::hash_password("a newly changed password").unwrap();
    let restarted = owlytics::router(db.clone(), changed);
    assert_eq!(
        request(&restarted, "/", &owner_cookie, None, None)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    db.call(|c| c.execute("UPDATE owner_sessions SET expires_at=?1", [auth::now() - 1]))
        .await
        .unwrap();
    assert_eq!(
        request(&app, "/", &owner_cookie, None, None).await.status(),
        StatusCode::SEE_OTHER
    );
    let duplicate = format!("{owner_cookie}; {owner_cookie}");
    assert_eq!(
        request(&app, "/", &duplicate, None, None).await.status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn login_rate_limit_persists_across_router_restarts_and_recovers_after_window() {
    let db = Database::open(":memory:").unwrap();
    let app = owlytics::router(db.clone(), common::config());
    let page = request(&app, "/login", "", None, None).await;
    let pre_cookie = cookie(&page);
    let token = csrf(&text(page).await);
    db.call(|c| c.execute("INSERT INTO login_limit VALUES(1,?1,9)", [auth::now()]))
        .await
        .unwrap();
    let result = request(
        &app,
        "/login",
        &pre_cookie,
        Some(&[("csrf", &token), ("password", "incorrect password")]),
        Some("https://stats.example.com"),
    )
    .await;
    assert_eq!(result.status(), StatusCode::UNAUTHORIZED);
    let restarted = owlytics::router(db.clone(), common::config());
    let result = request(
        &restarted,
        "/login",
        &pre_cookie,
        Some(&[("csrf", &token), ("password", common::PASSWORD)]),
        Some("https://stats.example.com"),
    )
    .await;
    assert_eq!(result.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(result.headers()[header::RETRY_AFTER], "60");
    db.call(|c| c.execute("UPDATE login_limit SET window_start=?1", [auth::now() - 61]))
        .await
        .unwrap();
    let result = request(
        &restarted,
        "/login",
        &pre_cookie,
        Some(&[("csrf", &token), ("password", common::PASSWORD)]),
        Some("https://stats.example.com"),
    )
    .await;
    assert_eq!(result.status(), StatusCode::SEE_OTHER);
}

async fn attempt_password(app: &Router, password: &str) -> Response {
    let page = request(app, "/login", "", None, None).await;
    let pre_cookie = cookie(&page);
    let token = csrf(&text(page).await);
    request(
        app,
        "/login",
        &pre_cookie,
        Some(&[("password", password), ("csrf", &token)]),
        Some("https://stats.example.com"),
    )
    .await
}

#[tokio::test]
async fn password_settings_require_current_password_csrf_and_confirmation() {
    let db = Database::open(":memory:").unwrap();
    let app = owlytics::router(db.clone(), common::config());
    assert_eq!(
        request(&app, "/settings", "", None, None).await.status(),
        StatusCode::SEE_OTHER
    );
    let owner = login(&app).await;
    let token = csrf(&text(request(&app, "/settings", &owner, None, None).await).await);
    let new = "a new private owl password";
    for (current, new, confirm, csrf_value, origin, status) in [
        (
            common::PASSWORD,
            new,
            new,
            token.as_str(),
            "https://evil.example",
            StatusCode::FORBIDDEN,
        ),
        (
            common::PASSWORD,
            new,
            new,
            "bad-token",
            "https://stats.example.com",
            StatusCode::FORBIDDEN,
        ),
        (
            "incorrect current",
            new,
            new,
            token.as_str(),
            "https://stats.example.com",
            StatusCode::UNAUTHORIZED,
        ),
        (
            common::PASSWORD,
            "short",
            "short",
            token.as_str(),
            "https://stats.example.com",
            StatusCode::BAD_REQUEST,
        ),
        (
            common::PASSWORD,
            new,
            "a different confirmation",
            token.as_str(),
            "https://stats.example.com",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = request(
            &app,
            "/settings",
            &owner,
            Some(&[
                ("csrf", csrf_value),
                ("current_password", current),
                ("new_password", new),
                ("confirm_password", confirm),
            ]),
            Some(origin),
        )
        .await;
        assert_eq!(response.status(), status);
        assert!(!text(response).await.contains(new));
    }
    assert_eq!(
        db.read(
            |c| c.query_row("SELECT COUNT(*) FROM owner_credentials", [], |r| r
                .get::<_, i64>(0))
        )
        .await
        .unwrap(),
        0
    );
    db.call(|c| c.execute("UPDATE login_limit SET attempts=10", []))
        .await
        .unwrap();
    assert_eq!(
        request(
            &app,
            "/settings",
            &owner,
            Some(&[
                ("csrf", &token),
                ("current_password", common::PASSWORD),
                ("new_password", new),
                ("confirm_password", new)
            ]),
            Some("https://stats.example.com")
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn changed_password_survives_restart_backup_and_cli_recovery_revokes_sessions() {
    use std::io::Write;
    let folder = std::env::temp_dir().join(format!(
        "owlytics-password-{}",
        auth::random_token().unwrap()
    ));
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("test.sqlite");
    let db = Database::open(&path).unwrap();
    let app = owlytics::router(db.clone(), common::config());
    let first = login(&app).await;
    let second = login(&app).await;
    let token = csrf(&text(request(&app, "/settings", &first, None, None).await).await);
    let new = "a new private owl password";
    let changed = request(
        &app,
        "/settings",
        &first,
        Some(&[
            ("csrf", &token),
            ("current_password", common::PASSWORD),
            ("new_password", new),
            ("confirm_password", new),
        ]),
        Some("https://stats.example.com"),
    )
    .await;
    assert_eq!(changed.status(), StatusCode::SEE_OTHER);
    assert_eq!(changed.headers()[header::LOCATION], "/login");
    for old in [&first, &second] {
        assert_eq!(
            request(&app, "/", old, None, None).await.status(),
            StatusCode::SEE_OTHER
        );
    }
    let restarted = owlytics::router(Database::open(&path).unwrap(), common::config());
    assert_eq!(
        attempt_password(&restarted, common::PASSWORD)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let signed_in = attempt_password(&restarted, new).await;
    assert_eq!(signed_in.status(), StatusCode::SEE_OTHER);
    let changed_cookie = cookie(&signed_in);
    let backup = folder.join("backup.sqlite");
    let output = backup.clone();
    db.read(move |c| Ok(owlytics::storage::backup(c, &output).unwrap()))
        .await
        .unwrap();
    let copy = Database::open(&backup).unwrap();
    let restored = owlytics::router(copy, common::config());
    assert_eq!(
        attempt_password(&restored, new).await.status(),
        StatusCode::SEE_OTHER
    );
    let recovery = "recovered private owl password";
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_owlytics"))
        .arg("reset-password")
        .env("OWLYTICS_DATABASE", &path)
        .env_remove("OWLYTICS_PASSWORD_HASH")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(recovery.as_bytes())
        .unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(
        request(&restarted, "/", &changed_cookie, None, None)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        attempt_password(&restarted, new).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        attempt_password(&restarted, recovery).await.status(),
        StatusCode::SEE_OTHER
    );
    drop(restarted);
    drop(restored);
    drop(app);
    drop(db);
    std::fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
async fn cached_favicons_are_owner_only_and_svg_is_sandboxed() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        c.execute(
            "INSERT INTO site_favicons VALUES(1,'image/svg+xml',?1,?2)",
            rusqlite::params![
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".as_slice(),
                auth::now()
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let app = owlytics::router(db, common::config());
    assert_eq!(
        request(&app, "/sites/1/favicon", "", None, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let owner = login(&app).await;
    let icon = request(&app, "/sites/1/favicon", &owner, None, None).await;
    assert_eq!(icon.status(), StatusCode::OK);
    assert_eq!(icon.headers()[header::CONTENT_TYPE], "image/svg+xml");
    let max_age = icon.headers()[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .strip_prefix("private, max-age=")
        .unwrap()
        .parse::<i64>()
        .unwrap();
    assert!((604790..=604800).contains(&max_age));
    let etag = icon.headers()[header::ETAG].to_str().unwrap();
    for (cookie, status) in [
        (&owner[..], StatusCode::NOT_MODIFIED),
        ("", StatusCode::UNAUTHORIZED),
    ] {
        let revalidated = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sites/1/favicon")
                    .header(header::COOKIE, cookie)
                    .header(header::IF_NONE_MATCH, format!("\"other\", W/{etag}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revalidated.status(), status);
        if status == StatusCode::NOT_MODIFIED {
            assert_eq!(revalidated.headers()[header::ETAG], etag);
            assert!(
                revalidated
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty()
            );
        } else {
            assert_eq!(revalidated.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
    assert!(
        icon.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );
    assert_eq!(
        request(&app, "/sites/999/favicon", &owner, None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn missing_favicons_are_cached_and_stale_icons_are_served_during_refresh() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute(
            "INSERT INTO sites VALUES(1,'127.0.0.1','UTC',0),(2,'localhost','UTC',0)",
            [],
        )?;
        c.execute(
            "INSERT INTO site_favicons VALUES(1,'',X'',?1)",
            [auth::now() - 120],
        )?;
        c.execute(
            "INSERT INTO site_favicons VALUES(2,'image/svg+xml',?1,?2)",
            rusqlite::params![
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".as_slice(),
                auth::now() - 8 * 86400
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let app = owlytics::router(db, common::config());
    let owner = login(&app).await;
    let missing = request(&app, "/sites/1/favicon", &owner, None, None).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let max_age = missing.headers()[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .strip_prefix("private, max-age=")
        .unwrap()
        .parse::<i64>()
        .unwrap();
    assert!((3470..=3480).contains(&max_age));
    let stale = request(&app, "/sites/2/favicon", &owner, None, None).await;
    assert_eq!(stale.status(), StatusCode::OK);
    assert_eq!(
        stale.headers()[header::CACHE_CONTROL],
        "private, max-age=60"
    );
    assert_eq!(
        text(stale).await,
        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>"
    );
}
