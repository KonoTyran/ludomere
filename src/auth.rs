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
        SignInStage::Session
    );
    Ok(())
}

#[derive(Debug)]
enum SignInStage {
    Token,
    Profile,
    Cleanup,
    Credentials,
    Session,
}

impl std::fmt::Display for SignInStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Token => "GOG sign-in could not exchange the login response. Try signing in again.",
            Self::Profile => "GOG sign-in could not verify your account profile. Try signing in again.",
            Self::Cleanup => "GOG sign-in could not finish local sign-out cleanup. Retry sign-out cleanup before signing in again.",
            Self::Credentials => "GOG sign-in could not save your login in the system credential store. Check your desktop credential service, then try signing in again.",
            Self::Session => "The GOG session changed during sign-in. Try signing in again.",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialStoreIssue {
    Bus,
    Unavailable,
    Activation,
    Disabled,
    Access,
    Ambiguous,
}

impl std::fmt::Display for CredentialStoreIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Bus => "GOG sign-in could not contact the session credential service. Check your desktop session D-Bus connection, then try again.",
            Self::Unavailable => "GOG sign-in needs a Secret Service credential provider, but none is available in this desktop session. Enable a compatible desktop keyring, then try again.",
            Self::Activation => "GOG sign-in could not start the advertised KDE credential service. Check that your desktop wallet is enabled, then try again.",
            Self::Disabled => "The KDE credential service started, but its Secret Service interface is unavailable. Check that the wallet and its Secret Service API are enabled, then try again.",
            Self::Access => "GOG sign-in could not access the credential store. Unlock your desktop wallet and allow its access prompt, then try again.",
            Self::Ambiguous => "GOG sign-in found duplicate matching login entries in the credential store. Review Ludomere entries in your desktop keyring, then try again.",
        })
    }
}

impl std::error::Error for CredentialStoreIssue {}

/// Only fixed diagnostic text and numeric HTTP status may leave the authentication boundary.
pub fn sign_in_error_message(error: &anyhow::Error) -> String {
    if matches!(
        error.downcast_ref::<SignInStage>(),
        Some(SignInStage::Credentials)
    ) {
        if let Some(issue) = error.downcast_ref::<CredentialStoreIssue>() {
            return issue.to_string();
        }
        match error.downcast_ref::<keyring::Error>() {
            Some(keyring::Error::NoStorageAccess(_)) => {
                return CredentialStoreIssue::Access.to_string();
            }
            Some(keyring::Error::Ambiguous(_)) => {
                return CredentialStoreIssue::Ambiguous.to_string();
            }
            _ => {}
        }
    }
    let mut message = error.downcast_ref::<SignInStage>().map_or_else(
        || "GOG sign-in could not finish. Try signing in again.".to_owned(),
        ToString::to_string,
    );
    if let Some(request) = error.downcast_ref::<reqwest::Error>() {
        if let Some(status) = request.status() {
            message.push_str(&format!(" GOG request returned HTTP {}.", status.as_u16()));
        } else if request.is_timeout() {
            message.push_str(" The request timed out; check your connection.");
        } else if request.is_connect() {
            message.push_str(" Could not connect to GOG; check your connection.");
        }
    }
    message
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
    let client = http_client().context(SignInStage::Token)?;
    let response: TokenResponse = client
        .get("https://auth.gog.com/token")
        .query(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
            ("code", code),
        ])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::json)
        .context(SignInStage::Token)?;
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

fn prepare_credential_store(expected: u64) -> Result<()> {
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|_| CredentialStoreIssue::Bus)?;
    prepare_secret_service(|method, parameters| {
        check_session(expected, true)?;
        let response = connection
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                method,
                parameters,
                None,
                gio::DBusCallFlags::NONE,
                5_000,
                gio::Cancellable::NONE,
            )
            .map_err(|_| {
                if method == "StartServiceByName" {
                    CredentialStoreIssue::Activation
                } else {
                    CredentialStoreIssue::Bus
                }
            })?;
        check_session(expected, true)?;
        Ok(response)
    })
}

