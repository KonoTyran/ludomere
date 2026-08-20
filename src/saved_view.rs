use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedView {
    pub id: i64,
    pub name: String,
    pub query: SavedViewQuery,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedViewQuery {
    pub version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloaded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub played: Option<bool>,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub all_tags: bool,
    #[serde(default)]
    pub operating_systems: Vec<String>,
    #[serde(default)]
    pub sort: SavedViewSort,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedViewSort {
    #[default]
    Title,
    LastPlayed,
    Playtime,
    ReleaseDate,
}

impl Default for SavedViewQuery {
    fn default() -> Self {
        Self {
            version: 1,
            text: None,
            installed: None,
            downloaded: None,
            owned: Some(true),
            favorite: None,
            played: None,
            include_hidden: false,
            tags: Vec::new(),
            all_tags: false,
            operating_systems: Vec::new(),
            sort: SavedViewSort::Title,
        }
    }
}

impl SavedViewQuery {
    pub fn from_json(json: &str) -> Result<Self> {
        let query: Self = serde_json::from_str(json)?;
        query.validate()?;
        Ok(query)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!(
                "saved view version {} is not supported; update Ludomere or recreate the view",
                self.version
            );
        }
        for os in &self.operating_systems {
            if !matches!(os.as_str(), "windows" | "linux" | "macos") {
                bail!("saved view contains unsupported operating system `{os}`");
            }
        }
        if self.tags.iter().any(|tag| tag.trim().is_empty()) {
            bail!("saved view contains an empty tag");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_fields_and_versions() {
        assert!(SavedViewQuery::from_json(r#"{"version":1,"future":true}"#).is_err());
        assert!(SavedViewQuery::from_json(r#"{"version":2}"#).is_err());
    }

    #[test]
    fn round_trips_current_query() {
        let query = SavedViewQuery {
            tags: vec!["RPG".into()],
            all_tags: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&query).unwrap();
        assert_eq!(SavedViewQuery::from_json(&json).unwrap(), query);
    }
}
