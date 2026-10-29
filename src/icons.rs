// Brand paths are bundled from Simple Icons under CC0. See docs/licenses/icons.md.
// No icon markup or URL comes from untrusted report data.
pub(crate) fn control(name: &str) -> String {
    let drawing = match name {
        "calendar" => {
            "<rect x='3' y='5' width='18' height='16' rx='2'/><path d='M7 3v4m10-4v4M3 11h18'/>"
        }
        "filter" => "<path d='M3 6h18M6 12h12m-9 6h6'/>",
        "expand" => "<path d='M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5'/>",
        "close" => "<path d='m6 6 12 12M6 18 18 6'/>",
        _ => unreachable!("Unknown dashboard control icon"),
    };
    format!(
        "<svg class=\"control-icon\" width=\"16\" height=\"16\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">{drawing}</svg>"
    )
}

macro_rules! brand {
    ($name:literal) => {
        format!(
            "<span class=\"report-icon brand-{}\" aria-hidden=\"true\">{}</span>",
            $name,
            include_str!(concat!("../assets/icons/", $name, ".svg"))
                .replace("<svg ", "<svg focusable=\"false\" ")
                .replace("<path ", "<path fill=\"currentColor\" ")
        )
    };
}

pub(crate) fn report(dimension: &str, name: &str) -> String {
    if dimension == "channel" {
        let drawing = match name {
            "Organic Search" | "Paid Search" => {
                "<circle cx='10' cy='10' r='6'/><path d='m15 15 6 6'/>"
            }
            "Organic Social" | "Paid Social" => {
                "<circle cx='9' cy='8' r='3'/><path d='M3 21v-3a6 6 0 0 1 12 0v3M16 5a3 3 0 0 1 0 6m2 4a5 5 0 0 1 3 4v2'/>"
            }
            "Organic Video" | "Paid Video" => {
                "<rect x='3' y='5' width='18' height='14' rx='3'/><path d='m10 9 5 3-5 3Z'/>"
            }
            "Organic Shopping" | "Paid Shopping" => {
                "<path d='M5 8h14l2 13H3ZM8 8V6a4 4 0 0 1 8 0v2'/>"
            }
            "Email" => "<rect x='3' y='5' width='18' height='14' rx='2'/><path d='m3 6 9 7 9-7'/>",
            "AI Assistants" => {
                "<path d='m10 2 2.5 6.5L19 11l-6.5 2.5L10 20l-2.5-6.5L1 11l6.5-2.5ZM19 2v5m-2.5-2.5h5M20 16v6m-3-3h6'/>"
            }
            "Direct" => "<path d='M4 12h16m-6-6 6 6-6 6'/>",
            "Mobile Push Notifications" => {
                "<path d='M5 17h14l-2-3V9a5 5 0 0 0-10 0v5Zm5 3a2 2 0 0 0 4 0'/>"
            }
            "Audio" => {
                "<path d='M4 14V10a8 8 0 0 1 16 0v4'/><rect x='3' y='12' width='4' height='9' rx='2'/><rect x='17' y='12' width='4' height='9' rx='2'/>"
            }
            "SMS" => {
                "<path d='M5 4h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H9l-5 4v-4a2 2 0 0 1-2-2V7a3 3 0 0 1 3-3Z'/><path d='M7 9h10M7 13h7'/>"
            }
            "Cross-network" => {
                "<circle cx='5' cy='5' r='2'/><circle cx='19' cy='5' r='2'/><circle cx='12' cy='19' r='2'/><path d='M7 5h10M6 7l5 10m7-10-5 10'/>"
            }
            "Affiliates" | "Referral" => {
                "<path d='m9 15 6-6m-5-3 2-2a5 5 0 0 1 7 7l-2 2m-3 5-2 2a5 5 0 0 1-7-7l2-2'/>"
            }
            "Display" => {
                "<rect x='3' y='4' width='18' height='16' rx='2'/><circle cx='8' cy='9' r='1.5'/><path d='m3 17 6-5 4 3 4-5 4 4'/>"
            }
            _ => "<path d='M3 10h5l12-5v14L8 14H3Zm4 4 2 7h4l-2-6M20 8a4 4 0 0 1 0 8'/>",
        };
        return format!(
            "<span class=\"report-icon\" aria-hidden=\"true\"><svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\">{drawing}</svg></span>"
        );
    }
    if matches!(dimension, "country" | "region" | "city") {
        let code = name.get(..2).unwrap_or("");
        if code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()) {
            let flag = code
                .bytes()
                .filter_map(|b| char::from_u32(0x1f1e6 + (b - b'A') as u32))
                .collect::<String>();
            return format!("<span class=\"report-icon flag\" aria-hidden=\"true\">{flag}</span>");
        }
    }
    match dimension {
        "source" | "site" => source(name),
        "referrer" => {
            let address = if name.contains("://") {
                name.to_owned()
            } else {
                format!("https://{name}")
            };
            match url::Url::parse(&address)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
            {
                Some(host) => source(&host),
                None => source(name),
            }
        }
        "browser" => match name {
            "Chrome" | "Chrome Mobile" | "Chrome Mobile iOS" | "Chrome WebView" | "Chromium" => {
                brand!("googlechrome")
            }
            "Safari" | "Mobile Safari" => drawing(
                "safari",
                "<circle cx='12' cy='12' r='10.5' fill='#398ad2'/><circle cx='12' cy='12' r='8.5' fill='none' stroke='white' stroke-dasharray='1 3'/><path d='m16.5 7.5-3 6-3-3Z' fill='#ec5a54'/><path d='m7.5 16.5 3-6 3 3Z' fill='white'/>",
            ),
            "Firefox" | "Firefox Mobile" | "Firefox iOS" => brand!("firefox"),
            "Edge" | "Microsoft Edge" | "Edge Mobile" => brand!("microsoftedge"),
            "Opera" | "Opera Mobile" | "Opera Mini" => brand!("opera"),
            "Samsung Internet" => drawing(
                "samsunginternet",
                "<circle cx='12' cy='12' r='7' fill='currentColor'/><ellipse cx='12' cy='12' rx='11' ry='3.8' transform='rotate(-30 12 12)' fill='none' stroke='currentColor' stroke-width='1.5'/><path d='m5 14 14-8' stroke='white' stroke-width='1.2'/>",
            ),
            "Brave" => brand!("brave"),
            _ => globe(),
        },
        "os" => match name {
            "macOS" | "Mac" | "Mac OS" | "Mac OS X" | "iOS" => brand!("apple"),
            "Linux" | "GNU/Linux" => brand!("linux"),
            "Ubuntu" => brand!("ubuntu"),
            "Chrome OS" | "ChromeOS" => brand!("googlechrome"),
            "Windows" => drawing(
                "windows",
                "<path d='M2 4 11 3v8H2zm11-1 9-1v9h-9zM2 13h9v8l-9-1zm11 0h9v9l-9-1z' fill='currentColor'/>",
            ),
            "Android" => drawing(
                "android",
                "<path d='m7 5-2-3m12 3 2-3M4 12a8 8 0 0 1 16 0v7H4Z' fill='currentColor' stroke='currentColor' stroke-width='1.5'/><circle cx='8' cy='9' r='1' fill='white'/><circle cx='16' cy='9' r='1' fill='white'/>",
            ),
            _ => screen(),
        },
        "device" => match name {
            "Mobile" => drawing(
                "device",
                "<rect x='6' y='2' width='12' height='20' rx='2' stroke='currentColor' stroke-width='1.5'/><path d='M10 18h4' stroke='currentColor'/>",
            ),
            "Tablet" => drawing(
                "device",
                "<rect x='3' y='2' width='18' height='20' rx='2' stroke='currentColor' stroke-width='1.5'/><circle cx='12' cy='19' r='1' fill='currentColor'/>",
            ),
            _ => screen(),
        },
        _ => globe(),
    }
}

