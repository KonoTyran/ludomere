use crate::cloud_saves::metadata::GameCredentials;
use anyhow::{Context, Result, bail};
use flate2::{Compression, GzBuilder, read::GzDecoder};
use reqwest::{Url, blocking::Client, header};
use serde::Deserialize;
use std::io::{Read, Write};

const BASE_URL: &str = "https://cloudstorage.gog.com/v1";
const MAX_RESPONSE: usize = 256 * 1024 * 1024;
const MAX_FILES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteObject {
    pub namespace: String,
    pub path: String,
    pub size: u64,
    pub modified_at: i64,
    pub etag: String,
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
    fn account_id(&self) -> Option<&str> {
        None
    }
    fn list(&self) -> Result<Vec<RemoteObject>>;
    fn download(&self, namespace: &str, path: &str) -> Result<Vec<u8>>;
    fn upload(
        &self,
        namespace: &str,
        path: &str,
        data: &[u8],
        modified_at: i64,
    ) -> Result<RemoteObject>;
    fn download_revision(&self, _object: &RemoteObject) -> Result<Vec<u8>> {
        bail!("cloud storage does not support revision-checked exports")
    }
    fn delete_revision(&self, _object: &RemoteObject) -> Result<()> {
        bail!("cloud storage does not support revision-checked deletion")
    }
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
    fn account_id(&self) -> Option<&str> {
        Some(&self.user_id)
    }

    fn download_revision(&self, object: &RemoteObject) -> Result<Vec<u8>> {
        validate_remote_path(&object.namespace)?;
        validate_remote_path(&object.path)?;
        let revision = conditional_revision(&object.etag)?;
        let mut response = self
            .client
            .get(self.object_url(&object.namespace, &object.path)?)
            .bearer_auth(&self.access_token)
            .header(header::IF_MATCH, revision)
            .send()
            .map_err(|_| anyhow::anyhow!("cloud-save export request failed"))?
            .error_for_status()
            .map_err(|_| {
                anyhow::anyhow!("cloud save changed or could not be downloaded; refresh and retry")
            })?;
        let returned = response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .trim_matches('"');
        if returned != object.etag {
            bail!("cloud server did not confirm the selected save revision; export refused");
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE as u64)
        {
            bail!("cloud-save object exceeds the safety limit");
        }
        let mut compressed = Vec::new();
        response
            .by_ref()
            .take(MAX_RESPONSE as u64 + 1)
            .read_to_end(&mut compressed)
            .map_err(|_| anyhow::anyhow!("cloud-save export transfer failed"))?;
        if compressed.len() > MAX_RESPONSE {
            bail!("cloud-save object exceeds the safety limit");
        }
        let mut decoded = Vec::new();
        GzDecoder::new(compressed.as_slice())
            .take(MAX_RESPONSE as u64 + 1)
            .read_to_end(&mut decoded)
            .context("decoding cloud-save export")?;
        if decoded.len() > MAX_RESPONSE {
            bail!("expanded cloud-save object exceeds the safety limit");
        }
        Ok(decoded)
    }

    fn delete_revision(&self, object: &RemoteObject) -> Result<()> {
        validate_remote_path(&object.namespace)?;
        validate_remote_path(&object.path)?;
        self.client
            .delete(self.object_url(&object.namespace, &object.path)?)
            .bearer_auth(&self.access_token)
            .header(header::IF_MATCH, conditional_revision(&object.etag)?)
            .send()
            .map_err(|_| {
                anyhow::anyhow!(
                    "cloud-save deletion response was not received; refresh to check the result"
                )
            })?
            .error_for_status()
            .map_err(|_| {
                anyhow::anyhow!(
                    "cloud save changed or deletion was rejected; refresh before retrying"
                )
            })?;
        Ok(())
    }
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
        let bytes = bounded_listing(response, MAX_RESPONSE)?;
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
        let bytes = response.bytes()?;
        if bytes.len() > MAX_RESPONSE {
            bail!("cloud-save object exceeds the safety limit");
        }
        let mut decoded = Vec::new();
        GzDecoder::new(bytes.as_ref())
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
}

fn bounded_listing(reader: impl Read, maximum: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        bail!("cloud-save listing exceeds the safety limit");
    }
    Ok(bytes)
}

pub(super) fn conditional_revision(etag: &str) -> Result<String> {
    if etag.is_empty()
        || etag.starts_with("W/")
        || etag.contains(['"', '\\'])
        || !etag.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        bail!("cloud save has no usable strong revision; refresh before exporting or deleting");
    }
    Ok(format!("\"{etag}\""))
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

    #[test]
    fn chunked_listing_without_content_length_is_bounded_while_reading() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut buffer = [0_u8; 2048];
            assert!(stream.read(&mut buffer).unwrap() > 0);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n9\r\n123456789\r\n0\r\n\r\n");
        });
        let response = Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .unwrap();
        assert!(response.content_length().is_none());
        assert!(
            bounded_listing(response, 8)
                .unwrap_err()
                .to_string()
                .contains("safety limit")
        );
        worker.join().unwrap();
        assert_eq!(bounded_listing(&b"[]"[..], 2).unwrap(), b"[]");
    }
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

    fn fixture_response(
        status: &str,
        etag: Option<&str>,
        body: Vec<u8>,
    ) -> (CloudClient, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n",
            body.len(),
            etag.map_or_else(String::new, |etag| format!("ETag: \"{etag}\"\r\n"))
        );
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0_u8];
            while !request.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).unwrap() == 0 {
                    break;
                }
                request.push(byte[0]);
            }
            stream.write_all(headers.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            String::from_utf8(request).unwrap()
        });
        (
            CloudClient::new(
                Client::builder().no_proxy().build().unwrap(),
                "fixture-user".into(),
                "fixture-client".into(),
                "fixture-token".into(),
            )
            .with_base_url(format!("http://{address}/v1")),
            server,
        )
    }

    #[test]
    fn revision_download_requires_matching_response_etag_and_sends_precondition() {
        let object = RemoteObject {
            namespace: "main".into(),
            path: "save.dat".into(),
            size: 4,
            modified_at: 1,
            etag: "selected-revision".into(),
        };
        let (cloud, server) = fixture_response(
            "200 OK",
            Some("selected-revision"),
            deterministic_gzip(b"save").unwrap(),
        );
        assert_eq!(cloud.download_revision(&object).unwrap(), b"save");
        assert!(
            server
                .join()
                .unwrap()
                .to_lowercase()
                .contains("if-match: \"selected-revision\"")
        );
        for returned in [None, Some("changed-revision")] {
            let (cloud, server) =
                fixture_response("200 OK", returned, deterministic_gzip(b"save").unwrap());
            assert!(cloud.download_revision(&object).is_err());
            server.join().unwrap();
        }
    }

    #[test]
    fn deletion_sends_strong_precondition_and_redacts_failure_details() {
        let object = RemoteObject {
            namespace: "main".into(),
            path: "save.dat".into(),
            size: 4,
            modified_at: 1,
            etag: "selected-revision".into(),
        };
        let (cloud, server) = fixture_response(
            "412 Precondition Failed",
            None,
            b"sensitive service response".to_vec(),
        );
        let error = cloud.delete_revision(&object).unwrap_err().to_string();
        assert!(!error.contains("fixture-token"));
        assert!(!error.contains("sensitive service response"));
        let request = server.join().unwrap().to_lowercase();
        assert!(request.starts_with("delete "));
        assert!(request.contains("if-match: \"selected-revision\""));
        for revision in ["", "W/weak", "a\"b", "bad\r\nheader"] {
            assert!(conditional_revision(revision).is_err());
        }
    }
}
