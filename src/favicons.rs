//! Owner-only favicon discovery. HTTPS requests are bounded, DNS-pinned to
//! public addresses, and cached in SQLite; visitors never contact an icon vendor.
use crate::{
    auth,
    web::{App, session, unavailable},
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use rusqlite::OptionalExtension;
use std::{net::IpAddr, time::Duration};
use url::Url;

const LIMIT: usize = 256 * 1024;
type Icon = (String, Vec<u8>);

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let b = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || b[0] == 0
                || b[0] >= 224
                || (b[0] == 100 && (64..=127).contains(&b[1]))
                || (b[0] == 198 && (18..=19).contains(&b[1]))
                || (b[0] == 192 && b[1] == 0 && b[2] == 0))
        }
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or_else(
            || {
                let b = ip.segments();
                b[0] & 0xe000 == 0x2000
                    && b[0] != 0x2002
                    && !(b[0] == 0x2001 && (b[1] == 0xdb8 || b[1] < 0x200))
            },
            |v4| public_ip(IpAddr::V4(v4)),
        ),
    }
}

async fn fetch(mut url: Url) -> Option<(Url, String, Vec<u8>)> {
    for _ in 0..4 {
        if url.scheme() != "https"
            || url.port_or_known_default() != Some(443)
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        let host = url.host_str()?;
        let addresses: Vec<_> = tokio::net::lookup_host((host, 443)).await.ok()?.collect();
        if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
            return None;
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .user_agent("Mozilla/5.0 (compatible; Owlytics favicon fetcher)")
            .resolve_to_addrs(host, &addresses)
            .build()
            .ok()?;
        let mut response = client.get(url.clone()).send().await.ok()?;
        if response.status().is_redirection() {
            url = url
                .join(
                    response
                        .headers()
                        .get(reqwest::header::LOCATION)?
                        .to_str()
                        .ok()?,
                )
                .ok()?;
            continue;
        }
        if !response.status().is_success()
            || response.content_length().is_some_and(|n| n > LIMIT as u64)
        {
            return None;
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)?
            .to_str()
            .ok()?
            .split(';')
            .next()?
            .trim()
            .to_ascii_lowercase();
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if data.len() + chunk.len() > LIMIT {
                return None;
            }
            data.extend_from_slice(&chunk);
            if mime == "text/html"
                && String::from_utf8_lossy(&data)
                    .to_ascii_lowercase()
                    .contains("</head>")
            {
                break;
            }
        }
        return Some((url, mime, data));
    }
    None
}

// A small attribute tokenizer, not an HTML renderer. Only <link rel=icon href>
// is consumed; all resulting URLs still pass the network checks above.
fn icon_links(html: &str, base: &Url) -> Vec<Url> {
    let lower = html.to_ascii_lowercase();
    let mut links = Vec::new();
    for (start, _) in lower.match_indices("<link") {
        if !lower
            .as_bytes()
            .get(start + 5)
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            continue;
        }
        let Some(end) = html[start..].find('>') else {
            continue;
        };
        let mut rest = &html[start + 5..start + end];
        let mut rel = String::new();
        let mut href = String::new();
        while !rest.is_empty() {
            rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
            let len = rest
                .find(|c: char| c.is_ascii_whitespace() || c == '=')
                .unwrap_or(rest.len());
            if len == 0 {
                break;
            }
            let name = rest[..len].to_ascii_lowercase();
            rest = rest[len..].trim_start();
            if !rest.starts_with('=') {
                continue;
            }
            rest = rest[1..].trim_start();
            let (value, consumed) = if rest.starts_with(['\'', '"']) {
                let quote = rest.as_bytes()[0] as char;
                let Some(end) = rest[1..].find(quote) else {
                    break;
                };
                (&rest[1..1 + end], end + 2)
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (&rest[..end], end)
            };
            if name == "rel" {
                rel = value.to_ascii_lowercase();
            }
            if name == "href" {
                href = value.replace("&amp;", "&");
            }
            rest = &rest[consumed..];
        }
        if rel
            .split_ascii_whitespace()
            .any(|r| r == "icon" || r == "apple-touch-icon")
            && !href.is_empty()
        {
            if let Ok(url) = base.join(&href) {
                if url.scheme() == "https" && !links.contains(&url) {
                    links.push(url);
                }
            }
        }
        if links.len() == 4 {
            break;
        }
    }
    links
}

