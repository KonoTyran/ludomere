use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

const MANIFEST: &str = include_str!("../../resources/gog-api-capabilities.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GogCapability {
    FriendsRead,
    FriendRequests,
    PresenceRead,
    PresenceWrite,
    AchievementsRead,
    SessionsRead,
    SessionsWrite,
    AccountTags,
    HiddenGames,
    FriendMutations,
    StatisticsRead,
    StatisticsWrite,
    Chat,
}

impl FromStr for GogCapability {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let quoted = serde_json::to_string(value)?;
        serde_json::from_str(&quoted).context("unknown GOG capability")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityStatus {
    pub name: GogCapability,
    pub read: bool,
    pub write: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GogCapabilityRegistry {
    pub schema: u32,
    pub capabilities: Vec<CapabilityStatus>,
}

impl GogCapabilityRegistry {
    pub fn load() -> Result<Self> {
        let registry: Self =
            serde_json::from_str(MANIFEST).context("decoding GOG API capability manifest")?;
        anyhow::ensure!(registry.schema == 1, "unsupported GOG capability schema");
        Ok(registry)
    }

    pub fn permits_read(&self, capability: GogCapability) -> bool {
        self.capabilities
            .iter()
            .any(|entry| entry.name == capability && entry.read)
    }

    pub fn permits_write(&self, capability: GogCapability) -> bool {
        self.capabilities
            .iter()
            .any(|entry| entry.name == capability && entry.write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_disables_every_unvalidated_write() {
        let registry = GogCapabilityRegistry::load().unwrap();
        assert!(registry.permits_read(GogCapability::FriendsRead));
        assert!(registry.permits_read(GogCapability::AchievementsRead));
        assert!(!registry.permits_write(GogCapability::PresenceWrite));
        assert!(!registry.permits_write(GogCapability::FriendMutations));
        assert!(!registry.permits_write(GogCapability::Chat));
    }

    #[test]
    fn capability_names_match_cli_values() {
        assert_eq!(
            "presence_write".parse::<GogCapability>().unwrap(),
            GogCapability::PresenceWrite
        );
        assert!("store".parse::<GogCapability>().is_err());
    }
}
