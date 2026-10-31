use crate::{analytics, auth, config::Config, dashboard, db::Database};
use axum::{
    Form, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(crate) struct App {
    pub(crate) db: Database,
    pub(crate) config: Config,
    pub(crate) password_check: Arc<Semaphore>,
    pub(crate) favicon_fetch: Arc<crate::favicons::Fetches>,
}

impl App {
    pub(crate) fn secure(&self) -> bool {
        self.config.public_url.scheme() == "https"
    }
    pub(crate) fn origin(&self) -> String {
        self.config.public_url.origin().ascii_serialization()
    }
}

pub fn router(db: Database, config: Config) -> Router {
    let tracker = include_str!("../assets/script.js").replace(
        "const respectGpc = true;",
        &format!("const respectGpc = {};", config.respect_gpc),
    );
    let app = App {
        db,
        config,
        password_check: Arc::new(Semaphore::new(1)),
        favicon_fetch: Arc::default(),
    };
    let mut router = Router::new()
        .route("/up", get(up))
        .route("/", get(dashboard::home))
        .route("/sites", post(dashboard::create))
        .route("/sites/new", get(dashboard::new_site))
        .route("/sites/{id}", get(dashboard::show))
        .route("/sites/{id}/filter", get(dashboard::apply_filter))
        .route("/sites/{id}/export", get(dashboard::export))
        .route("/sites/{id}/settings", get(dashboard::settings))
        .route("/sites/{id}/favicon", get(crate::favicons::show))
        .route(
            "/settings",
            get(crate::settings::show).post(crate::settings::change_password),
        )
        .route("/api/event", post(event))
        .route("/api/engagement", post(engagement))
        .route(
            "/script.js",
            asset("text/javascript; charset=utf-8", tracker),
        )
        .route("/login", get(login_page).post(login))
        .route("/logout", post(logout))
        .route(
            "/assets/favicon.svg",
            asset("image/svg+xml", include_str!("../assets/favicon.svg")),
        )
        .route(
            "/assets/favicons.js",
            asset(
                "text/javascript; charset=utf-8",
                include_str!("../assets/favicons.js"),
            ),
        )
        .route(
            "/assets/app.css",
            asset("text/css; charset=utf-8", include_str!("../assets/app.css")),
        )
        .route(
            "/assets/dashboard.js",
            asset(
                "text/javascript; charset=utf-8",
                include_str!("../assets/dashboard.js"),
            ),
        )
        .route(
            crate::geography::country_sprite_url(),
            asset_with_cache(
                "image/svg+xml",
                crate::geography::country_sprite(),
                "public, max-age=31536000, immutable",
            ),
        )
        .route(
            "/assets/country-shapes.svg",
            asset("image/svg+xml", crate::geography::country_sprite()),
        )
        .route(
            "/assets/logo.svg",
            asset("image/svg+xml", include_str!("../assets/logo.svg")),
        )
        .route(
            "/assets/owl.svg",
            asset("image/svg+xml", include_str!("../assets/owl.svg")),
        );
    for icon in crate::brand_assets::assets() {
        router = router.route(
            &icon.url,
            asset_bytes(icon.mime, icon.body, "public, max-age=31536000, immutable"),
        );
    }
    router
        .layer(DefaultBodyLimit::max(8 * 1024))
        .layer(middleware::from_fn(security_headers))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(app)
}

// Revalidate public assets on each use. The tracker includes the instance's
// configuration, so changing that configuration also changes its ETag.
fn asset(
    content_type: &'static str,
    body: impl Into<axum::body::Bytes>,
) -> axum::routing::MethodRouter<App> {
    asset_bytes(content_type, body, "public, no-cache")
}

fn asset_with_cache(
    content_type: &'static str,
    body: &'static str,
    cache_control: &'static str,
) -> axum::routing::MethodRouter<App> {
    asset_bytes(content_type, body.as_bytes(), cache_control)
}

fn asset_bytes(
    content_type: &'static str,
    body: impl Into<axum::body::Bytes>,
    cache_control: &'static str,
) -> axum::routing::MethodRouter<App> {
    use sha2::{Digest, Sha256};
    let body = body.into();
    let etag = format!("\"{:x}\"", Sha256::digest(&body));
    get(move |headers: HeaderMap| {
        let etag = etag.clone();
        let body = body.clone();
        async move {
            let unchanged = headers
                .get_all(header::IF_NONE_MATCH)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .flat_map(|value| value.split(','))
                .map(str::trim)
                .any(|value| value == "*" || value.strip_prefix("W/").unwrap_or(value) == etag);
            let mut response = if unchanged {
                StatusCode::NOT_MODIFIED.into_response()
            } else {
                ([(header::CONTENT_TYPE, content_type)], body).into_response()
            };
            response
                .headers_mut()
                .insert(header::ETAG, etag.parse().expect("hex digest"));
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(cache_control),
            );
            response
        }
    })
}

