use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Default, Serialize)]
struct AuditSummary {
    owned_products: usize,
    structured_products: usize,
    structured_missing_with_legacy_installers: usize,
    games: usize,
    dlcs: usize,
    packages: usize,
    non_installable: usize,
    installer_groups: usize,
    patch_groups: usize,
    language_pack_groups: usize,
    bonus_groups: usize,
    parts: usize,
    missing_group_ids: usize,
    duplicate_group_ids: usize,
    missing_file_ids: usize,
    duplicate_file_ids: usize,
    old_parser_artifacts: usize,
    old_without_structured_equivalent: usize,
    structured_without_old_equivalent: usize,
    store_v2_coverage: usize,
    gamesdb_coverage: usize,
    windows_build_coverage: usize,
    macos_build_coverage: usize,
    endpoint_failures: Vec<EndpointFailure>,
    artifact_discrepancies: Vec<ArtifactDiscrepancy>,
}

#[derive(Serialize)]
struct EndpointFailure {
    product_id: i64,
    source: &'static str,
    error: String,
}

#[derive(Serialize)]
struct ArtifactDiscrepancy {
    product_id: i64,
    old_only: Vec<String>,
    structured_only: Vec<String>,
}

pub fn run() -> Result<()> {
    let token =
        crate::auth::load_saved_token()?.context("sign in to GOG before running the audit")?;
    let owned = crate::auth::fetch_owned_product_ids(&token)?;
    let client = crate::gog::client()?;
    let mut summary = AuditSummary {
        owned_products: owned.len(),
        ..Default::default()
    };
    let jobs = std::sync::Arc::new(std::sync::Mutex::new(
        owned.into_iter().collect::<std::collections::VecDeque<_>>(),
    ));
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let jobs = jobs.clone();
            let sender = sender.clone();
            let client = client.clone();
            let access_token = token.access_token.clone();
            scope.spawn(move || {
                loop {
                    let product_id = jobs.lock().ok().and_then(|mut jobs| jobs.pop_front());
                    let Some(product_id) = product_id else { break };
                    if sender
                        .send(audit_product(&client, &access_token, product_id))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for product in receiver {
            merge_summary(&mut summary, product);
        }
    });
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn audit_product(
    client: &reqwest::blocking::Client,
    access_token: &str,
    product_id: i64,
) -> AuditSummary {
    let mut summary = AuditSummary::default();
    let product = match crate::gog::product::fetch(client, product_id) {
        Ok(value) => value,
        Err(error) => {
            failure(&mut summary, product_id, "product_api", error);
            if legacy_has_installers(client, access_token, product_id) {
                summary.structured_missing_with_legacy_installers += 1;
            }
            return summary;
        }
    };
    summary.structured_products += 1;
    match product.get("game_type").and_then(serde_json::Value::as_str) {
        Some("game") => summary.games += 1,
        Some("dlc") => summary.dlcs += 1,
        Some("pack") => summary.packages += 1,
        _ => {}
    }
    if !product
        .get("is_installable")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        summary.non_installable += 1;
    }
    audit_downloads(&product, &mut summary);
    compare_legacy_manifest(client, access_token, product_id, &product, &mut summary);
    if crate::gog::store::fetch(client, product_id).is_ok() {
        summary.store_v2_coverage += 1;
    } else {
        summary.endpoint_failures.push(EndpointFailure {
            product_id,
            source: "store_v2",
            error: "request failed".into(),
        });
    }
    match crate::gog::gamesdb::fetch(client, product_id) {
        Ok(Some(_)) => summary.gamesdb_coverage += 1,
        Ok(None) => {}
        Err(_) => summary.endpoint_failures.push(EndpointFailure {
            product_id,
            source: "gamesdb",
            error: "request failed".into(),
        }),
    }
    if crate::gog::builds::fetch(client, product_id, "windows").is_ok() {
        summary.windows_build_coverage += 1;
    }
    if crate::gog::builds::fetch(client, product_id, "osx").is_ok() {
        summary.macos_build_coverage += 1;
    }
    summary
}

fn merge_summary(total: &mut AuditSummary, value: AuditSummary) {
    total.structured_products += value.structured_products;
    total.structured_missing_with_legacy_installers +=
        value.structured_missing_with_legacy_installers;
    total.games += value.games;
    total.dlcs += value.dlcs;
    total.packages += value.packages;
    total.non_installable += value.non_installable;
    total.installer_groups += value.installer_groups;
    total.patch_groups += value.patch_groups;
    total.language_pack_groups += value.language_pack_groups;
    total.bonus_groups += value.bonus_groups;
    total.parts += value.parts;
    total.missing_group_ids += value.missing_group_ids;
    total.duplicate_group_ids += value.duplicate_group_ids;
    total.missing_file_ids += value.missing_file_ids;
    total.duplicate_file_ids += value.duplicate_file_ids;
    total.old_parser_artifacts += value.old_parser_artifacts;
    total.old_without_structured_equivalent += value.old_without_structured_equivalent;
    total.structured_without_old_equivalent += value.structured_without_old_equivalent;
    total.store_v2_coverage += value.store_v2_coverage;
    total.gamesdb_coverage += value.gamesdb_coverage;
    total.windows_build_coverage += value.windows_build_coverage;
    total.macos_build_coverage += value.macos_build_coverage;
    total.endpoint_failures.extend(value.endpoint_failures);
    total
        .artifact_discrepancies
        .extend(value.artifact_discrepancies);
}

fn legacy_has_installers(
    client: &reqwest::blocking::Client,
    access_token: &str,
    product_id: i64,
) -> bool {
    client
        .get(format!(
            "https://embed.gog.com/account/gameDetails/{product_id}.json"
        ))
        .bearer_auth(access_token)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::json::<serde_json::Value>)
        .ok()
        .is_some_and(|value| {
            crate::online::normalize_download_artifacts(product_id, &value)
                .iter()
                .any(|artifact| artifact.kind == crate::domain::ArtifactKind::Installer)
        })
}

