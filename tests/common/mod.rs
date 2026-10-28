use std::sync::OnceLock;
pub const PASSWORD: &str = "correct horse owl telescope";
pub fn hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| owlytics::auth::hash_password(PASSWORD).unwrap())
}
pub fn config() -> owlytics::config::Config {
    owlytics::config::Config::parse(
        "127.0.0.1:3000",
        ":memory:",
        "https://stats.example.com",
        hash(),
    )
    .unwrap()
}