async fn security_headers(request: Request, next: Next) -> Response {
    let favicon = request.uri().path().ends_with("/favicon");
    let mut response = next.run(request).await;
    // Serve private HTML unchanged through CDNs: injected scripts violate our
    // CSP, and edge transformations can delay delivery of live reports.
    let cache_control = if response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"))
    {
        "no-store, no-transform"
    } else {
        "no-store"
    };
    response
        .headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static(cache_control));
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "same-origin"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    if favicon {
        response.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
        );
    }
    response
}

async fn up(State(app): State<App>) -> Response {
    match app
        .db
        .call(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)))
        .await
    {
        Ok(1) => (StatusCode::OK, "ok").into_response(),
        _ => unavailable(),
    }
}

pub(crate) struct Session {
    pub(crate) token_hash: String,
    pub(crate) csrf: String,
}

pub(crate) async fn session(
    app: &App,
    headers: &HeaderMap,
) -> Result<Option<Session>, crate::db::Error> {
    let Some(token) = auth::cookie(headers, &auth::cookie_name(app.secure(), "session")) else {
        return Ok(None);
    };
    let token_hash = auth::digest(&token);
    let initial = app.config.password_hash.clone();
    app.db.read(move |c| {
        let credential = auth::digest(&auth::owner_hash(c, &initial)?);
        c.query_row(
        "SELECT csrf FROM owner_sessions WHERE token_hash=?1 AND credential_hash=?2 AND expires_at>?3",
        rusqlite::params![token_hash, credential, auth::now()],
        |row| Ok(Session { token_hash: token_hash.clone(), csrf: row.get(0)? })
    ).optional() }).await
}

async fn event(
    State(app): State<App>,
    peer: Option<axum::Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Ok(event) = serde_json::from_slice::<analytics::Event>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    collect_request(app, peer, headers, Payload::Pageview(event)).await
}

async fn engagement(
    State(app): State<App>,
    peer: Option<axum::Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Ok(event) = serde_json::from_slice::<crate::engagement::Event>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    collect_request(app, peer, headers, Payload::Engagement(event)).await
}

enum Payload {
    Pageview(analytics::Event),
    Engagement(crate::engagement::Event),
}