fn drawing(class: &str, paths: &str) -> String {
    format!(
        "<span class=\"report-icon brand-{class}\" aria-hidden=\"true\"><svg viewBox=\"0 0 24 24\" fill=\"none\" focusable=\"false\">{paths}</svg></span>"
    )
}

fn globe() -> String {
    drawing(
        "generic",
        "<g stroke='currentColor' stroke-width='1.5'><circle cx='12' cy='12' r='10'/><ellipse cx='12' cy='12' rx='4' ry='10'/><path d='M2 12h20'/></g>",
    )
}

fn screen() -> String {
    drawing(
        "device",
        "<g stroke='currentColor' stroke-width='1.5'><rect x='2' y='3' width='20' height='14' rx='2'/><path d='M12 17v4m-5 0h10'/></g>",
    )
}

// One canonical label drives both acquisition reports and their icons. Rendering
// never changes the stored source or the filter value, including unknown tags.
fn source(name: &str) -> String {
    let label = crate::acquisition::source_name(name);
    match label.as_str() {
        "" | "Direct / None" => drawing(
            "direct",
            "<path d='M3 12h17m-6-6 6 6-6 6' stroke='currentColor' stroke-width='1.7' stroke-linecap='round' stroke-linejoin='round'/>",
        ),
        "Google" => brand!("google"),
        "Bing" => brand!("microsoftbing"),
        "DuckDuckGo" => brand!("duckduckgo"),
        "Yahoo!" => drawing(
            "yahoo",
            "<path d='M2 4h4l4 7 4-7h4l-6 10v6H8v-6ZM20 3h3l-2 12h-2Zm-1 14h3l-.5 3h-3Z' fill='currentColor'/>",
        ),
        "Brave" => brand!("brave"),
        "Ecosia" => brand!("ecosia"),
        "Yandex" => drawing(
            "yandex",
            "<circle cx='12' cy='12' r='11' fill='currentColor'/><path d='M15 5h-3a4 4 0 0 0-1 8l-4 6h3l4-7h-2a2.5 2.5 0 0 1 0-5h1v12h2Z' fill='white'/>",
        ),
        "Baidu" => brand!("baidu"),
        "Qwant" => brand!("qwant"),
        "Startpage" => brand!("startpage"),
        "Sogou" => brand!("sogou"),
        "ChatGPT" => brand!("openai"),
        "Claude" => brand!("claude"),
        "Perplexity" => brand!("perplexity"),
        "Gemini" => brand!("googlegemini"),
        "Copilot" => drawing(
            "copilot",
            "<path d='M4 5a3 3 0 0 1 3-3h8l-4 8H2Z' fill='#35aca7'/><path d='M13 2h4a3 3 0 0 1 3 3l2 9h-9l-3-7Z' fill='#8478cf'/><path d='M22 14a6 6 0 0 1-6 8H8l4-8Z' fill='#e08d6e'/><path d='M2 10h9L8 22H6a3 3 0 0 1-3-3Z' fill='#518dd6'/>",
        ),
        "Grok" => drawing(
            "grok",
            "<path d='M18 4a9 9 0 1 0 2 13M7 17 21 3M12 12l8 8' stroke='currentColor' stroke-width='2.4'/>",
        ),
        "DeepSeek" => brand!("deepseek"),
        "Facebook" => brand!("facebook"),
        "Twitter" => brand!("x"),
        "Instagram" => brand!("instagram"),
        "LinkedIn" => brand!("linkedin"),
        "Reddit" => brand!("reddit"),
        "GitHub" => brand!("github"),
        "Hacker News" => brand!("ycombinator"),
        "Product Hunt" => brand!("producthunt"),
        "Bluesky" => brand!("bluesky"),
        "Threads" => brand!("threads"),
        "Pinterest" => brand!("pinterest"),
        "Telegram" => brand!("telegram"),
        "Discord" => brand!("discord"),
        "Slack" => brand!("slack"),
        "WhatsApp" => brand!("whatsapp"),
        "Microsoft Teams" => brand!("microsoftteams"),
        "YouTube" => brand!("youtube"),
        "TikTok" => brand!("tiktok"),
        "Vimeo" => brand!("vimeo"),
        "Twitch" => brand!("twitch"),
        "Amazon" => brand!("amazon"),
        "eBay" => brand!("ebay"),
        "Etsy" => brand!("etsy"),
        "Gmail" => brand!("gmail"),
        _ => monogram(name),
    }
}

