use crate::domain::{ArtifactKind, DownloadCategory, RemoteArtifact};
use anyhow::{Context, Result};

pub const EXPANSIONS: &str =
    "downloads,expanded_dlcs,description,screenshots,videos,related_products,changelog";

pub fn fetch(client: &reqwest::blocking::Client, product_id: i64) -> Result<serde_json::Value> {
    client
        .get(format!("https://api.gog.com/products/{product_id}"))
        .query(&[("expand", EXPANSIONS)])
        .send()?
        .error_for_status()?
        .json()
        .with_context(|| format!("parsing structured GOG product {product_id}"))
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