fn compare_legacy_manifest(
    client: &reqwest::blocking::Client,
    access_token: &str,
    product_id: i64,
    product: &serde_json::Value,
    summary: &mut AuditSummary,
) {
    let response = client
        .get(format!(
            "https://embed.gog.com/account/gameDetails/{product_id}.json"
        ))
        .bearer_auth(access_token)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::json::<serde_json::Value>);
    let legacy = match response {
        Ok(value) => value,
        Err(error) => {
            summary.endpoint_failures.push(EndpointFailure {
                product_id,
                source: "legacy_game_details",
                error: error.to_string(),
            });
            return;
        }
    };
    let legacy_artifacts = crate::online::normalize_download_artifacts(product_id, &legacy);
    let structured_artifacts = crate::gog::product::download_artifacts(product_id, product);
    let legacy_groups = artifact_signatures(&legacy_artifacts);
    let structured_groups = artifact_signatures(&structured_artifacts);
    summary.old_parser_artifacts += legacy_artifacts.len();
    summary.old_without_structured_equivalent +=
        legacy_groups.difference(&structured_groups).count();
    summary.structured_without_old_equivalent +=
        structured_groups.difference(&legacy_groups).count();
    let mut old_only = legacy_groups
        .difference(&structured_groups)
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    let mut structured_only = structured_groups
        .difference(&legacy_groups)
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    old_only.sort();
    structured_only.sort();
    if !old_only.is_empty() || !structured_only.is_empty() {
        summary.artifact_discrepancies.push(ArtifactDiscrepancy {
            product_id,
            old_only,
            structured_only,
        });
    }
}

fn artifact_signatures(
    artifacts: &[crate::domain::RemoteArtifact],
) -> std::collections::HashSet<String> {
    crate::download_selection::group_artifacts(artifacts)
        .into_iter()
        .map(|group| {
            format!(
                "{}|{}|{}|{}|{}|{}",
                group.kind.as_str(),
                group
                    .operating_system
                    .as_deref()
                    .unwrap_or("any")
                    .to_lowercase(),
                group
                    .language
                    .as_deref()
                    .unwrap_or("neutral")
                    .to_lowercase(),
                group.name.to_lowercase(),
                group.version.as_deref().unwrap_or("current").to_lowercase(),
                group.artifacts.len(),
            )
        })
        .collect()
}

fn audit_downloads(product: &serde_json::Value, summary: &mut AuditSummary) {
    let Some(downloads) = product.get("downloads") else {
        return;
    };
    for (field, counter) in [
        ("installers", &mut summary.installer_groups),
        ("patches", &mut summary.patch_groups),
        ("language_packs", &mut summary.language_pack_groups),
        ("bonus_content", &mut summary.bonus_groups),
    ] {
        let groups = downloads
            .get(field)
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        *counter += groups.len();
        let mut group_ids = std::collections::HashSet::new();
        for group in groups {
            if let Some(id) = provider_id(group.get("id")) {
                if !group_ids.insert(id) {
                    summary.duplicate_group_ids += 1;
                }
            } else {
                summary.missing_group_ids += 1;
            }
            let files = group
                .get("files")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_else(|| vec![group.clone()]);
            summary.parts += files.len();
            let mut file_ids = std::collections::HashSet::new();
            for file in files {
                if let Some(id) = provider_id(file.get("id")) {
                    if !file_ids.insert(id) {
                        summary.duplicate_file_ids += 1;
                    }
                } else {
                    summary.missing_file_ids += 1;
                }
            }
        }
    }
}

fn provider_id(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|id| id.to_string()))
}

fn failure(
    summary: &mut AuditSummary,
    product_id: i64,
    source: &'static str,
    error: anyhow::Error,
) {
    summary.endpoint_failures.push(EndpointFailure {
        product_id,
        source,
        error: error.to_string(),
    });
}
