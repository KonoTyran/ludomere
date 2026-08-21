use std::path::PathBuf;

pub const APP_NAME: &str = "Ludomere";
pub const APP_ID: &str = "io.github.KonoTyran.Ludomere";
pub const XDG_DIRECTORY: &str = "ludomere";
pub const MARKER_DIRECTORY: &str = ".ludomere";
pub const STAGING_DIRECTORY: &str = ".ludomere-staging";
pub const WRITE_PROBE: &str = ".ludomere-write-test";
pub const USER_AGENT: &str = concat!("ludomere/", env!("CARGO_PKG_VERSION"));

pub fn config_root() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join(XDG_DIRECTORY)
}
pub fn data_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join(XDG_DIRECTORY)
}
pub fn cache_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join(XDG_DIRECTORY)
}
pub fn config_file() -> PathBuf {
    config_root().join("config.toml")
}
pub fn database() -> PathBuf {
    data_root().join("library.sqlite3")
}
pub fn account_data() -> PathBuf {
    data_root().join("account")
}
pub fn installation_logs() -> PathBuf {
    data_root().join("installation-logs")
}
pub fn runtime_logs() -> PathBuf {
    data_root().join("runtime-logs")
}
pub fn screenshots() -> PathBuf {
    cache_root().join("screenshots")
}
pub fn custom_artwork() -> PathBuf {
    data_root().join("custom-artwork")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_is_ludomere() {
        assert_eq!(APP_ID, "io.github.KonoTyran.Ludomere");
        assert_eq!(
            USER_AGENT,
            format!("ludomere/{}", env!("CARGO_PKG_VERSION"))
        );
    }
}
