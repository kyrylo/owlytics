//! Local collection filters. No visitor data or requests leave the process.
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

fn crawlers() -> &'static regex::bytes::RegexSet {
    static PATTERNS: OnceLock<regex::bytes::RegexSet> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        regex::bytes::RegexSetBuilder::new(
            include_str!("../assets/bot-lists/crawlers.txt")
                .lines()
                .chain([
                    // Generic clients and automation that needn't advertise a
                    // named crawler. Avoid matching phone models like CUBOT.
                    r"\bbot\b|spider|crawler|headless|lighthouse|pagespeed",
                    r"curl/|wget/|python-|go-http-client/|libwww-perl|^java/",
                    r"phantomjs|selenium|playwright|puppeteer|^node(?:-fetch)?$|axios/|^undici$",
                    r"facebookexternalhit|facebookcatalog|httpunit|apache-httpclient|okhttp/",
                ])
                .map(|pattern| {
                    // Upstream often requires a version separator. A bare
                    // crawler name must also be filtered (e.g. "Googlebot").
                    pattern
                        .strip_suffix(r"\/")
                        .map_or_else(|| pattern.to_owned(), |name| format!("{name}(?:/|$)"))
                }),
        )
        .case_insensitive(true)
        .unicode(false)
        .build()
        .expect("validated bundled crawler patterns")
    })
}

pub(crate) fn user_agent(value: &str) -> bool {
    value.trim().is_empty() || value.len() > 1024 || crawlers().is_match(value.as_bytes())
}

pub(crate) fn referrer(value: &str) -> bool {
    static DOMAINS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let domains = DOMAINS.get_or_init(|| {
        include_str!("../assets/bot-lists/referrers.txt")
            .lines()
            .collect()
    });
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    // URL parsing handles case and IDNs. Match whole labels, never a query,
    // username, path or a lookalike such as not-spammer.com.
    let mut suffix = host.trim_end_matches('.');
    loop {
        if domains.contains(suffix) {
            return true;
        }
        let Some((_, rest)) = suffix.split_once('.') else {
            return false;
        };
        suffix = rest;
    }
}

#[derive(Default)]
struct Networks {
    v4: Vec<(u128, u128)>,
    v6: Vec<(u128, u128)>,
}

impl Networks {
    fn parse(input: &str) -> Self {
        let mut result = Self::default();
        for line in input.lines().filter(|line| !line.is_empty()) {
            match line
                .parse::<ipnet::IpNet>()
                .expect("validated hosting CIDR")
            {
                ipnet::IpNet::V4(net) => result.v4.push((
                    u32::from(net.network()) as u128,
                    u32::from(net.broadcast()) as u128,
                )),
                ipnet::IpNet::V6(net) => result
                    .v6
                    .push((u128::from(net.network()), u128::from(net.broadcast()))),
            }
        }
        // Merge overlaps and nested networks so a predecessor lookup cannot
        // miss the end of an enclosing range. This also keeps requests O(log n).
        for ranges in [&mut result.v4, &mut result.v6] {
            ranges.sort_unstable();
            let mut merged: Vec<(u128, u128)> = Vec::new();
            for &(start, end) in ranges.iter() {
                if let Some(last) = merged.last_mut()
                    && start <= last.1.saturating_add(1)
                {
                    last.1 = last.1.max(end);
                } else {
                    merged.push((start, end));
                }
            }
            *ranges = merged;
        }
        result
    }

    fn contains(&self, ip: IpAddr) -> bool {
        let (ranges, value) = match ip.to_canonical() {
            IpAddr::V4(ip) => (&self.v4, u32::from(ip) as u128),
            IpAddr::V6(ip) => (&self.v6, u128::from(ip)),
        };
        let index = ranges.partition_point(|&(start, _)| start <= value);
        index > 0 && value <= ranges[index - 1].1
    }
}

fn hosting() -> &'static Networks {
    static NETWORKS: OnceLock<Networks> = OnceLock::new();
    NETWORKS.get_or_init(|| Networks::parse(include_str!("../assets/bot-lists/hosting.txt")))
}

const WINDOW: Duration = Duration::from_secs(60);
const MAX_IDENTITIES: usize = 20_000;
const MAX_PAGEVIEWS: usize = 120;
const MAX_REQUESTS: usize = 600;

