use crate::domain::{ArtifactKind, DownloadCategory, RemoteArtifact};
use anyhow::{Context, Result};
use std::io::Read;

pub const EXPANSIONS: &str =
    "downloads,expanded_dlcs,description,screenshots,videos,related_products,changelog";

pub fn fetch(client: &reqwest::blocking::Client, product_id: i64) -> Result<serde_json::Value> {
    fetch_expanded(client, product_id, EXPANSIONS)
}

pub fn fetch_expanded(
    client: &reqwest::blocking::Client,
    product_id: i64,
    expansions: &str,
) -> Result<serde_json::Value> {
    let response = client
        .get(format!("https://api.gog.com/products/{product_id}"))
        .query(&[("expand", expansions)])
        .send()?
        .error_for_status()?;
    serde_json::from_slice(&bounded_metadata(response)?)
        .with_context(|| format!("parsing structured GOG product {product_id}"))
}

pub(crate) fn bounded_metadata(reader: impl Read) -> Result<Vec<u8>> {
    const MAXIMUM: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    reader.take(MAXIMUM + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAXIMUM,
        "GOG acquisition metadata exceeded its size limit"
    );
    Ok(bytes)
}

pub fn fetch_core(
    client: &reqwest::blocking::Client,
    ids: &[i64],
) -> Result<Vec<serde_json::Value>> {
    fetch_core_with(ids, &|ids| {
        client
            .get("https://api.gog.com/products")
            .query(&[(
                "ids",
                ids.iter().map(i64::to_string).collect::<Vec<_>>().join(","),
            )])
            .send()?
            .error_for_status()?
            .json()
            .context("decoding core GOG products")
    })
}