fn monogram(name: &str) -> String {
    let name = name.strip_prefix("www.").unwrap_or(name);
    let letter = name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?');
    let letter = letter.to_uppercase().next().unwrap_or(letter);
    let hash = name.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31)
            .wrapping_add(byte.to_ascii_lowercase() as u32)
    });
    format!(
        "<span class=\"report-icon site-monogram monogram-{}\" aria-hidden=\"true\"><svg viewBox=\"0 0 24 24\" focusable=\"false\"><rect x=\"1\" y=\"1\" width=\"22\" height=\"22\" rx=\"5\" fill=\"currentColor\" opacity=\".13\"/><text x=\"12\" y=\"17\" text-anchor=\"middle\" fill=\"currentColor\">{}</text></svg></span>",
        hash % 6,
        crate::web::escape(&letter.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::report;

    #[test]
    fn referrer_icons_use_the_host_and_respect_domain_boundaries() {
        for address in [
            "github.com/article",
            "https://github.com/article?q=private#private",
            "https://www.github.com/article",
        ] {
            assert_eq!(report("referrer", address), report("source", "GitHub"));
        }
        assert_eq!(report("source", "FB-Ads"), report("source", "Facebook"));
        assert_eq!(
            report(
                "referrer",
                "android-app://com.google.android.googlequicksearchbox/search"
            ),
            report("source", "Google")
        );
        for address in [
            "google.com.evil.example/path",
            "https://google.com@evil.example/path",
            "notgoogle.com",
        ] {
            assert!(
                !report("referrer", address).contains("brand-google"),
                "{address}"
            );
        }
        assert_eq!(
            report("site", "www.example.com"),
            report("site", "example.com")
        );
    }

    #[test]
    fn untrusted_names_cannot_add_markup_or_external_requests() {
        for name in [
            "<svg onload=alert(1)>",
            "\"/><img src=//evil.example>",
            "猫の日記",
            "🦉",
            "",
            "a&b",
            "https://secret:password@example.com/path",
        ] {
            let icon = report("source", name);
            for forbidden in [
                "onload", "<img", "<script", "href=", "src=", "password", "style=",
            ] {
                assert!(!icon.contains(forbidden), "{name}: {icon}");
            }
            assert!(icon.contains("aria-hidden=\"true\""));
        }
        assert!(report("source", "").contains("brand-direct"));
        assert!(report("browser", "").contains("brand-generic"));
        assert!(report("browser", "Brave").contains("brand-brave"));
        assert!(report("os", "Chrome OS").contains("brand-googlechrome"));
        assert!(!report("os", "Google").contains("brand-google"));
        assert!(!report("source", "Chrome").contains("brand-googlechrome"));
    }
}

pub(crate) fn site(id: i64, domain: &str) -> String {
    format!(
        "<span class=\"site-favicon\">{}<img data-favicon=\"/sites/{id}/favicon\" width=\"20\" height=\"20\" alt=\"\" hidden></span>",
        source(domain)
    )
}