async fn discover(domain: &str) -> Option<Icon> {
    let base = Url::parse(&format!("https://{domain}/")).ok()?;
    let mut candidates = Vec::new();
    if let Some((url, mime, html)) = fetch(base.clone()).await {
        if mime == "text/html" {
            candidates = icon_links(&String::from_utf8_lossy(&html), &url);
        }
    }
    candidates.push(base.join("favicon.ico").ok()?);
    for url in candidates {
        if let Some((_, mime, data)) = fetch(url).await {
            if matches!(
                mime.as_str(),
                "image/png"
                    | "image/svg+xml"
                    | "image/x-icon"
                    | "image/vnd.microsoft.icon"
                    | "image/jpeg"
                    | "image/webp"
                    | "image/gif"
            ) && !data.is_empty()
            {
                return Some((mime, data));
            }
        }
    }
    None
}

fn response(icon: Icon) -> Response {
    if icon.1.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut response = ([(header::CONTENT_TYPE, icon.0)], icon.1).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "private, max-age=3600".parse().unwrap(),
    );
    response
}

pub(crate) async fn show(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    match session(&app, &headers).await {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(_) => return unavailable(),
    }
    let cached = app
        .db
        .read(move |c| {
            let domain: Option<String> = c
                .query_row("SELECT domain FROM sites WHERE id=?1", [id], |r| r.get(0))
                .optional()?;
            let icon: Option<(String, Vec<u8>, i64)> = c
                .query_row(
                    "SELECT content_type,data,fetched_at FROM site_favicons WHERE site_id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            Ok((domain, icon))
        })
        .await;
    let (domain, cached) = match cached {
        Ok((Some(domain), icon)) => (domain, icon),
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return unavailable(),
    };
    if let Some((mime, data, fetched)) = &cached {
        if *fetched > auth::now() - if data.is_empty() { 3600 } else { 7 * 86400 } {
            return response((mime.clone(), data.clone()));
        }
    }
    let Ok(_permit) = app.favicon_fetch.clone().try_acquire_owned() else {
        return cached.map(|(m, d, _)| response((m, d))).unwrap_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::RETRY_AFTER, "2")],
            )
                .into_response()
        });
    };
    let found = tokio::time::timeout(Duration::from_secs(12), discover(&domain))
        .await
        .ok()
        .flatten();
    let icon = found
        .or_else(|| cached.map(|(m, d, _)| (m, d)))
        .unwrap_or_default();
    let stored = icon.clone();
    let _ = app.db.call(move |c| c.execute("INSERT INTO site_favicons VALUES(?1,?2,?3,?4) ON CONFLICT(site_id) DO UPDATE SET content_type=excluded.content_type,data=excluded.data,fetched_at=excluded.fetched_at", rusqlite::params![id,stored.0,stored.1,auth::now()])).await;
    response(icon)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_public_addresses_are_fetchable() {
        for value in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.168.1.1",
            "0.0.0.0",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_ip(value.parse().unwrap()), "{value}");
        }
        for value in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_ip(value.parse().unwrap()), "{value}");
        }
    }
    #[test]
    fn discovers_relative_icons_without_consuming_other_links() {
        let base = Url::parse("https://example.com/docs/").unwrap();
        let links = icon_links(
            "<link rel=stylesheet href=/style.css><LINK href='../icon.png?v=1&amp;x=2' sizes='32x32' REL='shortcut ICON'><link rel=icon href='javascript:alert(1)'><link rel='icon' href=//cdn.example/icon.svg>",
            &base,
        );
        assert_eq!(
            links.iter().map(Url::as_str).collect::<Vec<_>>(),
            [
                "https://example.com/icon.png?v=1&x=2",
                "https://cdn.example/icon.svg"
            ]
        );
    }
}
