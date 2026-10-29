// Locally bundled brand artwork. See docs/licenses/icons.md.
use std::sync::LazyLock;

pub(crate) struct Asset {
    pub name: &'static str,
    pub mime: &'static str,
    pub body: &'static [u8],
    pub url: String,
}

macro_rules! svg {
    ($name:literal) => {
        (
            $name,
            "image/svg+xml",
            include_bytes!(concat!("../assets/brands/", $name, ".svg")).as_slice(),
            "svg",
        )
    };
}

pub(crate) fn assets() -> &'static [Asset] {
    static ASSETS: LazyLock<Vec<Asset>> = LazyLock::new(|| {
        use sha2::{Digest, Sha256};
        [
            svg!("googlechrome"),
            svg!("chromium"),
            svg!("firefox"),
            svg!("microsoftedge"),
            svg!("safari"),
            svg!("opera"),
            svg!("samsunginternet"),
            svg!("brave"),
            svg!("microsoftbing"),
            svg!("duckduckgo"),
            svg!("gmail"),
            svg!("googlegemini"),
            svg!("instagram"),
            svg!("slack"),
            svg!("tiktok"),
            svg!("linux"),
            svg!("android"),
            svg!("microsoftteams"),
            svg!("reddit"),
            svg!("whatsapp"),
            svg!("facebook"),
            svg!("linkedin"),
            svg!("youtube"),
            svg!("claude"),
            svg!("perplexity"),
            (
                "google",
                "image/x-icon",
                include_bytes!("../assets/brands/google.ico").as_slice(),
                "ico",
            ),
            svg!("copilot"),
            svg!("windows"),
            svg!("amazon"),
            svg!("ebay"),
            svg!("ycombinator"),
            svg!("producthunt"),
            svg!("telegram"),
            svg!("pinterest"),
            svg!("discord"),
            svg!("twitch"),
            svg!("yahoo"),
            (
                "yandex",
                "image/x-icon",
                include_bytes!("../assets/brands/yandex.ico").as_slice(),
                "ico",
            ),
        ]
        .into_iter()
        .map(|(name, mime, body, extension)| Asset {
            name,
            mime,
            body,
            url: format!(
                "/assets/brands/{name}.{}.{extension}",
                &format!("{:x}", Sha256::digest(body))[..16]
            ),
        })
        .collect()
    });
    &ASSETS
}

pub(crate) fn icon(name: &str) -> String {
    let asset = assets()
        .iter()
        .find(|asset| asset.name == name)
        .expect("bundled brand icon");
    format!(
        r#"<span class="report-icon brand-{name}" aria-hidden="true"><img src="{}" width="16" height="16" alt="" decoding="async"></span>"#,
        asset.url
    )
}
