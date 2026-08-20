use crate::cloud_saves::metadata::GameCredentials;
use anyhow::{Context, Result, bail};
use flate2::{Compression, GzBuilder, read::GzDecoder};
use reqwest::{Url, blocking::Client, header};
use serde::Deserialize;
use std::io::{Read, Write};

const BASE_URL: &str = "https://cloudstorage.gog.com/v1";
const MAX_RESPONSE: usize = 256 * 1024 * 1024;
const MAX_FILES: usize = 10_000;
pub const DELETION_ETAG: &str = "aadd86936a80ee8a369579c3926f1b3c";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteObject {
    pub namespace: String,
    pub path: String,
    pub size: u64,
    pub modified_at: i64,
    pub etag: String,
}

impl RemoteObject {
    pub fn is_deleted(&self) -> bool {
        self.etag.eq_ignore_ascii_case(DELETION_ETAG)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct ListedObject {
    #[serde(default, alias = "name", alias = "path")]
    path: String,
    #[serde(default, alias = "bytes")]
    size: u64,
    #[serde(default, alias = "last_modified", alias = "lastModified")]
    modified_at: serde_json::Value,
    #[serde(default, alias = "hash")]
    etag: String,
    #[serde(default)]
    namespace: String,
}

pub fn client() -> Result<Client> {
    Client::builder()
        .user_agent(crate::identity::USER_AGENT)
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("creating cloud-save HTTP client")
}

pub fn exchange_scoped_token(
    client: &Client,
    refresh_token: &str,
    credentials: &GameCredentials,
) -> Result<String> {
    let response: TokenResponse = client
        .get("https://auth.gog.com/token")
        .query(&[
            ("client_id", credentials.client_id.as_str()),
            ("client_secret", credentials.client_secret.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("without_new_session", "1"),
        ])
        .send()
        .map_err(|_| anyhow::anyhow!("game-scoped authorization request failed"))?
        .error_for_status()
        .map_err(|_| anyhow::anyhow!("game-scoped authorization was rejected"))?
        .json()
        .map_err(|_| anyhow::anyhow!("game-scoped authorization response is malformed"))?;
    Ok(response.access_token)
}

pub trait Storage {
    fn list(&self) -> Result<Vec<RemoteObject>>;
    fn download(&self, namespace: &str, path: &str) -> Result<Vec<u8>>;
    fn upload(
        &self,
        namespace: &str,
        path: &str,
        data: &[u8],
        modified_at: i64,
    ) -> Result<RemoteObject>;
    fn delete(&self, namespace: &str, path: &str, etag: Option<&str>) -> Result<()>;
}

pub struct CloudClient {
    client: Client,
    user_id: String,
    client_id: String,
    access_token: String,
    base_url: String,
}

impl CloudClient {
    pub fn new(client: Client, user_id: String, client_id: String, access_token: String) -> Self {
        Self {
            client,
            user_id,
            client_id,
            access_token,
            base_url: BASE_URL.into(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    fn url(&self, suffix: &[&str]) -> Result<Url> {
        let mut url = Url::parse(&self.base_url)?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("invalid cloud-storage endpoint"))?;
            segments
                .pop_if_empty()
                .push(&self.user_id)
                .push(&self.client_id);
            for segment in suffix {
                segments.push(segment);
            }
        }
        Ok(url)
    }

    fn object_url(&self, namespace: &str, path: &str) -> Result<Url> {
        let mut segments = vec![namespace];
        segments.extend(path.split('/'));
        self.url(&segments)
    }

    fn listing_url(&self) -> Result<Url> {
        let mut url = self.url(&[])?;
        url.query_pairs_mut().append_pair("format", "json");
        Ok(url)
    }
}

impl Storage for CloudClient {
    fn list(&self) -> Result<Vec<RemoteObject>> {
        let response = self
            .client
            .get(self.listing_url()?)
            .bearer_auth(&self.access_token)
            .send()?
            .error_for_status()?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE as u64)
        {
            bail!("cloud-save listing exceeds the safety limit");
        }
        let bytes = response.bytes()?;
        if bytes.len() > MAX_RESPONSE {
            bail!("cloud-save listing exceeds the safety limit");
        }
        let listed: Vec<ListedObject> =
            serde_json::from_slice(&bytes).context("decoding cloud-save listing")?;
        if listed.len() > MAX_FILES {
            bail!("cloud-save listing contains too many files");
        }
        listed.into_iter().map(remote_object_from_listing).collect()
    }

    fn download(&self, namespace: &str, path: &str) -> Result<Vec<u8>> {
        validate_remote_path(path)?;
        let response = self
            .client
            .get(self.object_url(namespace, path)?)
            .bearer_auth(&self.access_token)
            .send()?
            .error_for_status()?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE as u64)
        {
            bail!("cloud-save object exceeds the safety limit");
        }
        let bytes = read_download(response, MAX_RESPONSE)?;
        if bytes.len() > MAX_RESPONSE {
            bail!("cloud-save object exceeds the safety limit");
        }
        let mut decoded = Vec::new();
        GzDecoder::new(bytes.as_slice())
            .take(MAX_RESPONSE as u64 + 1)
            .read_to_end(&mut decoded)?;
        if decoded.len() > MAX_RESPONSE {
            bail!("expanded cloud-save object exceeds the safety limit");
        }
        Ok(decoded)
    }

    fn upload(
        &self,
        namespace: &str,
        path: &str,
        data: &[u8],
        modified_at: i64,
    ) -> Result<RemoteObject> {
        validate_remote_path(path)?;
        if data.len() > MAX_RESPONSE {
            bail!("cloud-save object exceeds the safety limit");
        }
        let compressed = deterministic_gzip(data)?;
        let etag = format!("{:x}", md5::compute(&compressed));
        self.client
            .put(self.object_url(namespace, path)?)
            .bearer_auth(&self.access_token)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header("X-Object-Meta-LocalLastModified", modified_at)
            .header(header::ETAG, &etag)
            .body(compressed)
            .send()?
            .error_for_status()?;
        Ok(RemoteObject {
            namespace: namespace.into(),
            path: path.into(),
            size: data.len() as u64,
            modified_at,
            etag,
        })
    }

    fn delete(&self, namespace: &str, path: &str, etag: Option<&str>) -> Result<()> {
        validate_remote_path(path)?;
        let mut request = self
            .client
            .delete(self.object_url(namespace, path)?)
            .bearer_auth(&self.access_token);
        if let Some(etag) = etag.filter(|etag| !etag.is_empty()) {
            request = request.header(header::IF_MATCH, etag);
        }
        request
            .send()
            .map_err(|_| anyhow::anyhow!("cloud-save deletion request failed"))?
            .error_for_status()
            .map_err(|error| {
                if error.status() == Some(reqwest::StatusCode::PRECONDITION_FAILED) {
                    anyhow::anyhow!("cloud-save object changed before deletion")
                } else {
                    anyhow::anyhow!("cloud-save deletion was rejected")
                }
            })?;
        Ok(())
    }
}

fn read_download(mut response: reqwest::blocking::Response, maximum: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        crate::download::acquire_bandwidth(read as u64);
        if bytes.len().saturating_add(read) > maximum {
            bail!("cloud-save object exceeds the safety limit");
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(bytes)
}

fn remote_object_from_listing(object: ListedObject) -> Result<RemoteObject> {
    let (namespace, path) = if object.namespace.is_empty() {
        object
            .path
            .split_once('/')
            .map(|(namespace, path)| (namespace.to_owned(), path.to_owned()))
            .context("cloud-save listing contains an object without a namespace")?
    } else {
        (object.namespace, object.path)
    };
    validate_remote_path(&namespace)?;
    validate_remote_path(&path)?;
    let modified_at = match object.modified_at {
        serde_json::Value::Number(value) => value.as_i64(),
        serde_json::Value::String(value) => value.parse().ok().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(&value)
                .map(|time| time.timestamp())
                .ok()
                .or_else(|| {
                    chrono::NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f")
                        .map(|time| time.and_utc().timestamp())
                        .ok()
                })
        }),
        _ => None,
    }
    .context("cloud-save listing has an invalid modification time")?;
    Ok(RemoteObject {
        namespace,
        path,
        size: object.size,
        modified_at,
        etag: object.etag.trim_matches('"').into(),
    })
}

pub fn deterministic_gzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn validate_remote_path(path: &str) -> Result<()> {
    let path = std::path::Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        bail!("cloud-save service returned an unsafe path");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};
    #[test]
    fn gzip_is_deterministic() {
        assert_eq!(
            deterministic_gzip(b"save").unwrap(),
            deterministic_gzip(b"save").unwrap()
        );
    }
    #[test]
    fn url_encodes_path_segments() {
        let client = CloudClient::new(
            client().unwrap(),
            "user".into(),
            "client".into(),
            "secret".into(),
        )
        .with_base_url("http://localhost/v1".into());
        assert!(
            client
                .object_url("slot one", "日本語/save.dat")
                .unwrap()
                .as_str()
                .contains("slot%20one/%E6%97%A5%E6%9C%AC%E8%AA%9E/save.dat")
        );
        assert_eq!(client.listing_url().unwrap().query(), Some("format=json"));
    }

    #[test]
    fn parses_swift_json_listing_objects() {
        let listed: ListedObject = serde_json::from_str(
            r#"{"name":"saves/main/character.sav","bytes":42,"hash":"etag","last_modified":"2026-08-15T16:10:54.833818"}"#,
        )
        .unwrap();
        assert_eq!(
            remote_object_from_listing(listed).unwrap(),
            RemoteObject {
                namespace: "saves".into(),
                path: "main/character.sav".into(),
                size: 42,
                modified_at: 1_786_810_254,
                etag: "etag".into(),
            }
        );
    }

    #[test]
    fn delete_uses_encoded_object_path_and_revision_precondition() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap();
            sender
                .send(String::from_utf8_lossy(&request[..read]).into_owned())
                .unwrap();
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        let cloud = CloudClient::new(
            client().unwrap(),
            "user".into(),
            "client".into(),
            "secret".into(),
        )
        .with_base_url(format!("http://{address}/v1"));
        cloud
            .delete("save slot", "profile one/save.dat", Some("revision"))
            .unwrap();
        let request = receiver.recv().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("delete /v1/user/client/save%20slot/profile%20one/save.dat"));
        assert!(request.contains("if-match: revision"));
        server.join().unwrap();
    }
}
