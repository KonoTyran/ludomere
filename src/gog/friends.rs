use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GogFriend {
    pub user_id: String,
    pub username: String,
    pub is_employee: bool,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FriendList {
    #[serde(default)]
    items: Vec<FriendResponse>,
}

#[derive(Debug, Deserialize)]
struct FriendResponse {
    user_id: String,
    username: String,
    #[serde(default)]
    is_employee: bool,
    #[serde(default)]
    images: FriendImages,
}

#[derive(Debug, Default, Deserialize)]
struct FriendImages {
    medium: Option<String>,
    medium_2x: Option<String>,
}

pub fn list(
    client: &reqwest::blocking::Client,
    token: &crate::auth::Token,
) -> Result<Vec<GogFriend>> {
    let response: FriendList = client
        .get(format!(
            "https://chat.gog.com/users/{}/friends",
            token.user_id
        ))
        .bearer_auth(&token.access_token)
        .send()?
        .error_for_status()?
        .json()
        .context("decoding GOG friends")?;
    Ok(response
        .items
        .into_iter()
        .map(|friend| GogFriend {
            user_id: friend.user_id,
            username: friend.username,
            is_employee: friend.is_employee,
            avatar_url: friend.images.medium_2x.or(friend.images.medium),
        })
        .collect())
}

pub fn invitation_count(
    client: &reqwest::blocking::Client,
    token: &crate::auth::Token,
) -> Result<usize> {
    let value: serde_json::Value = client
        .get(format!(
            "https://chat.gog.com/users/{}/invitations",
            token.user_id
        ))
        .bearer_auth(&token.access_token)
        .send()?
        .error_for_status()?
        .json()
        .context("decoding GOG friend invitations")?;
    Ok(value
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friend_fixture_is_typed_and_prefers_larger_avatar() {
        let response: FriendList = serde_json::from_str(
            r#"{"items":[{"user_id":"42","username":"Ada","is_employee":false,"images":{"medium":"small","medium_2x":"large"}}]}"#,
        )
        .unwrap();
        let friend = response.items.into_iter().next().unwrap();
        assert_eq!(friend.user_id, "42");
        assert_eq!(friend.images.medium_2x.as_deref(), Some("large"));
    }
}
