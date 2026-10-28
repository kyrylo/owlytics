//! First-touch acquisition, independently implemented from the documented
//! source/channel behavior. Only allowlisted campaign labels reach storage.
use serde::{Deserialize, Serialize};

pub const DIMENSIONS: &[(&str, &str)] = &[
    ("channel", "Channel"),
    ("utm_medium", "UTM medium"),
    ("utm_source", "UTM source"),
    ("utm_campaign", "UTM campaign"),
    ("utm_content", "UTM content"),
    ("utm_term", "UTM term"),
];

pub fn is_dimension(key: &str) -> bool {
    DIMENSIONS.iter().any(|(name, _)| *name == key)
}

pub fn is_campaign(key: &str) -> bool {
    key != "channel" && is_dimension(key)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClickId {
    Gclid,
    Msclkid,
}
impl ClickId {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gclid => "gclid",
            Self::Msclkid => "msclkid",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Campaign {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_id: Option<ClickId>,
}
impl Campaign {
    pub fn valid(&self) -> bool {
        [
            &self.source,
            &self.medium,
            &self.name,
            &self.content,
            &self.term,
        ]
        .into_iter()
        .flatten()
        .all(|value| value.len() <= 2048 && !value.chars().any(char::is_control))
            && serde_json::to_vec(self).is_ok_and(|encoded| encoded.len() <= 2048)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Search,
    Social,
    Video,
    Shopping,
    Email,
    Ai,
}
struct Source {
    name: &'static str,
    kind: Kind,
    domains: &'static [&'static str],
    aliases: &'static [&'static str],
}
// Domain and product names are facts, not a copied referrer database. Keep
// host suffixes on label boundaries so lookalike domains remain distinct.
const SOURCES: &[Source] = &[
    Source {
        name: "Google",
        kind: Kind::Search,
        domains: &[
            "google.com",
            "google.co.uk",
            "google.de",
            "google.fr",
            "google.co.in",
            "google.com.au",
            "google.ca",
            "google.co.jp",
            "google.es",
            "google.it",
            "google.com.br",
            "google.com.ph",
            "google.nl",
            "google.pl",
            "google.ru",
            "google.ch",
            "google.at",
            "google.be",
            "google.se",
            "google.no",
            "google.dk",
            "google.fi",
            "google.ie",
            "google.co.nz",
            "google.com.sg",
            "google.co.id",
            "google.co.za",
            "google.com.mx",
            "google.com.tr",
            "google.pt",
            "com.google.android.googlequicksearchbox",
        ],
        aliases: &["adwords"],
    },
    Source {
        name: "Bing",
        kind: Kind::Search,
        domains: &["bing.com"],
        aliases: &[],
    },
    Source {
        name: "DuckDuckGo",
        kind: Kind::Search,
        domains: &["duckduckgo.com"],
        aliases: &["ddg"],
    },
    Source {
        name: "Yahoo!",
        kind: Kind::Search,
        domains: &["search.yahoo.com"],
        aliases: &["yahoo"],
    },
    Source {
        name: "Brave",
        kind: Kind::Search,
        domains: &["search.brave.com"],
        aliases: &[],
    },
    Source {
        name: "Ecosia",
        kind: Kind::Search,
        domains: &["ecosia.org"],
        aliases: &[],
    },
    Source {
        name: "Yandex",
        kind: Kind::Search,
        domains: &["yandex.com", "yandex.ru"],
        aliases: &[],
    },
    Source {
        name: "Baidu",
        kind: Kind::Search,
        domains: &["baidu.com"],
        aliases: &[],
    },
    Source {
        name: "Qwant",
        kind: Kind::Search,
        domains: &["qwant.com"],
        aliases: &[],
    },
    Source {
        name: "Startpage",
        kind: Kind::Search,
        domains: &["startpage.com"],
        aliases: &[],
    },
    Source {
        name: "Sogou",
        kind: Kind::Search,
        domains: &["sogou.com"],
        aliases: &[],
    },
    Source {
        name: "ChatGPT",
        kind: Kind::Ai,
        domains: &["chatgpt.com", "chat.openai.com", "com.openai.chatgpt"],
        aliases: &["openai"],
    },
    Source {
        name: "Claude",
        kind: Kind::Ai,
        domains: &["claude.ai"],
        aliases: &["anthropic"],
    },
    Source {
        name: "Perplexity",
        kind: Kind::Ai,
        domains: &["perplexity.ai"],
        aliases: &[],
    },
    Source {
        name: "Gemini",
        kind: Kind::Ai,
        domains: &["gemini.google.com", "bard.google.com"],
        aliases: &["bard"],
    },
    Source {
        name: "Copilot",
        kind: Kind::Ai,
        domains: &["copilot.microsoft.com"],
        aliases: &["microsoft copilot"],
    },
    Source {
        name: "Grok",
        kind: Kind::Ai,
        domains: &["grok.com"],
        aliases: &[],
    },
    Source {
        name: "DeepSeek",
        kind: Kind::Ai,
        domains: &["chat.deepseek.com"],
        aliases: &[],
    },
    Source {
        name: "Facebook",
        kind: Kind::Social,
        domains: &[
            "facebook.com",
            "fb.com",
            "fb.me",
            "com.facebook.katana",
            "com.facebook.lite",
        ],
        aliases: &["fb"],
    },
    Source {
        name: "Twitter",
        kind: Kind::Social,
        domains: &["twitter.com", "x.com", "t.co", "com.twitter.android"],
        aliases: &["x"],
    },
    Source {
        name: "Instagram",
        kind: Kind::Social,
        domains: &["instagram.com", "com.instagram.android"],
        aliases: &["ig"],
    },
    Source {
        name: "LinkedIn",
        kind: Kind::Social,
        domains: &["linkedin.com", "lnkd.in", "com.linkedin.android"],
        aliases: &[],
    },
    Source {
        name: "Reddit",
        kind: Kind::Social,
        domains: &["reddit.com", "redd.it", "com.reddit.frontpage"],
        aliases: &[],
    },
    Source {
        name: "GitHub",
        kind: Kind::Social,
        domains: &["github.com"],
        aliases: &[],
    },
    Source {
        name: "Hacker News",
        kind: Kind::Social,
        domains: &["news.ycombinator.com"],
        aliases: &["hn", "hackernews"],
    },
    Source {
        name: "Product Hunt",
        kind: Kind::Social,
        domains: &["producthunt.com"],
        aliases: &["producthunt"],
    },
    Source {
        name: "Bluesky",
        kind: Kind::Social,
        domains: &["bsky.app", "bsky.social"],
        aliases: &["bsky"],
    },
    Source {
        name: "Threads",
        kind: Kind::Social,
        domains: &["threads.net", "threads.com"],
        aliases: &[],
    },
    Source {
        name: "Pinterest",
        kind: Kind::Social,
        domains: &["pinterest.com", "pin.it"],
        aliases: &[],
    },
    Source {
        name: "Telegram",
        kind: Kind::Social,
        domains: &["t.me", "telegram.org", "org.telegram.messenger"],
        aliases: &[],
    },
    Source {
        name: "Discord",
        kind: Kind::Social,
        domains: &["discord.com", "discord.gg"],
        aliases: &[],
    },
    Source {
        name: "Slack",
        kind: Kind::Social,
        domains: &["slack.com"],
        aliases: &[],
    },
    Source {
        name: "WhatsApp",
        kind: Kind::Social,
        domains: &["whatsapp.com", "wa.me", "com.whatsapp"],
        aliases: &[],
    },
    Source {
        name: "Microsoft Teams",
        kind: Kind::Social,
        domains: &["teams.microsoft.com", "teams.live.com"],
        aliases: &["teams"],
    },
    Source {
        name: "YouTube",
        kind: Kind::Video,
        domains: &["youtube.com", "youtu.be", "com.google.android.youtube"],
        aliases: &["yt"],
    },
    Source {
        name: "TikTok",
        kind: Kind::Video,
        domains: &["tiktok.com"],
        aliases: &[],
    },
    Source {
        name: "Vimeo",
        kind: Kind::Video,
        domains: &["vimeo.com"],
        aliases: &[],
    },
    Source {
        name: "Twitch",
        kind: Kind::Video,
        domains: &["twitch.tv"],
        aliases: &[],
    },
    Source {
        name: "Amazon",
        kind: Kind::Shopping,
        domains: &[
            "amazon.com",
            "amazon.co.uk",
            "amazon.de",
            "amazon.ca",
            "amazon.co.jp",
        ],
        aliases: &[],
    },
    Source {
        name: "eBay",
        kind: Kind::Shopping,
        domains: &["ebay.com", "ebay.co.uk", "ebay.de"],
        aliases: &[],
    },
    Source {
        name: "Etsy",
        kind: Kind::Shopping,
        domains: &["etsy.com"],
        aliases: &[],
    },
    Source {
        name: "Gmail",
        kind: Kind::Email,
        domains: &["mail.google.com", "com.google.android.gm"],
        aliases: &[],
    },
];

fn paid_alias(value: &str, alias: &str) -> bool {
    if ["ads", "ad"].iter().any(|ending| {
        value
            .strip_suffix(ending)
            .is_some_and(|base| base.eq_ignore_ascii_case(alias))
    }) {
        return true;
    }
    value
        .get(..alias.len())
        .filter(|prefix| prefix.eq_ignore_ascii_case(alias))
        .is_some_and(|_| {
            let suffix = &value[alias.len()..];
            suffix.starts_with(['-', '_', ' '])
                && matches!(suffix.rsplit(['-', '_', ' ']).next(), Some("ad" | "ads"))
        })
}

fn known(value: &str) -> Option<(&'static Source, bool)> {
    if value.is_empty() {
        return None;
    }
    let value = value.to_ascii_lowercase();
    let host = value
        .strip_prefix("android-app://")
        .unwrap_or(&value)
        .trim_end_matches('.');
    // Choose the most specific domain (mail.google.com before google.com).
    if let Some((source, _)) = SOURCES
        .iter()
        .flat_map(|s| s.domains.iter().map(move |domain| (s, *domain)))
        .filter(|(_, domain)| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
        .max_by_key(|(_, domain)| domain.len())
    {
        return Some((source, false));
    }
    for source in SOURCES {
        for alias in std::iter::once(&source.name).chain(source.aliases.iter()) {
            if value.eq_ignore_ascii_case(alias) {
                return Some((source, value == "adwords"));
            }
            if paid_alias(&value, alias) {
                return Some((source, true));
            }
        }
    }
    None
}

pub fn source_name(value: &str) -> String {
    known(value)
        .map_or(value, |(source, _)| source.name)
        .to_owned()
}

#[test]
fn source_catalog_icons_follow_domain_and_campaign_aliases() {
    for source in SOURCES {
        let expected = crate::icons::report("source", source.name);
        assert!(!expected.contains("site-monogram"), "{}", source.name);
        for alias in source.domains.iter().chain(source.aliases.iter()) {
            assert_eq!(crate::icons::report("source", alias), expected, "{alias}");
        }
    }
}

fn shopping(campaign: &str) -> bool {
    campaign.contains("shopping")
        || campaign.match_indices("shop").any(|(index, _)| {
            index == 0
                || campaign[..index]
                    .chars()
                    .next_back()
                    .is_some_and(|c| !c.is_ascii_lowercase() || c == 'e')
        })
}

/// Source is the display label; raw_source is the original, case-sensitive tag.
/// Inferred click media are labels only, never the identifier's value.
pub fn channel(source: &str, raw_source: &str, medium: &str, campaign: &str) -> &'static str {
    classify(source, raw_source, medium, campaign, None)
}

fn classify(
    source: &str,
    raw_source: &str,
    medium: &str,
    campaign: &str,
    click: Option<ClickId>,
) -> &'static str {
    let category = known(source).map(|(source, _)| source.kind);
    let ad_alias = known(raw_source).is_some_and(|(_, paid)| paid);
    let raw = raw_source.to_ascii_lowercase();
    let medium = medium.to_ascii_lowercase();
    let campaign = campaign.to_ascii_lowercase();
    let paid = medium.contains("cp")
        || matches!(medium.as_str(), "ppc" | "retargeting")
        || medium.starts_with("paid");
    let shop = category == Some(Kind::Shopping) || shopping(&campaign);
    if medium == "cross-network" || campaign.contains("cross-network") {
        return "Cross-network";
    }
    if shop && paid {
        return "Paid Shopping";
    }
    if category == Some(Kind::Search)
        && (paid
            || ad_alias
            || (source == "Google" && (medium == "(gclid)" || click == Some(ClickId::Gclid)))
            || (source == "Bing" && (medium == "(msclkid)" || click == Some(ClickId::Msclkid))))
    {
        return "Paid Search";
    }
    if category == Some(Kind::Social) && (paid || ad_alias) {
        return "Paid Social";
    }
    if category == Some(Kind::Video) && (paid || ad_alias) {
        return "Paid Video";
    }
    if ["display", "banner", "expandable", "interstitial", "cpm"].contains(&medium.as_str()) {
        return "Display";
    }
    if paid {
        return "Paid Other";
    }
    if shop {
        return "Organic Shopping";
    }
    if category == Some(Kind::Social)
        || [
            "social",
            "social-network",
            "social-media",
            "sm",
            "social network",
            "social media",
        ]
        .contains(&medium.as_str())
    {
        return "Organic Social";
    }
    if category == Some(Kind::Video) || medium.contains("video") {
        return "Organic Video";
    }
    if category == Some(Kind::Ai) {
        return "AI Assistants";
    }
    if category == Some(Kind::Search) || medium == "organic" {
        return "Organic Search";
    }
    if category == Some(Kind::Email)
        || ["email", "e-mail", "e_mail", "e mail", "newsletter"]
            .iter()
            .any(|label| raw.contains(label) || medium.contains(label))
    {
        return "Email";
    }
    if medium == "affiliate" {
        return "Affiliates";
    }
    if medium == "audio" {
        return "Audio";
    }
    if raw == "sms" || medium == "sms" {
        return "SMS";
    }
    if raw == "firebase"
        || medium.ends_with("push")
        || medium.contains("mobile")
        || medium.contains("notification")
    {
        return "Mobile Push Notifications";
    }
    if !source.is_empty() || ["referral", "app", "link"].contains(&medium.as_str()) {
        return "Referral";
    }
    "Direct"
}