#[derive(Default)]
struct Activity {
    requests: usize,
    pages: HashSet<[u8; 16]>,
    blocked: bool,
}

struct BurstFilter {
    started: Instant,
    secret: [u8; 32],
    identities: HashMap<[u8; 32], Activity>,
}

impl BurstFilter {
    fn new() -> Self {
        let mut secret = [0; 32];
        getrandom::fill(&mut secret).expect("OS randomness for ephemeral bot filtering");
        Self {
            started: Instant::now(),
            secret,
            identities: HashMap::new(),
        }
    }

    fn blocked(
        &mut self,
        now: Instant,
        domain: &str,
        ip: IpAddr,
        ua: &str,
        page_id: Option<&str>,
    ) -> bool {
        if now.duration_since(self.started) >= WINDOW {
            self.identities.clear();
            // Erase the previous minute's key along with its counters. Nothing
            // in this filter links visitors across windows or survives restart.
            self.secret = Sha256::digest(self.secret).into();
            self.started = now;
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret).expect("HMAC key");
        for part in [
            domain.as_bytes(),
            ip.to_canonical().to_string().as_bytes(),
            ua.as_bytes(),
        ] {
            mac.update(&(part.len() as u64).to_be_bytes());
            mac.update(part);
        }
        let key: [u8; 32] = mac.finalize().into_bytes().into();
        if !self.identities.contains_key(&key) && self.identities.len() >= MAX_IDENTITIES {
            // Bound memory without allowing an attacker to evict existing
            // counters or suppress unrelated newcomers. Static filters remain.
            return false;
        }
        let activity = self.identities.entry(key).or_default();
        if activity.blocked {
            return true;
        }
        activity.requests += 1;
        if activity.requests > MAX_REQUESTS {
            activity.blocked = true;
        } else if let Some(id) = page_id {
            let mut id_bytes = [0; 16];
            if hex::decode_to_slice(id, &mut id_bytes).is_ok()
                && !activity.pages.contains(&id_bytes)
            {
                if activity.pages.len() == MAX_PAGEVIEWS {
                    activity.blocked = true;
                } else {
                    activity.pages.insert(id_bytes);
                }
            }
        }
        activity.blocked
    }
}

pub(crate) struct Filter {
    burst: Mutex<BurstFilter>,
}

impl Default for Filter {
    fn default() -> Self {
        // Compile once during startup, not on the first visitor's request.
        crawlers();
        hosting();
        Self {
            burst: Mutex::new(BurstFilter::new()),
        }
    }
}

impl Filter {
    pub(crate) fn reason(
        &self,
        config: &crate::config::Config,
        domain: &str,
        ip: IpAddr,
        ua: &str,
        page: Option<(&str, &str)>,
    ) -> Option<&'static str> {
        if user_agent(ua) {
            return Some("user-agent");
        }
        if config.filter_datacenters
            && !config
                .bot_allowed_ips
                .iter()
                .any(|net| net.contains(&ip.to_canonical()))
            && hosting().contains(ip)
        {
            return Some("datacenter");
        }
        if page.is_some_and(|(_, referring)| referrer(referring)) {
            return Some("referrer-spam");
        }
        if self.burst.lock().expect("bot filter mutex").blocked(
            Instant::now(),
            domain,
            ip,
            ua,
            page.map(|(id, _)| id),
        ) {
            return Some("rate-limit");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_crawlers_and_clients_without_rejecting_real_browsers() {
        for ua in [
            "",
            "   ",
            "Googlebot/2.1",
            "Googlebot",
            "PerplexityBot",
            "Diffbot",
            "BLEXBot",
            "bingbot/2.0",
            "GPTBot/1.0",
            "ClaudeBot/1.0",
            "Claude-User/1.0",
            "PerplexityBot/1.0",
            "ChatGPT-User/1.0",
            "Bytespider",
            "Amazonbot/0.1",
            "Applebot/0.1",
            "facebookexternalhit/1.1",
            "Twitterbot/1.0",
            "Mozilla/5.0 HeadlessChrome/130.0",
            "Mozilla/5.0 (compatible; Pingdom.com_bot_version_1.4;)",
            "Go-http-client/1.1",
            "curl/8.0",
            "python-requests/2.0",
            "Java/17",
            "axios/1.7",
            "node",
            "undici",
        ] {
            assert!(user_agent(ua), "Missed {ua}");
        }
        for ua in [
            "Mozilla/5.0 (Linux; Android 12; CUBOT P80) AppleWebKit/537.36 Chrome/130.0.0.0 Mobile Safari/537.36",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) Gecko/20100101 Firefox/132.0",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1",
            "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
            "Mozilla/5.0 (Linux; Android 14) SamsungBrowser/27.0 Chrome/130.0 Mobile Safari/537.36",
            "Mozilla/5.0 Chrome/130.0.0.0 Mobile Safari/537.36 [FBAN/FB4A;FBAV/490.0]",
        ] {
            assert!(!user_agent(ua), "Rejected {ua}");
        }
    }