fn prepare_secret_service(
    mut call: impl FnMut(&str, Option<&gio::glib::Variant>) -> Result<gio::glib::Variant>,
) -> Result<()> {
    use gio::glib::variant::ToVariant;
    let standard = ("org.freedesktop.secrets",).to_variant();
    if call("NameHasOwner", Some(&standard))?
        .get::<(bool,)>()
        .ok_or(CredentialStoreIssue::Bus)?
        .0
    {
        return Ok(());
    }
    let (names,) = call("ListActivatableNames", None)?
        .get::<(Vec<String>,)>()
        .ok_or(CredentialStoreIssue::Bus)?;
    if names.iter().any(|name| name == "org.freedesktop.secrets") {
        return Ok(());
    }
    anyhow::ensure!(
        names
            .iter()
            .any(|name| name == "org.kde.secretservicecompat"),
        CredentialStoreIssue::Unavailable
    );
    // Start only the advertised compatibility service; keep using the standard API.
    // KWallet registers that name only when its Secret Service API is enabled.
    let (status,) = call(
        "StartServiceByName",
        Some(&("org.kde.secretservicecompat", 0u32).to_variant()),
    )?
    .get::<(u32,)>()
    .ok_or(CredentialStoreIssue::Activation)?;
    anyhow::ensure!(matches!(status, 1 | 2), CredentialStoreIssue::Activation);
    anyhow::ensure!(
        call("NameHasOwner", Some(&standard))?
            .get::<(bool,)>()
            .ok_or(CredentialStoreIssue::Bus)?
            .0,
        CredentialStoreIssue::Disabled
    );
    Ok(())
}

struct SignOutCompletion;

impl Drop for SignOutCompletion {
    fn drop(&mut self) {
        *LOGOUT_PENDING
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = false;
        LOGOUT_PENDING.1.notify_all();
    }
}