pub struct Attribution {
    pub source: String,
    pub channel: &'static str,
    pub utm_source: String,
    pub utm_medium: String,
    pub utm_campaign: String,
    pub utm_content: String,
    pub utm_term: String,
    pub click_id: Option<&'static str>,
}
impl Attribution {
    pub fn resolve(referrer_source: &str, tags: &Campaign) -> Self {
        let source = source_name(tags.source.as_deref().unwrap_or(referrer_source));
        let utm_source = tags.source.clone().unwrap_or_default();
        let utm_medium =
            tags.medium
                .clone()
                .unwrap_or_else(|| match (source.as_str(), tags.click_id) {
                    ("Google", Some(ClickId::Gclid)) => "(gclid)".into(),
                    ("Bing", Some(ClickId::Msclkid)) => "(msclkid)".into(),
                    _ => String::new(),
                });
        let utm_campaign = tags.name.clone().unwrap_or_default();
        Self {
            channel: classify(
                &source,
                &utm_source,
                &utm_medium,
                &utm_campaign,
                tags.click_id,
            ),
            source,
            utm_source,
            utm_medium,
            utm_campaign,
            utm_content: tags.content.clone().unwrap_or_default(),
            utm_term: tags.term.clone().unwrap_or_default(),
            click_id: tags.click_id.map(ClickId::label),
        }
    }
}