    #[test]
    fn spam_matches_hosts_and_subdomains_only() {
        for url in ["https://semalt.com/", "https://sub.SEMALT.com./path?q=x"] {
            assert!(referrer(url), "{url}");
        }
        for url in [
            "",
            "invalid",
            "https://notsemalt.com/",
            "https://semalt.com.example.org/",
            "https://example.org/semalt.com",
            "https://example.org/?q=semalt.com",
            "https://semalt.com@example.org/",
            "https://chatgpt.com/",
            "https://claude.ai/",
        ] {
            assert!(!referrer(url), "{url}");
        }
    }

    #[test]
    fn networks_cover_both_families_overlaps_and_mapped_addresses() {
        let nets =
            Networks::parse("203.0.113.0/24\n203.0.113.64/27\n2001:db8::/32\n2001:db8:1::/48\n");
        for ip in [
            "203.0.113.0",
            "203.0.113.255",
            "::ffff:203.0.113.255",
            "2001:db8::",
            "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff",
        ] {
            assert!(nets.contains(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "203.0.112.255",
            "203.0.114.0",
            "2001:db7:ffff::",
            "2001:db9::",
            "::1",
        ] {
            assert!(!nets.contains(ip.parse().unwrap()), "{ip}");
        }
        assert!(hosting().contains("165.227.0.1".parse().unwrap()));
        assert!(hosting().contains("2604:a880::1".parse().unwrap()));
        assert!(!hosting().contains("203.0.113.1".parse().unwrap()));
    }

    #[test]
    fn bursts_are_scoped_expire_and_do_not_count_retries_as_pageviews() {
        let mut filter = BurstFilter::new();
        let now = filter.started;
        let ip = "203.0.113.1".parse().unwrap();
        for i in 0..MAX_PAGEVIEWS {
            let id = format!("{i:032x}");
            for _ in 0..2 {
                assert!(!filter.blocked(now, "example.com", ip, "browser", Some(&id)));
            }
        }
        assert!(filter.blocked(
            now,
            "example.com",
            ip,
            "browser",
            Some(&format!("{:032x}", MAX_PAGEVIEWS))
        ));
        assert!(filter.blocked(now, "example.com", ip, "browser", None));
        assert!(!filter.blocked(now, "other.com", ip, "browser", None));
        assert!(!filter.blocked(now, "example.com", ip, "another browser", None));
        assert!(!filter.blocked(
            now,
            "example.com",
            "203.0.113.2".parse().unwrap(),
            "browser",
            None
        ));
        let secret = filter.secret;
        assert!(!filter.blocked(now + WINDOW, "example.com", ip, "browser", None));
        assert_ne!(secret, filter.secret);
        assert_eq!(filter.identities.len(), 1);
        for _ in 1..MAX_REQUESTS {
            assert!(!filter.blocked(now + WINDOW, "example.com", ip, "browser", None));
        }
        assert!(filter.blocked(now + WINDOW, "example.com", ip, "browser", None));
    }

    #[test]
    fn identity_floods_have_bounded_memory_and_do_not_block_new_visitors() {
        let mut filter = BurstFilter::new();
        let now = filter.started;
        let ip = "203.0.113.1".parse().unwrap();
        for i in 0..MAX_IDENTITIES + 10 {
            assert!(!filter.blocked(now, "example.com", ip, &i.to_string(), None));
        }
        assert_eq!(filter.identities.len(), MAX_IDENTITIES);
    }
}
