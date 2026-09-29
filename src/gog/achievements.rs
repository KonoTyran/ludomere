use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::io::Read;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Achievement {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: String,
    pub visible: bool,
    pub unlocked_at: Option<String>,
    pub progress: Option<f64>,
    pub progress_max: Option<f64>,
    pub rarity: Option<f64>,
}

pub fn refresh(
    token: &crate::auth::Token,
    product_id: i64,
    session: u64,
) -> Result<crate::state::CachedAchievements> {
    crate::online::with_account_session(session, || Ok(()))?;
    let response = super::client()?
        .get(format!(
            "https://gameplay.gog.com/clients/{product_id}/users/{}/achievements",
            token.user_id
        ))
        .bearer_auth(&token.access_token)
        .send()
        .map_err(|_| anyhow::anyhow!("Could not contact GOG achievements. Retry when online."))?;
    ensure!(
        response.status().is_success(),
        "GOG achievements returned HTTP {}. Retry or sign in again.",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    response
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Could not read GOG achievements"))?;
    ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "Achievement response too large"
    );
    let achievements = decode(&bytes)?;
    crate::online::with_account_session(session, || {
        let store = crate::state::StateStore::open()?;
        store.replace_achievements(&token.user_id, product_id, &achievements)?;
        store
            .cached_achievements(&token.user_id, product_id)?
            .ok_or_else(|| anyhow::anyhow!("Could not read saved achievements"))
    })
}

fn decode(bytes: &[u8]) -> Result<Vec<Achievement>> {
    #[derive(Deserialize)]
    struct Response {
        items: Vec<Item>,
        total_count: Option<usize>,
    }
    #[derive(Deserialize)]
    struct Item {
        achievement_id: String,
        achievement_key: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        visible: bool,
        date_unlocked: Option<String>,
        progress: Option<f64>,
        progress_max: Option<f64>,
        rarity: Option<f64>,
    }
    let response: Response = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("GOG returned an unsupported achievement response"))?;
    ensure!(
        response
            .total_count
            .is_none_or(|total| total <= response.items.len()),
        "GOG returned only part of the achievement list. Cached achievements were kept; retry later."
    );
    let mut ids = std::collections::HashSet::new();
    Ok(response
        .items
        .into_iter()
        .filter(|item| ids.insert(item.achievement_id.clone()))
        .map(|item| Achievement {
            id: item.achievement_id,
            key: item.achievement_key,
            name: item.name,
            description: item.description,
            visible: item.visible,
            unlocked_at: item.date_unlocked,
            progress: item.progress.filter(|v| v.is_finite() && *v >= 0.0),
            progress_max: item.progress_max.filter(|v| v.is_finite() && *v > 0.0),
            rarity: item
                .rarity
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v)),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_and_empty_results_do_not_invent_progress() {
        let values = decode(br#"{"items":[{"achievement_id":"1","achievement_key":"FIRST","visible":true,"name":"First","date_unlocked":null}]}"#).unwrap();
        assert_eq!(values[0].name, "First");
        assert!(values[0].unlocked_at.is_none());
        assert!(values[0].progress.is_none());
        assert!(decode(br#"{"items":[]}"#).unwrap().is_empty());
        assert!(decode(br#"{"error":"not achievements"}"#).is_err());
        assert!(decode(br#"{"items":[],"total_count":1,"page_token":"next"}"#).is_err());
    }

    #[test]
    fn provided_progress_and_unlocks_survive_without_duplicate_entries() {
        let values = decode(br#"{"items":[{"achievement_id":"1","achievement_key":"FIRST","date_unlocked":"2026-01-01","progress":2,"progress_max":5,"rarity":3.5},{"achievement_id":"1","achievement_key":"FIRST"}]}"#).unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].progress, Some(2.0));
        assert_eq!(values[0].progress_max, Some(5.0));
        assert!(values[0].unlocked_at.is_some());
    }
}
