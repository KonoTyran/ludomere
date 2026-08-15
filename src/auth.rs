use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, time::Duration};

const CLIENT_ID: &str = "46899977096215655";
const CLIENT_SECRET: &str = "9d85c43b1482497dbbce61f6e4aa173a433796eeae2ca8c5f6129f2dc4de46d9";
const REDIRECT_URI: &str = "https://embed.gog.com/on_login_success?origin=client";
const KEYRING_SERVICE: &str = crate::identity::APP_ID;
const KEYRING_USER: &str = "gog-oauth";

#[derive(Clone, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    pub expires_at: i64,
}

impl std::fmt::Debug for Token {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Token")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("user_id", &self.user_id)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    pub user_id: String,
    pub username: String,
    pub email: String,
    pub country: String,
    pub preferred_language: String,
    pub selected_currency: String,
    pub member_since: Option<i64>,
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub avatar_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    user_id: String,
    expires_in: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserData {
    user_id: String,
    username: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    country: String,
    preferred_language: Option<NamedValue>,
    selected_currency: Option<NamedValue>,
    is_logged_in: bool,
}

#[derive(Debug, Deserialize)]
struct NamedValue {
    #[serde(default)]
    code: String,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicProfile {
    user_since: Option<i64>,
    avatars: Option<Avatars>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Avatars {
    large2x: Option<String>,
    large: Option<String>,
    medium2x: Option<String>,
    medium: Option<String>,
}

pub fn login_url() -> String {
    let mut url = reqwest::Url::parse("https://auth.gog.com/auth").expect("valid GOG auth URL");
    url.query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("response_type", "code")
        .append_pair("layout", "client2");
    url.into()
}

pub fn authorization_code(uri: &str) -> Option<String> {
    let url = reqwest::Url::parse(uri).ok()?;
    let is_callback = url.host_str() == Some("embed.gog.com") && url.path() == "/on_login_success";
    is_callback
        .then(|| url.query_pairs().find(|(key, _)| key == "code"))
        .flatten()
        .map(|(_, value)| value.into_owned())
}

pub fn exchange_code(code: &str) -> Result<(Token, Profile)> {
    let client = http_client()?;
    let response: TokenResponse = client
        .get("https://auth.gog.com/token")
        .query(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
            ("code", code),
        ])
        .send()?
        .error_for_status()?
        .json()
        .context("decoding GOG token response")?;
    finish_authentication(&client, response, None)
}

pub fn refresh(token: &Token) -> Result<(Token, Profile)> {
    let client = http_client()?;
    let response: TokenResponse = client
        .get("https://auth.gog.com/token")
        .query(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("grant_type", "refresh_token"),
            ("refresh_token", token.refresh_token.as_str()),
        ])
        .send()?
        .error_for_status()?
        .json()
        .context("decoding refreshed GOG token")?;
    finish_authentication(&client, response, Some(&token.refresh_token))
}

pub fn restore() -> Result<Option<(Token, Profile)>> {
    let Some(token) = load_saved_token()? else {
        return Ok(None);
    };
    refresh(&token).map(Some)
}

pub fn load_saved_token() -> Result<Option<Token>> {
    read_token(KEYRING_SERVICE)
}

fn read_token(service: &str) -> Result<Option<Token>> {
    match keyring::Entry::new(service, KEYRING_USER)?.get_password() {
        Ok(serialized) => Ok(Some(serde_json::from_str(&serialized)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn save_token(token: &Token) -> Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?
        .set_password(&serde_json::to_string(token)?)?;
    Ok(())
}

pub fn logout() -> Result<()> {
    match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub fn fetch_owned_product_ids(token: &Token) -> Result<Vec<i64>> {
    #[derive(Deserialize)]
    struct OwnedGames {
        owned: Vec<i64>,
    }
    let response: OwnedGames = http_client()?
        .get("https://embed.gog.com/user/data/games")
        .bearer_auth(&token.access_token)
        .send()?
        .error_for_status()?
        .json()
        .context("decoding owned GOG library")?;
    let mut ids = response.owned;
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

fn finish_authentication(
    client: &reqwest::blocking::Client,
    response: TokenResponse,
    existing_refresh_token: Option<&str>,
) -> Result<(Token, Profile)> {
    let token = Token {
        access_token: response.access_token,
        refresh_token: response
            .refresh_token
            .or_else(|| existing_refresh_token.map(str::to_owned))
            .context("GOG token response did not include a refresh token")?,
        user_id: response.user_id,
        expires_at: chrono::Utc::now().timestamp() + response.expires_in,
    };
    let profile = fetch_profile(client, &token)?;
    save_token(&token)?;
    Ok((token, profile))
}

fn fetch_profile(client: &reqwest::blocking::Client, token: &Token) -> Result<Profile> {
    let bearer = format!("Bearer {}", token.access_token);
    let user: UserData = client
        .get("https://embed.gog.com/userData.json")
        .header(reqwest::header::AUTHORIZATION, &bearer)
        .send()?
        .error_for_status()?
        .json()?;
    if !user.is_logged_in {
        bail!("GOG reported that the session is not logged in");
    }
    let public: PublicProfile = client
        .get(format!("https://embed.gog.com/users/info/{}", user.user_id))
        .header(reqwest::header::AUTHORIZATION, bearer)
        .send()?
        .error_for_status()?
        .json()?;
    let avatar_url = public.avatars.and_then(|avatars| {
        avatars
            .large2x
            .or(avatars.large)
            .or(avatars.medium2x)
            .or(avatars.medium)
    });
    let mut profile = Profile {
        user_id: user.user_id,
        username: user.username,
        email: user.email,
        country: user.country,
        preferred_language: user.preferred_language.map_or_else(String::new, |value| {
            if value.name.is_empty() {
                value.code
            } else {
                value.name
            }
        }),
        selected_currency: user
            .selected_currency
            .map_or_else(String::new, |value| value.code),
        member_since: public.user_since,
        avatar_url,
        avatar_path: None,
    };
    profile.avatar_path = cache_avatar(client, &profile)?;
    Ok(profile)
}

fn cache_avatar(client: &reqwest::blocking::Client, profile: &Profile) -> Result<Option<PathBuf>> {
    let Some(url) = &profile.avatar_url else {
        return Ok(None);
    };
    let directory = crate::identity::cache_root().join("account");
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("avatar-{}.jpg", profile.user_id));
    let bytes = client.get(url).send()?.error_for_status()?.bytes()?;
    let temporary = path.with_extension("jpg.part");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, &path)?;
    Ok(Some(path))
}

fn http_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(crate::identity::USER_AGENT)
        .build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_code_only_from_gog_callback() {
        assert_eq!(
            authorization_code("https://embed.gog.com/on_login_success?origin=client&code=abc123"),
            Some("abc123".into())
        );
        assert_eq!(authorization_code("https://example.com/?code=abc123"), None);
    }

    #[test]
    fn debug_output_redacts_credentials() {
        let token = Token {
            access_token: "access-secret".into(),
            refresh_token: "refresh-secret".into(),
            user_id: "42".into(),
            expires_at: 123,
        };
        let output = format!("{token:?}");
        assert!(!output.contains("access-secret"));
        assert!(!output.contains("refresh-secret"));
        assert!(output.contains("[REDACTED]"));
    }
}
