use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GogPresence {
    pub user_id: String,
    pub client_id: Option<String>,
    pub data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct PresenceList {
    #[serde(default)]
    items: Vec<PresenceResponse>,
}

#[derive(Debug, Deserialize)]
struct PresenceResponse {
    user_id: String,
    client_id: Option<String>,
    #[serde(default)]
    data: serde_json::Value,
}

pub fn statuses(
    client: &reqwest::blocking::Client,
    token: &crate::auth::Token,
    user_ids: &[String],
) -> Result<Vec<GogPresence>> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let response: PresenceList = client
        .get("https://presence.gog.com/statuses")
        .query(&[("user_id", user_ids.join(","))])
        .bearer_auth(&token.access_token)
        .send()?
        .error_for_status()?
        .json()
        .context("decoding GOG presence")?;
    Ok(response
        .items
        .into_iter()
        .map(|presence| GogPresence {
            user_id: presence.user_id,
            client_id: presence.client_id,
            data: presence.data,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_fixture_accepts_unknown_data() {
        let response: PresenceList = serde_json::from_str(
            r#"{"items":[{"user_id":"42","client_id":"7","data":{"status":"playing"}}]}"#,
        )
        .unwrap();
        assert_eq!(response.items[0].data["status"], "playing");
    }
}
