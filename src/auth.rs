use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::http::{HeaderMap, header};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const SESSION_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Environment credentials bootstrap an installation. Once changed in Settings,
/// the SQLite credential takes precedence, including after a restart.
pub fn owner_hash(connection: &rusqlite::Connection, initial: &str) -> rusqlite::Result<String> {
    use rusqlite::OptionalExtension;
    Ok(connection
        .query_row(
            "SELECT password_hash FROM owner_credentials WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_else(|| initial.to_owned()))
}

pub fn replace_owner_password(
    connection: &mut rusqlite::Connection,
    hash: &str,
) -> rusqlite::Result<()> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("INSERT INTO owner_credentials VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET password_hash=excluded.password_hash", [hash])?;
    tx.execute("DELETE FROM owner_sessions", [])?;
    tx.execute("DELETE FROM login_limit", [])?;
    tx.commit()
}

fn hasher() -> Argon2<'static> {
    // OWASP's lower-memory Argon2id profile; the application bounds concurrency.
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(7168, 5, 1, Some(32)).expect("fixed Argon2 parameters"),
    )
}

pub fn random_token() -> Result<String, &'static str> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "system randomness unavailable")?;
    Ok(hex::encode(bytes))
}

pub fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

pub fn hash_password(password: &str) -> Result<String, String> {
    if password.chars().count() < 12 || password.len() > 1024 {
        return Err("Use a password of at least 12 characters and at most 1024 bytes".into());
    }
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| "System randomness unavailable")?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| "Cannot encode salt")?;
    hasher()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| "Cannot hash password".into())
}

pub fn validate_hash(value: &str) -> bool {
    let Ok(hash) = PasswordHash::new(value) else {
        return false;
    };
    hash.algorithm.as_str() == "argon2id"
        && hash.version == Some(19)
        && hash.params.get_decimal("m") == Some(7168)
        && hash.params.get_decimal("t") == Some(5)
        && hash.params.get_decimal("p") == Some(1)
        && hash.salt.is_some()
        && hash.hash.is_some_and(|output| output.len() == 32)
}

pub fn verify_password(password: &str, value: &str) -> bool {
    if password.len() > 1024 {
        return false;
    }
    let Ok(hash) = PasswordHash::new(value) else {
        return false;
    };
    hasher().verify_password(password.as_bytes(), &hash).is_ok()
}

pub fn valid_token(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn matches_token(left: &str, right: &str) -> bool {
    valid_token(left) && valid_token(right) && bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}

pub fn cookie_name(secure: bool, purpose: &str) -> String {
    format!("{}owlytics_{purpose}", if secure { "__Host-" } else { "" })
}

pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        for pair in value.to_str().ok()?.split(';') {
            let Some((key, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if key == name {
                if found.is_some() || !valid_token(value) {
                    return None;
                }
                found = Some(value.to_owned());
            }
        }
    }
    found
}

pub fn set_cookie(secure: bool, purpose: &str, value: &str, seconds: i64) -> String {
    format!(
        "{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={seconds}{}",
        cookie_name(secure, purpose),
        if secure { "; Secure" } else { "" }
    )
}

pub fn same_origin(headers: &HeaderMap, origin: &str) -> bool {
    if let Some(value) = headers.get(header::ORIGIN) {
        return value.to_str().is_ok_and(|value| value == origin);
    }
    headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| url::Url::parse(value).ok())
        .is_some_and(|url| url.origin().ascii_serialization() == origin)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hashes_are_salted_and_verified_without_truncation() {
        let password = "an owls very long passphrase";
        let first = hash_password(password).unwrap();
        let second = hash_password(password).unwrap();
        assert_ne!(first, second);
        assert!(validate_hash(&first));
        assert!(verify_password(password, &first));
        assert!(!verify_password("another passphrase", &first));
        assert!(!verify_password(&"x".repeat(1025), &first));
        assert!(!validate_hash(&first.replace("m=7168", "m=1048576")));
        assert!(hash_password("too short").is_err());
    }
}
