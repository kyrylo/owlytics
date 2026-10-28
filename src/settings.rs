use crate::{
    auth, dashboard,
    web::{App, Session, escape, password_attempt, rate_limited, session, unavailable},
};
use axum::{
    Form,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;

fn form(session: &Session, error: Option<&str>, status: StatusCode) -> Response {
    let error = error
        .map(|message| format!("<p class=\"error\" role=\"alert\">{}</p>", escape(message)))
        .unwrap_or_default();
    let mut response = dashboard::page(
        "Settings",
        session,
        &format!(
            "<a class=\"back\" href=\"/\">← Websites</a><section class=\"form-card\"><h1>Change password</h1>{error}<form method=\"post\" action=\"/settings\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><label for=\"current-password\">Current password</label><input id=\"current-password\" name=\"current_password\" type=\"password\" autocomplete=\"current-password\" required maxlength=\"1024\"><label for=\"new-password\">New password</label><input id=\"new-password\" name=\"new_password\" type=\"password\" autocomplete=\"new-password\" required minlength=\"12\" maxlength=\"1024\" aria-describedby=\"password-help\"><label for=\"confirm-password\">Confirm password</label><input id=\"confirm-password\" name=\"confirm_password\" type=\"password\" autocomplete=\"new-password\" required minlength=\"12\" maxlength=\"1024\"><p class=\"muted\" id=\"password-help\">At least 12 characters. Changing it signs out all sessions.</p><button class=\"primary\">Change password</button></form></section>",
            escape(&session.csrf)
        ),
    );
    *response.status_mut() = status;
    response
}

pub(crate) async fn show(State(app): State<App>, headers: HeaderMap) -> Response {
    match session(&app, &headers).await {
        Ok(Some(owner)) => form(&owner, None, StatusCode::OK),
        Ok(None) => Redirect::to("/login").into_response(),
        Err(_) => unavailable(),
    }
}

#[derive(Deserialize)]
pub(crate) struct PasswordForm {
    csrf: String,
    current_password: String,
    new_password: String,
    confirm_password: String,
}

pub(crate) async fn change_password(
    State(app): State<App>,
    headers: HeaderMap,
    Form(input): Form<PasswordForm>,
) -> Response {
    let owner = match session(&app, &headers).await {
        Ok(Some(owner)) => owner,
        Ok(None) => return Redirect::to("/login").into_response(),
        Err(_) => return unavailable(),
    };
    if !auth::same_origin(&headers, &app.origin()) || !auth::matches_token(&owner.csrf, &input.csrf)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let invalid = if input.current_password.len() > 1024
        || input.new_password.len() > 1024
        || input.new_password.chars().count() < 12
    {
        Some("Use a password of at least 12 characters and at most 1024 bytes.")
    } else if input.new_password != input.confirm_password {
        Some("The new passwords don’t match.")
    } else if input.new_password == input.current_password {
        Some("Choose a different password.")
    } else {
        None
    };
    if let Some(message) = invalid {
        return form(&owner, Some(message), StatusCode::BAD_REQUEST);
    }
    let Ok(permit) = app.password_check.clone().try_acquire_owned() else {
        return rate_limited();
    };
    match password_attempt(&app).await {
        Ok(true) => {}
        Ok(false) => return rate_limited(),
        Err(_) => return unavailable(),
    }
    let initial = app.config.password_hash.clone();
    let old_hash = match app.db.read(move |c| auth::owner_hash(c, &initial)).await {
        Ok(hash) => hash,
        Err(_) => return unavailable(),
    };
    let checked_hash = old_hash.clone();
    let replacement = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if !auth::verify_password(&input.current_password, &checked_hash) {
            return Ok(None);
        }
        auth::hash_password(&input.new_password).map(Some)
    })
    .await;
    let new_hash = match replacement {
        Ok(Ok(Some(hash))) => hash,
        Ok(Ok(None)) => {
            return form(
                &owner,
                Some("The current password didn’t match."),
                StatusCode::UNAUTHORIZED,
            );
        }
        _ => return unavailable(),
    };
    let initial = app.config.password_hash.clone();
    let saved = app.db.call(move |c| {
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let still_signed_in: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM owner_sessions WHERE token_hash=?1 AND credential_hash=?2 AND expires_at>?3)", rusqlite::params![owner.token_hash, auth::digest(&old_hash), auth::now()], |r| r.get(0))?;
        if !still_signed_in || auth::owner_hash(&tx, &initial)? != old_hash { return Ok(false); }
        tx.execute("INSERT INTO owner_credentials VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET password_hash=excluded.password_hash", [new_hash])?;
        tx.execute("DELETE FROM owner_sessions", [])?;
        tx.execute("DELETE FROM login_limit", [])?;
        tx.commit()?;
        Ok(true)
    }).await;
    if saved.is_err() {
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