async fn collect_request(
    app: App,
    peer: Option<axum::Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    mut payload: Payload,
) -> Response {
    let (valid, domain) = match &mut payload {
        Payload::Pageview(event) => (event.validate(), &event.domain),
        Payload::Engagement(event) => (event.validate(), &event.domain),
    };
    if !valid {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| url::Url::parse(v).ok());
    let Some(origin) = origin.filter(|url| {
        matches!(url.scheme(), "http" | "https")
            && url
                .host_str()
                .is_some_and(|host| analytics::matches_domain(host, domain))
    }) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let mut response = if headers.get("dnt").is_some_and(|v| v == "1")
        || (app.config.respect_gpc && headers.get("sec-gpc").is_some_and(|v| v == "1"))
    {
        StatusCode::NO_CONTENT.into_response()
    } else {
        let Some(peer) = peer else {
            return unavailable();
        };
        let Some(ip) = analytics::client_ip(
            peer.0.0.ip(),
            headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()),
            &app.config.trusted_proxies,
        ) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let ua = headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let status = match payload {
            Payload::Pageview(event) => {
                match analytics::collect(&app.db, event, ip, ua.to_owned(), auth::now).await {
                    Ok(analytics::Collected::Stored | analytics::Collected::Duplicate) => {
                        StatusCode::NO_CONTENT
                    }
                    Ok(analytics::Collected::UnknownSite) => StatusCode::NOT_FOUND,
                    Ok(analytics::Collected::Invalid) => StatusCode::BAD_REQUEST,
                    Err(_) => StatusCode::SERVICE_UNAVAILABLE,
                }
            }
            Payload::Engagement(event) => {
                match crate::engagement::collect(&app.db, event, ua.to_owned(), auth::now).await {
                    Ok(
                        crate::engagement::Collected::Stored
                        | crate::engagement::Collected::Duplicate,
                    ) => StatusCode::NO_CONTENT,
                    Ok(crate::engagement::Collected::Invalid) => StatusCode::BAD_REQUEST,
                    Ok(crate::engagement::Collected::PendingPageview) | Err(_) => {
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                }
            }
        };
        if status == StatusCode::SERVICE_UNAVAILABLE {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::RETRY_AFTER, "1")],
            )
                .into_response()
        } else {
            status.into_response()
        }
    };
    if let Ok(value) = origin.origin().ascii_serialization().parse() {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Origin"));
    response
}

async fn login_page(State(app): State<App>, headers: HeaderMap) -> Response {
    match session(&app, &headers).await {
        Ok(Some(_)) => return Redirect::to("/").into_response(),
        Err(_) => return unavailable(),
        _ => {}
    }
    let Ok(csrf) = auth::random_token() else {
        return unavailable();
    };
    let mut response = login_form(&csrf, None, StatusCode::OK);
    response.headers_mut().append(
        header::SET_COOKIE,
        auth::set_cookie(app.secure(), "csrf", &csrf, 3600)
            .parse()
            .unwrap(),
    );
    response
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
    csrf: String,
}

async fn login(
    State(app): State<App>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let csrf = auth::cookie(&headers, &auth::cookie_name(app.secure(), "csrf"));
    if !auth::same_origin(&headers, &app.origin())
        || !csrf.is_some_and(|csrf| auth::matches_token(&csrf, &form.csrf))
    {
        return (
            StatusCode::FORBIDDEN,
            "This form expired. Reload the sign-in page and try again.",
        )
            .into_response();
    }
    if form.password.len() > 1024 {
        return login_form(
            &form.csrf,
            Some("That password is too long."),
            StatusCode::BAD_REQUEST,
        );
    }
    let Ok(permit) = app.password_check.clone().try_acquire_owned() else {
        return rate_limited();
    };
    let permitted = password_attempt(&app).await;
    match permitted {
        Ok(true) => {}
        Ok(false) => return rate_limited(),
        Err(_) => return unavailable(),
    }
    let initial = app.config.password_hash.clone();
    let hash = match app.db.read(move |c| auth::owner_hash(c, &initial)).await {
        Ok(hash) => hash,
        Err(_) => return unavailable(),
    };
    let verified_hash = hash.clone();
    let verified = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        auth::verify_password(&form.password, &hash)
    })
    .await;
    match verified {
        Ok(true) => {}
        Ok(false) => {
            return login_form(
                &form.csrf,
                Some("That password didn’t match. Try again."),
                StatusCode::UNAUTHORIZED,
            );
        }
        Err(_) => return unavailable(),
    }
    let (Ok(token), Ok(csrf)) = (auth::random_token(), auth::random_token()) else {
        return unavailable();
    };
    let token_hash = auth::digest(&token);
    let credential = auth::digest(&verified_hash);
    let initial = app.config.password_hash.clone();
    let previous = auth::cookie(&headers, &auth::cookie_name(app.secure(), "session"))
        .map(|token| auth::digest(&token));
    let saved = app.db.call(move |c| {
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // A password change while Argon2 was running must not create a session
        // for the superseded credential.
        if auth::owner_hash(&tx, &initial)? != verified_hash { return Ok(false); }
        tx.execute("DELETE FROM owner_sessions WHERE expires_at<=?1 OR credential_hash!=?2 OR token_hash=?3", rusqlite::params![auth::now(), credential, previous])?;
        tx.execute("INSERT INTO owner_sessions VALUES(?1,?2,?3,?4)", rusqlite::params![token_hash, csrf, credential, auth::now()+auth::SESSION_SECONDS])?;
        tx.execute("DELETE FROM owner_sessions WHERE token_hash IN (SELECT token_hash FROM owner_sessions ORDER BY expires_at DESC, rowid DESC LIMIT -1 OFFSET 20)", [])?;
        tx.commit()?;
        Ok(true)
    }).await;
    match saved {
        Ok(true) => {}
        Ok(false) => return Redirect::to("/login").into_response(),
        Err(_) => return unavailable(),
    }
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        auth::set_cookie(app.secure(), "session", &token, auth::SESSION_SECONDS)
            .parse()
            .unwrap(),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        auth::set_cookie(app.secure(), "csrf", "", 0)
            .parse()
            .unwrap(),
    );
    response
}

