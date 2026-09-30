use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

static SESSION: AtomicU64 = AtomicU64::new(1);
static SIGNED_OUT: AtomicU64 = AtomicU64::new(0);
static CREDENTIAL_WRITES: Mutex<()> = Mutex::new(());
static SIGN_OUT_FILE: Mutex<()> = Mutex::new(());
static SIGN_OUT_DURABLE: AtomicBool = AtomicBool::new(false);
static LOGOUT_PENDING: (Mutex<bool>, std::sync::Condvar) =
    (Mutex::new(false), std::sync::Condvar::new());

pub fn session() -> u64 {
    SESSION.load(Ordering::Acquire)
}

/// Immediate revocation never waits for filesystem, keyring or worker locks.
pub fn invalidate_session() {
    let generation = SESSION.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    SIGNED_OUT.store(generation, Ordering::Release);
}

/// Called on the UI thread before dispatch, so closing cannot skip the queued cleanup.
pub fn begin_sign_out() {
    invalidate_session();
    SIGN_OUT_DURABLE.store(false, Ordering::Release);
    *LOGOUT_PENDING
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = true;
}

pub fn wait_for_sign_out() -> Result<()> {
    wait_for_sign_out_for(Duration::from_secs(5))
}

fn wait_for_sign_out_for(timeout: Duration) -> Result<()> {
    let pending = LOGOUT_PENDING
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (pending, _) = LOGOUT_PENDING
        .1
        .wait_timeout_while(pending, timeout, |pending| *pending)
        .unwrap_or_else(|error| error.into_inner());
    anyhow::ensure!(
        !*pending,
        "{}",
        if SIGN_OUT_DURABLE.load(Ordering::Acquire) {
            "Sign-out credential cleanup is still pending; the saved sign-out marker prevents automatic login"
        } else {
            "Sign-out cleanup did not finish and its durable marker was not confirmed. Stored login data may remain; retry cleanup."
        }
    );
    Ok(())
}

pub fn session_is_current(expected: u64) -> bool {
    session() == expected && SIGNED_OUT.load(Ordering::Acquire) == 0
}

fn signed_out_path() -> PathBuf {
    crate::identity::config_root().join(".gog-signed-out")
}

fn check_session(expected: u64, explicit_login: bool) -> Result<()> {
    anyhow::ensure!(
        session() == expected && (explicit_login || SIGNED_OUT.load(Ordering::Acquire) == 0),
        "GOG session changed; sign in again"
    );
    Ok(())
}

pub fn cache_profile_if_current(profile: &Profile, expected: u64) -> Result<()> {
    let _lock = CREDENTIAL_WRITES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    check_session(expected, false)?;
    crate::state::StateStore::open()?.cache_profile(profile)
}

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

pub fn exchange_code(code: &str, expected: u64) -> Result<(Token, Profile)> {
    check_session(expected, true)?;
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
    finish_authentication(&client, response, None, expected, true)
}

pub fn refresh(token: &Token, expected: u64) -> Result<(Token, Profile)> {
    check_session(expected, false)?;
    anyhow::ensure!(!restoration_blocked()?, "Signed out; sign in again");
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
    finish_authentication(
        &client,
        response,
        Some(&token.refresh_token),
        expected,
        false,
    )
}

pub fn restore(expected: u64) -> Result<Option<(Token, Profile)>> {
    check_session(expected, false)?;
    let Some(token) = load_saved_token()? else {
        return Ok(None);
    };
    refresh(&token, expected).map(Some)
}

pub fn load_saved_token() -> Result<Option<Token>> {
    load_saved_token_with(|| read_token(KEYRING_SERVICE))
}

fn load_saved_token_with(read: impl FnOnce() -> Result<Option<Token>>) -> Result<Option<Token>> {
    let expected = session();
    if SIGNED_OUT.load(Ordering::Acquire) != 0 || restoration_blocked()? {
        return Ok(None);
    }
    let token = read()?;
    if !session_is_current(expected) || restoration_blocked()? {
        return Ok(None);
    }
    Ok(token)
}

pub(crate) fn restoration_blocked() -> Result<bool> {
    match fs::symlink_metadata(signed_out_path()) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("Could not check local sign-out state"),
    }
}

