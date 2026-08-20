use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GogAchievement {
    pub id: String,
    pub key: String,
    pub visible: bool,
    pub name: String,
    pub description: String,
    pub unlocked_image_url: Option<String>,
    pub locked_image_url: Option<String>,
    pub unlocked_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AchievementList {
    #[serde(default)]
    items: Vec<AchievementResponse>,
}

#[derive(Debug, Deserialize)]
struct AchievementResponse {
    achievement_id: String,
    achievement_key: String,
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    image_url_unlocked: Option<String>,
    image_url_locked: Option<String>,
    date_unlocked: Option<String>,
}

pub fn achievements(
    client: &reqwest::blocking::Client,
    token: &crate::auth::Token,
    product_id: i64,
    user_id: &str,
) -> Result<Vec<GogAchievement>> {
    let response: AchievementList = client
        .get(format!(
            "https://gameplay.gog.com/clients/{product_id}/users/{user_id}/achievements"
        ))
        .bearer_auth(&token.access_token)
        .send()?
        .error_for_status()?
        .json()
        .context("decoding GOG achievements")?;
    Ok(response
        .items
        .into_iter()
        .map(|achievement| GogAchievement {
            id: achievement.achievement_id,
            key: achievement.achievement_key,
            visible: achievement.visible,
            name: achievement.name,
            description: achievement.description,
            unlocked_image_url: achievement.image_url_unlocked,
            locked_image_url: achievement.image_url_locked,
            unlocked_at: achievement.date_unlocked,
        })
        .collect())
}

pub fn session_count(
    client: &reqwest::blocking::Client,
    token: &crate::auth::Token,
    product_id: i64,
    user_id: &str,
) -> Result<usize> {
    let value: serde_json::Value = client
        .get(format!(
            "https://gameplay.gog.com/clients/{product_id}/users/{user_id}/sessions"
        ))
        .bearer_auth(&token.access_token)
        .send()?
        .error_for_status()?
        .json()
        .context("decoding GOG sessions")?;
    Ok(value
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn achievement_fixture_allows_locked_entries() {
        let response: AchievementList = serde_json::from_str(
            r#"{"items":[{"achievement_id":"1","achievement_key":"FIRST","visible":true,"name":"First","description":"Do it","image_url_unlocked":"u","image_url_locked":"l","date_unlocked":null}]}"#,
        )
        .unwrap();
        assert_eq!(response.items[0].achievement_key, "FIRST");
        assert!(response.items[0].date_unlocked.is_none());
    }
}