pub(crate) async fn password_attempt(app: &App) -> Result<bool, crate::db::Error> {
    app.db.call(|c| {
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = auth::now();
        tx.execute("INSERT INTO login_limit VALUES(1,?1,0) ON CONFLICT(id) DO NOTHING", [now])?;
        tx.execute("UPDATE login_limit SET window_start=?1,attempts=0 WHERE window_start<=?1-60 OR window_start>?1", [now])?;
        let changed = tx.execute("UPDATE login_limit SET attempts=attempts+1 WHERE id=1 AND attempts<10", [])?;
        tx.commit()?;
        Ok(changed == 1)
    }).await
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf: String,
}

async fn logout(
    State(app): State<App>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    let session = match session(&app, &headers).await {
        Ok(Some(session)) => session,
        Ok(None) => return Redirect::to("/login").into_response(),
        Err(_) => return unavailable(),
    };
    if !auth::same_origin(&headers, &app.origin())
        || !auth::matches_token(&session.csrf, &form.csrf)
    {
        return (StatusCode::FORBIDDEN, "Reload the page and try again.").into_response();
    }
    if app
        .db
        .call(move |c| {
            c.execute(
                "DELETE FROM owner_sessions WHERE token_hash=?1",
                [session.token_hash],
            )
        })
        .await
        .is_err()
    {
        return unavailable();
    }
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        auth::set_cookie(app.secure(), "session", "", 0)
            .parse()
            .unwrap(),
    );
    response
}

pub(crate) fn rate_limited() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, "60")],
        "Too many sign-in attempts. Please try again in a minute.",
    )
        .into_response()
}

pub(crate) fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Temporarily unavailable. Please try again.",
    )
        .into_response()
}

fn login_form(csrf: &str, error: Option<&str>, status: StatusCode) -> Response {
    let error = error
        .map(|message| format!("<p class=\"error\" role=\"alert\">{}</p>", escape(message)))
        .unwrap_or_default();
    (
        status,
        Html(shell(
            "Welcome home",
            &format!(
                include_str!("../templates/login.html"),
                csrf = escape(csrf),
                error = error
            ),
        )),
    )
        .into_response()
}

pub(crate) fn shell(title: &str, body: &str) -> String {
    format!(
        include_str!("../templates/layout.html"),
        title = escape(title),
        body = body
    )
}

pub(crate) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