fn fetch_core_with(
    ids: &[i64],
    request: &impl Fn(&[i64]) -> Result<Vec<serde_json::Value>>,
) -> Result<Vec<serde_json::Value>> {
    let mut products = Vec::new();
    for batch in ids.chunks(50) {
        let values = match request(batch) {
            Ok(values) => values,
            Err(error)
                if batch.len() == 1
                    && http_status(&error) == Some(reqwest::StatusCode::NOT_FOUND) =>
            {
                continue;
            }
            Err(error)
                if batch.len() > 1
                    && matches!(
                        http_status(&error),
                        Some(reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::URI_TOO_LONG)
                    ) =>
            {
                for smaller in batch.chunks(if batch.len() > 10 { 10 } else { 1 }) {
                    products.extend(fetch_core_with(smaller, request)?);
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        let mut seen = std::collections::HashSet::new();
        products.extend(values.into_iter().filter(|value| {
            value
                .get("id")
                .and_then(serde_json::Value::as_i64)
                .is_some_and(|id| batch.contains(&id) && seen.insert(id))
        }));
        if batch.len() > 1 {
            let missing = batch
                .iter()
                .copied()
                .filter(|id| !seen.contains(id))
                .collect::<Vec<_>>();
            for smaller in missing.chunks(if batch.len() > 10 { 10 } else { 1 }) {
                products.extend(fetch_core_with(smaller, request)?);
            }
        }
    }
    Ok(products)
}

fn http_status(error: &anyhow::Error) -> Option<reqwest::StatusCode> {
    error.chain().find_map(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .and_then(reqwest::Error::status)
    })
}

pub fn download_artifacts(product_id: i64, product: &serde_json::Value) -> Vec<RemoteArtifact> {
    let Some(downloads) = product.get("downloads") else {
        return Vec::new();
    };
    let mut artifacts = Vec::new();
    for (field, category, kind) in [
        (
            "installers",
            DownloadCategory::Installer,
            ArtifactKind::Installer,
        ),
        ("patches", DownloadCategory::Patch, ArtifactKind::Patch),
        (
            "language_packs",
            DownloadCategory::LanguagePack,
            ArtifactKind::Extra,
        ),
        (
            "bonus_content",
            DownloadCategory::Bonus,
            ArtifactKind::Extra,
        ),
    ] {
        let Some(groups) = downloads.get(field).and_then(serde_json::Value::as_array) else {
            continue;
        };
        for group in groups {
            append_group(product_id, group, category, kind, &mut artifacts);
        }
    }
    artifacts
}

fn append_group(
    product_id: i64,
    group: &serde_json::Value,
    category: DownloadCategory,
    kind: ArtifactKind,
    output: &mut Vec<RemoteArtifact>,
) {
    let group_id = identifier(group, "id").unwrap_or_else(|| format!("unnamed-{}", output.len()));
    let name = string(group, "name").unwrap_or_else(|| "GOG download".into());
    let files = group
        .get("files")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![group.clone()]);
    let part_count = u32::try_from(files.len()).ok();
    for (index, file) in files.iter().enumerate() {
        let Some(downlink) = string(file, "downlink") else {
            continue;
        };
        let size = file.get("size").and_then(serde_json::Value::as_u64);
        output.push(RemoteArtifact {
            product_id,
            kind,
            name: name.clone(),
            language: string(group, "language_full").or_else(|| string(group, "language")),
            operating_system: string(group, "os"),
            version: string(group, "version").filter(|value| !value.is_empty()),
            release_date: string(group, "date"),
            size_label: size.map(crate::domain::human_size),
            size_bytes: size,
            part_number: u32::try_from(index + 1).ok(),
            part_count,
            download_path: downlink,
            provider_group_id: Some(group_id.clone()),
            provider_file_id: identifier(file, "id"),
            provider_category: Some(category),
        });
    }
}

fn string(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

fn identifier(value: &serde_json::Value, key: &str) -> Option<String> {
    let value = value.get(key)?;
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|id| id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquisition_metadata_is_bounded_before_json_parsing() {
        assert_eq!(
            bounded_metadata(&b"{\"downloads\":[]}"[..]).unwrap(),
            b"{\"downloads\":[]}"
        );
        assert!(bounded_metadata(std::io::repeat(b' ').take(16 * 1024 * 1024 + 1)).is_err());
        assert_eq!(
            bounded_metadata(std::io::repeat(b' ').take(16 * 1024 * 1024))
                .unwrap()
                .len(),
            16 * 1024 * 1024
        );
    }

    #[test]
    fn core_catalog_uses_fifty_and_recovers_only_missing_ids() {
        let calls = std::cell::RefCell::new(Vec::new());
        let products = fetch_core_with(&(1..=501).collect::<Vec<_>>(), &|ids| {
            calls.borrow_mut().push(ids.to_vec());
            Ok(ids
                .iter()
                .filter(|id| **id != 42 || ids.len() == 1)
                .map(|id| serde_json::json!({"id":id,"title":"Game","images":{}}))
                .collect())
        })
        .unwrap();
        assert_eq!(products.len(), 501);
        assert!(calls.borrow().iter().all(|ids| ids.len() <= 50));
        assert_eq!(
            calls.borrow().iter().filter(|ids| ids.len() == 50).count(),
            10
        );
        assert!(calls.borrow().contains(&vec![42]));
    }

    #[test]
    fn core_catalog_deduplicates_filters_unknown_and_preserves_missing() {
        let calls = std::cell::Cell::new(0);
        let products = fetch_core_with(&[1, 2], &|_| {
            calls.set(calls.get() + 1);
            Ok(vec![
                serde_json::json!({"id":1}),
                serde_json::json!({"id":1}),
                serde_json::json!({"id":999}),
            ])
        })
        .unwrap();
        assert_eq!(products, vec![serde_json::json!({"id":1})]);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn transport_failures_do_not_fan_out() {
        let calls = std::cell::Cell::new(0);
        assert!(
            fetch_core_with(&(1..=50).collect::<Vec<_>>(), &|_| {
                calls.set(calls.get() + 1);
                anyhow::bail!("transport unavailable")
            })
            .is_err()
        );
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn throttling_does_not_retry_smaller_batches() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let calls = std::cell::Cell::new(0);
        let client = reqwest::blocking::Client::new();
        let result = fetch_core_with(&(1..=50).collect::<Vec<_>>(), &|_| {
            calls.set(calls.get() + 1);
            Ok(client
                .get(format!("http://{address}"))
                .send()?
                .error_for_status()?
                .json()?)
        });
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
        server.join().unwrap();
    }

    #[test]
    fn multipart_group_retains_official_ids_and_exact_sizes() {
        let value = serde_json::json!({"downloads":{"installers":[{
            "id":"installer_windows_en", "name":"Example", "os":"windows",
            "language":"en", "language_full":"English", "version":"2.0",
            "files":[
                {"id":"en1installer0","size":10,"downlink":"/downlink/installer/en1installer0"},
                {"id":"en1installer1","size":20,"downlink":"/downlink/installer/en1installer1"}
            ]
        }]}});
        let artifacts = download_artifacts(42, &value);
        assert_eq!(artifacts.len(), 2);
        assert_eq!(
            artifacts[0].provider_group_id.as_deref(),
            Some("installer_windows_en")
        );
        assert_eq!(
            artifacts[1].provider_file_id.as_deref(),
            Some("en1installer1")
        );
        assert_eq!(
            artifacts
                .iter()
                .filter_map(|part| part.size_bytes)
                .sum::<u64>(),
            30
        );
    }
}