pub fn logout() -> Result<()> {
    let _finished = SignOutCompletion;
    logout_with(|| {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .and_then(|entry| entry.delete_credential())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetCredentialCleanup {
    Removed,
    Unavailable,
}

/// Factory reset removes local account data separately and retains the sign-out barrier.
pub fn logout_for_reset() -> Result<ResetCredentialCleanup> {
    logout_for_reset_with(|| {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .and_then(|entry| entry.delete_credential())
    })
}

fn logout_for_reset_with(
    delete: impl FnOnce() -> std::result::Result<(), keyring::Error>,
) -> Result<ResetCredentialCleanup> {
    let _finished = SignOutCompletion;
    let _lock = CREDENTIAL_WRITES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    SIGNED_OUT.store(session(), Ordering::Release);
    persist_sign_out()?;
    Ok(
        if matches!(delete(), Ok(()) | Err(keyring::Error::NoEntry)) {
            ResetCredentialCleanup::Removed
        } else {
            ResetCredentialCleanup::Unavailable
        },
    )
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
            .context("GOG token response did not include a refresh token")
            .context(SignInStage::Token)?,
        user_id: response.user_id,
        expires_at: chrono::Utc::now().timestamp() + response.expires_in,
    };
    let profile = fetch_profile(client, &token)?;
    commit_credentials(expected, explicit_login, || {
        if explicit_login {
            prepare_credential_store(expected)?;
            check_session(expected, true)?;
        }
        save_token(&token)
    })?;
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
    if explicit_login && restoration_blocked().context(SignInStage::Cleanup)? {
        crate::installation::normalize_signed_out_operations().context(SignInStage::Cleanup)?;
    }
    save().context(SignInStage::Credentials)?;
    let _marker = SIGN_OUT_FILE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    check_session(expected, explicit_login)?;
    if explicit_login {
        match fs::remove_file(signed_out_path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context(SignInStage::Cleanup),
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
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::json)
        .context(SignInStage::Profile)?;
    if !user.is_logged_in {
        bail!(SignInStage::Profile);
    }
    let public = client
        .get(format!("https://embed.gog.com/users/info/{}", user.user_id))
        .header(reqwest::header::AUTHORIZATION, bearer)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::json);
    Ok(profile_with_optional_details(
        user,
        public.map_err(Into::into),
        |profile| cache_avatar(client, profile),
    ))
}

fn profile_with_optional_details(
    user: UserData,
    public: Result<PublicProfile>,
    avatar: impl FnOnce(&Profile) -> Result<Option<PathBuf>>,
) -> Profile {
    let public = public.unwrap_or_else(|_| {
        tracing::warn!("Optional GOG public profile details unavailable; continuing sign-in");
        PublicProfile {
            user_since: None,
            avatars: None,
        }
    });
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
    profile.avatar_path = avatar(&profile).unwrap_or_else(|_| {
        tracing::warn!("Optional GOG account avatar unavailable; continuing sign-in");
        None
    });
    profile
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

    fn run_in_private_process(name: &str) -> bool {
        if std::env::var("LUDOMERE_TEST_AUTH_CHILD").as_deref() == Ok(name) {
            return false;
        }
        let directory = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", name])
            .env("LUDOMERE_TEST_AUTH_CHILD", name);
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            let path = directory.path().join(key);
            fs::create_dir(&path).unwrap();
            command.env(key, path);
        }
        assert!(command.status().unwrap().success());
        true
    }

    #[test]
    fn reset_credential_failure_stays_signed_out_across_restart_until_new_save_succeeds() {
        if run_in_private_process(
            "auth::tests::reset_credential_failure_stays_signed_out_across_restart_until_new_save_succeeds",
        ) {
            return;
        }
        let previous = (
            session(),
            SIGNED_OUT.load(Ordering::Acquire),
            SIGN_OUT_DURABLE.load(Ordering::Acquire),
        );
        let marker = signed_out_path();
        assert!(!marker.exists(), "requires an isolated test profile");
        begin_sign_out();
        let outcome = logout_for_reset_with(|| {
            assert_eq!(fs::read(&marker).unwrap(), b"signed out\n");
            assert!(SIGN_OUT_DURABLE.load(Ordering::Acquire));
            Err(keyring::Error::PlatformFailure(Box::new(
                std::io::Error::other("PRIVATE_KEYRING_ERROR"),
            )))
        })
        .unwrap();
        assert_eq!(outcome, ResetCredentialCleanup::Unavailable);
        assert!(!format!("{outcome:?}").contains("PRIVATE_KEYRING_ERROR"));
        wait_for_sign_out_for(Duration::from_millis(1)).unwrap();
        // A new process starts with these atomics reset; the retained file is authoritative.
        SESSION.store(1, Ordering::Release);
        SIGNED_OUT.store(0, Ordering::Release);
        SIGN_OUT_DURABLE.store(false, Ordering::Release);
        assert!(
            load_saved_token_with(|| panic!("old stored token must not be read"))
                .unwrap()
                .is_none()
        );
        assert!(
            commit_credentials(1, true, || Err(anyhow::anyhow!("keyring unavailable"))).is_err()
        );
        assert!(marker.is_file());
        assert!(
            load_saved_token_with(|| panic!("failed replacement must not enable restore"))
                .unwrap()
                .is_none()
        );
        let replaced = std::cell::Cell::new(false);
        commit_credentials(1, true, || {
            assert!(
                marker.is_file(),
                "barrier stays until credential save succeeds"
            );
            replaced.set(true);
            Ok(())
        })
        .unwrap();
        assert!(replaced.get());
        assert!(!marker.exists());
        assert!(session_is_current(1));
        for result in [Ok(()), Err(keyring::Error::NoEntry)] {
            assert_eq!(
                logout_for_reset_with(|| result).unwrap(),
                ResetCredentialCleanup::Removed
            );
            assert!(marker.is_file());
        }
        fs::remove_file(marker).unwrap();
        SESSION.store(previous.0, Ordering::Release);
        SIGNED_OUT.store(previous.1, Ordering::Release);
        SIGN_OUT_DURABLE.store(previous.2, Ordering::Release);
    }

    #[test]
    fn reset_marker_failure_aborts_before_credential_cleanup_and_settles_pending() {
        if run_in_private_process(
            "auth::tests::reset_marker_failure_aborts_before_credential_cleanup_and_settles_pending",
        ) {
            return;
        }
        let previous = (
            session(),
            SIGNED_OUT.load(Ordering::Acquire),
            SIGN_OUT_DURABLE.load(Ordering::Acquire),
        );
        let marker = signed_out_path();
        assert!(!marker.exists(), "requires an isolated test profile");
        fs::create_dir_all(&marker).unwrap();
        begin_sign_out();
        let error = logout_for_reset_with(|| panic!("marker must be durable before keyring call"))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("saving the local sign-out marker failed")
        );
        assert!(!SIGN_OUT_DURABLE.load(Ordering::Acquire));
        wait_for_sign_out_for(Duration::from_millis(1)).unwrap();
        assert!(!session_is_current(session()));
        fs::remove_dir(marker).unwrap();
        SESSION.store(previous.0, Ordering::Release);
        SIGNED_OUT.store(previous.1, Ordering::Release);
        SIGN_OUT_DURABLE.store(previous.2, Ordering::Release);
    }

    #[test]
    fn credential_service_detection_prefers_standard_and_only_activates_advertised_kde() {
        use gio::glib::variant::ToVariant;
        for (owned, names, after, expected, calls) in [
            (true, vec![], false, None, vec!["NameHasOwner"]),
            (
                false,
                vec!["org.freedesktop.secrets", "org.kde.secretservicecompat"],
                false,
                None,
                vec!["NameHasOwner", "ListActivatableNames"],
            ),
            (
                false,
                vec![],
                false,
                Some(CredentialStoreIssue::Unavailable),
                vec!["NameHasOwner", "ListActivatableNames"],
            ),
            (
                false,
                vec!["org.kde.secretservicecompat"],
                true,
                None,
                vec![
                    "NameHasOwner",
                    "ListActivatableNames",
                    "StartServiceByName",
                    "NameHasOwner",
                ],
            ),
            (
                false,
                vec!["org.kde.secretservicecompat"],
                false,
                Some(CredentialStoreIssue::Disabled),
                vec![
                    "NameHasOwner",
                    "ListActivatableNames",
                    "StartServiceByName",
                    "NameHasOwner",
                ],
            ),
        ] {
            let mut seen = Vec::new();
            let result = prepare_secret_service(|method, arguments| {
                seen.push(method.to_owned());
                Ok(match method {
                    "NameHasOwner" => {
                        assert_eq!(
                            arguments.unwrap().get::<(String,)>().unwrap().0,
                            "org.freedesktop.secrets"
                        );
                        (if seen.len() == 1 { owned } else { after },).to_variant()
                    }
                    "ListActivatableNames" => {
                        assert!(arguments.is_none());
                        (names.clone(),).to_variant()
                    }
                    "StartServiceByName" => {
                        assert_eq!(
                            arguments.unwrap().get::<(String, u32)>().unwrap(),
                            ("org.kde.secretservicecompat".into(), 0)
                        );
                        (1u32,).to_variant()
                    }
                    _ => panic!("unexpected metadata call"),
                })
            });
            assert_eq!(
                result
                    .err()
                    .map(|error| *error.downcast_ref::<CredentialStoreIssue>().unwrap()),
                expected
            );
            assert_eq!(seen, calls);
        }
    }

    #[test]
    fn credential_detection_failures_do_not_fall_back_or_claim_service_absent() {
        use gio::glib::variant::ToVariant;
        for failing in ["NameHasOwner", "ListActivatableNames", "StartServiceByName"] {
            let mut seen_failure = false;
            let error = prepare_secret_service(|method, _| {
                assert!(!seen_failure, "must stop after failure");
                if method == failing {
                    seen_failure = true;
                    return Err(if method == "StartServiceByName" {
                        CredentialStoreIssue::Activation
                    } else {
                        CredentialStoreIssue::Bus
                    }
                    .into());
                }
                Ok(match method {
                    "NameHasOwner" => (false,).to_variant(),
                    "ListActivatableNames" => (vec!["org.kde.secretservicecompat"],).to_variant(),
                    _ => panic!("unexpected call"),
                })
            })
            .unwrap_err();
            assert!(seen_failure);
            assert_ne!(
                error.downcast_ref::<CredentialStoreIssue>(),
                Some(&CredentialStoreIssue::Unavailable)
            );
        }
        assert_eq!(
            prepare_secret_service(|_, _| Ok(("bad reply",).to_variant()))
                .unwrap_err()
                .downcast_ref::<CredentialStoreIssue>(),
            Some(&CredentialStoreIssue::Bus)
        );
    }

    #[test]
    fn credential_errors_are_actionable_without_exposing_platform_payloads() {
        for (error, expected) in [
            (
                keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("PRIVATE_TOKEN"))),
                "allow its access prompt",
            ),
            (keyring::Error::Ambiguous(Vec::new()), "duplicate matching"),
            (
                keyring::Error::PlatformFailure(Box::new(std::io::Error::other("PRIVATE_TOKEN"))),
                "could not save",
            ),
            (
                keyring::Error::Invalid("PRIVATE_ATTRIBUTE".into(), "PRIVATE_TOKEN".into()),
                "could not save",
            ),
            (
                keyring::Error::BadEncoding(b"PRIVATE_TOKEN".to_vec()),
                "could not save",
            ),
        ] {
            let message = sign_in_error_message(
                &anyhow::Error::from(error).context(SignInStage::Credentials),
            );
            assert!(message.contains(expected), "{message}");
            assert!(!message.contains("PRIVATE"));
        }
        for issue in [
            CredentialStoreIssue::Bus,
            CredentialStoreIssue::Unavailable,
            CredentialStoreIssue::Activation,
            CredentialStoreIssue::Disabled,
        ] {
            let error = anyhow::anyhow!("PRIVATE_TOKEN")
                .context(issue)
                .context(SignInStage::Credentials);
            assert_eq!(sign_in_error_message(&error), issue.to_string());
        }
    }

    #[test]
    fn sign_in_stages_hide_sensitive_sources_and_keep_actionable_context() {
        for (stage, expected) in [
            (SignInStage::Token, "exchange the login response"),
            (SignInStage::Profile, "verify your account profile"),
            (SignInStage::Cleanup, "Retry sign-out cleanup"),
            (SignInStage::Credentials, "desktop credential service"),
            (SignInStage::Session, "session changed"),
        ] {
            let error = anyhow::anyhow!(
                "https://fixture.invalid/?code=CODE_SECRET access_token=TOKEN_SECRET client_secret=CLIENT_SECRET"
            ).context(stage);
            let message = sign_in_error_message(&error);
            assert!(message.contains(expected), "{message}");
            assert!(!message.contains("SECRET"));
            assert!(!message.contains("https://"));
        }
        let error = reqwest::blocking::Client::new()
            .get("http://[invalid/?code=URL_SECRET")
            .build()
            .unwrap_err();
        let message =
            sign_in_error_message(&anyhow::Error::from(error).context(SignInStage::Token));
        assert!(message.contains("exchange the login response"));
        assert!(!message.contains("URL_SECRET"));
        assert!(
            !sign_in_error_message(&anyhow::anyhow!("UNKNOWN_SECRET")).contains("UNKNOWN_SECRET")
        );
    }

    #[test]
    fn optional_public_profile_and_avatar_failures_preserve_authenticated_identity() {
        let user = || UserData {
            user_id: "42".into(),
            username: "Fixture".into(),
            email: "fixture@example.invalid".into(),
            country: "US".into(),
            preferred_language: None,
            selected_currency: None,
            is_logged_in: true,
        };
        let profile = profile_with_optional_details(
            user(),
            Err(anyhow::anyhow!(
                "optional request failed with PRIVATE_SOURCE"
            )),
            |profile| {
                assert!(profile.avatar_url.is_none());
                Ok(None)
            },
        );
        assert_eq!(profile.user_id, "42");
        assert_eq!(profile.username, "Fixture");
        assert!(profile.member_since.is_none());
        assert!(profile.avatar_path.is_none());
        let public = || PublicProfile {
            user_since: Some(123),
            avatars: Some(Avatars {
                large2x: Some("https://fixture.invalid/avatar".into()),
                large: None,
                medium2x: None,
                medium: None,
            }),
        };
        let profile = profile_with_optional_details(user(), Ok(public()), |_| {
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "PRIVATE_PATH").into())
        });
        assert_eq!(profile.user_id, "42");
        assert_eq!(profile.member_since, Some(123));
        assert!(profile.avatar_path.is_none());
        let profile = profile_with_optional_details(user(), Ok(public()), |_| {
            Ok(Some(PathBuf::from("fixture-avatar.jpg")))
        });
        assert_eq!(
            profile.avatar_path,
            Some(PathBuf::from("fixture-avatar.jpg"))
        );
    }

    #[test]
    fn durable_sign_out_and_bounded_close_do_not_wait_for_credential_service() {
        if run_in_private_process(
            "auth::tests::durable_sign_out_and_bounded_close_do_not_wait_for_credential_service",
        ) {
            return;
        }
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
        if run_in_private_process(
            "auth::tests::sign_out_survives_credential_failure_and_rejects_delayed_authentication",
        ) {
            return;
        }
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
        let failure = commit_credentials(current, true, || {
            Err(anyhow::anyhow!("KEYRING_PRIVATE_SOURCE"))
        })
        .unwrap_err();
        assert!(sign_in_error_message(&failure).contains("desktop credential service"));
        assert!(!sign_in_error_message(&failure).contains("KEYRING_PRIVATE_SOURCE"));
        assert!(
            marker.is_file(),
            "failed save must retain the sign-out barrier"
        );
        assert!(!session_is_current(current));
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