fn read_token(service: &str) -> Result<Option<Token>> {
    match keyring::Entry::new(service, KEYRING_USER)?.get_password() {
        Ok(serialized) => Ok(Some(serde_json::from_str(&serialized)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn save_token(token: &Token) -> Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?
        .set_password(&serde_json::to_string(token)?)?;
    Ok(())
}

pub fn logout() -> Result<()> {
    struct Finished;
    impl Drop for Finished {
        fn drop(&mut self) {
            *LOGOUT_PENDING
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = false;
            LOGOUT_PENDING.1.notify_all();
        }
    }
    let _finished = Finished;
    logout_with(|| {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .and_then(|entry| entry.delete_credential())
    })
}

fn logout_with(delete: impl FnOnce() -> std::result::Result<(), keyring::Error>) -> Result<()> {
    let _lock = CREDENTIAL_WRITES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    SIGNED_OUT.store(session(), Ordering::Release);
    let marker = persist_sign_out();
    let credential = delete();
    let profile = crate::state::StateStore::open().and_then(|store| store.clear_cached_profile());
    marker?;
    anyhow::ensure!(
        matches!(credential, Ok(()) | Err(keyring::Error::NoEntry)),
        "Signed out locally; stored credential cleanup failed. Automatic login remains disabled. Retry cleanup."
    );
    profile.context("Signed out, but clearing cached account details failed; retry cleanup")
}

/// Worker-only durable barrier; it never waits for the credential service.
pub fn persist_sign_out() -> Result<()> {
    let _lock = SIGN_OUT_FILE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    (|| -> Result<()> {
        fs::create_dir_all(crate::identity::config_root())?;
        let mut marker = tempfile::NamedTempFile::new_in(crate::identity::config_root())?;
        marker.write_all(b"signed out\n")?;
        marker.as_file().sync_all()?;
        marker.persist(signed_out_path())?;
        fs::File::open(crate::identity::config_root())?.sync_all()?;
        SIGN_OUT_DURABLE.store(true, Ordering::Release);
        Ok(())
    })().context("Signed out in this session, but saving the local sign-out marker failed; retry sign-out cleanup before closing")
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
    expected: u64,
    explicit_login: bool,
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
    commit_credentials(expected, explicit_login, || save_token(&token))?;
    Ok((token, profile))
}

fn commit_credentials(
    expected: u64,
    explicit_login: bool,
    save: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let _lock = CREDENTIAL_WRITES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    check_session(expected, explicit_login)?;
    if explicit_login && restoration_blocked()? {
        crate::installation::normalize_signed_out_operations()
            .context("Sign-out cleanup is incomplete; retry cleanup before signing in")?;
    }
    save()?;
    let _marker = SIGN_OUT_FILE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    check_session(expected, explicit_login)?;
    if explicit_login {
        match fs::remove_file(signed_out_path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let _ = SIGNED_OUT.compare_exchange(expected, 0, Ordering::AcqRel, Ordering::Acquire);
    check_session(expected, false)?;
    if explicit_login {
        crate::installation::finish_sign_out_pause();
    }
    Ok(())
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
    fn durable_sign_out_and_bounded_close_do_not_wait_for_credential_service() {
        let previous = (session(), SIGNED_OUT.load(Ordering::Acquire));
        let credential = CREDENTIAL_WRITES.lock().unwrap();
        begin_sign_out();
        persist_sign_out().unwrap();
        assert!(signed_out_path().is_file());
        let started = std::time::Instant::now();
        let error = wait_for_sign_out_for(Duration::from_millis(10)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(error.to_string().contains("prevents automatic login"));
        drop(credential);
        *LOGOUT_PENDING.0.lock().unwrap() = false;
        fs::remove_file(signed_out_path()).unwrap();
        SESSION.store(previous.0, Ordering::Release);
        SIGNED_OUT.store(previous.1, Ordering::Release);
    }

    #[test]
    fn sign_out_survives_credential_failure_and_rejects_delayed_authentication() {
        let previous = (session(), SIGNED_OUT.load(Ordering::Acquire));
        let marker = signed_out_path();
        assert!(!marker.exists(), "requires an isolated test profile");
        let old = session();
        invalidate_session();
        let message = logout_with(|| {
            Err(keyring::Error::PlatformFailure(Box::new(
                std::io::Error::other("inert secret must not be shown"),
            )))
        })
        .unwrap_err()
        .to_string();
        assert!(message.contains("Signed out locally"));
        assert!(!message.contains("inert secret"));
        assert!(marker.is_file());
        let saved = std::cell::Cell::new(false);
        assert!(
            commit_credentials(old, false, || {
                saved.set(true);
                Ok(())
            })
            .is_err()
        );
        assert!(!saved.get());
        assert!(
            load_saved_token_with(|| panic!("tombstone must block keyring access"))
                .unwrap()
                .is_none()
        );
        let current = session();
        commit_credentials(current, true, || Ok(())).unwrap();
        assert!(!marker.exists());
        assert!(session_is_current(current));
        assert!(
            load_saved_token_with(|| {
                invalidate_session();
                Ok(Some(Token {
                    access_token: "inert".into(),
                    refresh_token: "inert".into(),
                    user_id: "fixture".into(),
                    expires_at: 1,
                }))
            })
            .unwrap()
            .is_none()
        );
        assert!(
            commit_credentials(session(), true, || {
                invalidate_session();
                Ok(())
            })
            .is_err()
        );
        assert!(!session_is_current(session()));
        SESSION.store(previous.0, Ordering::Release);
        SIGNED_OUT.store(previous.1, Ordering::Release);
    }

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
