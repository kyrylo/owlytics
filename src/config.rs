use std::{net::SocketAddr, path::PathBuf};

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub database: PathBuf,
    pub public_url: url::Url,
    pub password_hash: String,
    pub trusted_proxies: Vec<ipnet::IpNet>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self::parse(
            &std::env::var("OWLYTICS_BIND").unwrap_or_else(|_| "127.0.0.1:3000".into()),
            &std::env::var("OWLYTICS_DATABASE").unwrap_or_else(|_| "data/owlytics.sqlite".into()),
            &std::env::var("OWLYTICS_URL").unwrap_or_else(|_| "http://localhost:3000".into()),
            &std::env::var("OWLYTICS_PASSWORD_HASH").map_err(|_| ConfigError("Set OWLYTICS_PASSWORD_HASH using `owlytics hash-password` before starting the server".into()))?,
        )?;
        config.trusted_proxies = std::env::var("OWLYTICS_TRUSTED_PROXIES")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                s.trim().parse().map_err(|_| {
                    ConfigError(
                        "OWLYTICS_TRUSTED_PROXIES must contain comma-separated IP networks (CIDR)"
                            .into(),
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(config)
    }

    pub fn parse(
        bind: &str,
        database: &str,
        public_url: &str,
        password_hash: &str,
    ) -> Result<Self, ConfigError> {
        let bind = bind
            .parse()
            .map_err(|_| ConfigError("OWLYTICS_BIND must be an IP address and port".into()))?;
        if database.trim().is_empty() {
            return Err(ConfigError("OWLYTICS_DATABASE must be a file path".into()));
        }
        let public_url = url::Url::parse(public_url)
            .map_err(|_| ConfigError("OWLYTICS_URL must be an absolute HTTP(S) URL".into()))?;
        if !matches!(public_url.scheme(), "http" | "https")
            || public_url.host_str().is_none()
            || !public_url.username().is_empty()
            || public_url.password().is_some()
            || public_url.path() != "/"
            || public_url.query().is_some()
            || public_url.fragment().is_some()
        {
            return Err(ConfigError("OWLYTICS_URL must be an HTTP(S) origin without credentials, path, query, or fragment".into()));
        }
        if !crate::auth::validate_hash(password_hash) {
            return Err(ConfigError(
                "OWLYTICS_PASSWORD_HASH is invalid; generate it with `owlytics hash-password`"
                    .into(),
            ));
        }
        Ok(Self {
            bind,
            database: database.into(),
            public_url,
            password_hash: password_hash.into(),
            trusted_proxies: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_origins_and_bind_addresses() {
        let hash = crate::auth::hash_password("configuration test password").unwrap();
        for bad in [
            "ftp://example.com",
            "https://user:password@example.com",
            "https://example.com/path",
            "https://example.com?x=1",
            "https://example.com/#fragment",
            "not a URL",
        ] {
            assert!(Config::parse("127.0.0.1:3000", "test.sqlite", bad, &hash).is_err());
        }
        assert!(
            Config::parse("invalid", "test.sqlite", "https://stats.example.com", &hash).is_err()
        );
        assert!(Config::parse("127.0.0.1:3000", "", "https://stats.example.com", &hash).is_err());
        assert!(
            Config::parse(
                "127.0.0.1:3000",
                "test.sqlite",
                "https://stats.example.com",
                &hash
            )
            .is_ok()
        );
    }
}
