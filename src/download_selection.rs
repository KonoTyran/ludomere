use crate::{
    domain::{ArtifactKind, RemoteArtifact},
    download,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactGroup {
    pub job_id: String,
    pub product_id: i64,
    pub kind: ArtifactKind,
    pub name: String,
    pub operating_system: Option<String>,
    pub language: Option<String>,
    pub version: Option<String>,
    pub release_date: Option<String>,
    pub artifacts: Vec<RemoteArtifact>,
    pub total_size: Option<u64>,
}

pub fn group_artifacts(artifacts: &[RemoteArtifact]) -> Vec<ArtifactGroup> {
    let mut grouped = BTreeMap::<String, Vec<RemoteArtifact>>::new();
    for artifact in artifacts {
        let key = if let Some(group_id) = &artifact.provider_group_id {
            format!(
                "official|{}|{:?}|{}|{:?}",
                artifact.product_id, artifact.provider_category, group_id, artifact.version
            )
        } else if artifact.part_count.is_some() {
            format!(
                "{}|{}|{:?}|{:?}|{:?}|{:?}",
                artifact.product_id,
                artifact
                    .name
                    .split(" (Part ")
                    .next()
                    .unwrap_or(&artifact.name),
                artifact.kind,
                artifact.language,
                artifact.operating_system,
                artifact.version,
            )
        } else {
            format!(
                "file|{}|{}|{:?}|{:?}|{:?}|{:?}",
                artifact.product_id,
                artifact.download_path,
                artifact.kind,
                artifact.language,
                artifact.operating_system,
                artifact.version,
            )
        };
        grouped.entry(key).or_default().push(artifact.clone());
    }

    let mut groups = grouped
        .into_values()
        .map(|mut artifacts| {
            artifacts.sort_by_key(|artifact| artifact.part_number.unwrap_or(0));
            let first = &artifacts[0];
            let refs = artifacts.iter().collect::<Vec<_>>();
            ArtifactGroup {
                job_id: download::job_id(&refs),
                product_id: first.product_id,
                kind: first.kind,
                name: first
                    .name
                    .split(" (Part ")
                    .next()
                    .unwrap_or(&first.name)
                    .to_owned(),
                operating_system: first.operating_system.clone(),
                language: first.language.clone(),
                version: first.version.clone(),
                release_date: first.release_date.clone(),
                total_size: artifacts
                    .iter()
                    .map(|artifact| artifact.size_bytes)
                    .collect::<Option<Vec<_>>>()
                    .map(|sizes| sizes.into_iter().sum()),
                artifacts,
            }
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(|group| {
        (
            artifact_kind_rank(group.kind),
            operating_system_rank(group.operating_system.as_deref()),
            group.language.as_deref().unwrap_or_default().to_lowercase(),
            group.name.to_lowercase(),
        )
    });
    groups
}

fn artifact_kind_rank(kind: ArtifactKind) -> u8 {
    match kind {
        ArtifactKind::Installer => 0,
        ArtifactKind::Patch => 1,
        ArtifactKind::Extra => 2,
    }
}

pub fn operating_system_rank(operating_system: Option<&str>) -> u8 {
    let normalized = operating_system.unwrap_or_default().to_ascii_lowercase();
    let host_matches = match std::env::consts::OS {
        "linux" => normalized == "linux",
        "windows" => matches!(normalized.as_str(), "windows" | "win"),
        "macos" => matches!(normalized.as_str(), "mac" | "macos" | "osx"),
        _ => false,
    };
    if host_matches {
        return 0;
    }
    match normalized.as_str() {
        "linux" => 1,
        "windows" | "win" => 2,
        "mac" | "macos" | "osx" => 3,
        _ => 4,
    }
}

impl ArtifactGroup {
    pub fn release_sort_key(&self) -> &str {
        self.release_date.as_deref().unwrap_or_default()
    }
}

pub fn matches_preferences(
    group: &ArtifactGroup,
    operating_systems: &BTreeSet<String>,
    languages: &BTreeSet<String>,
) -> bool {
    let os_matches = group.operating_system.as_ref().is_none_or(|os| {
        operating_systems
            .iter()
            .any(|selected| selected.eq_ignore_ascii_case(os))
    });
    let language_matches = languages.is_empty()
        || group.language.as_ref().is_none_or(|language| {
            languages
                .iter()
                .any(|selected| selected.eq_ignore_ascii_case(language))
        });
    os_matches && language_matches
}

pub fn available_languages<'a>(groups: impl Iterator<Item = &'a ArtifactGroup>) -> Vec<String> {
    let mut languages = groups
        .filter_map(|group| group.language.clone())
        .filter(|language| !language.trim().is_empty())
        .collect::<Vec<_>>();
    languages.sort_by_key(|language| language.to_lowercase());
    languages.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    languages
}

pub fn default_languages(available: &[String], configured: Option<&str>) -> BTreeSet<String> {
    if let Some(configured) = configured
        && let Some(language) = find_language(available, configured)
    {
        return [language].into_iter().collect();
    }

    let locale = std::env::var("LC_ALL")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("LC_MESSAGES").ok())
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_default();
    let locale = locale
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .split(['_', '-'])
        .next()
        .unwrap_or_default();
    let mut selected = BTreeSet::new();
    if !locale.is_empty()
        && let Some(language) = find_language(available, locale)
    {
        selected.insert(language);
    }
    if let Some(english) =
        find_language(available, "english").or_else(|| find_language(available, "en"))
    {
        selected.insert(english);
    }
    if selected.is_empty()
        && let Some(first) = available.first()
    {
        selected.insert(first.clone());
    }
    selected
}

fn find_language(available: &[String], requested: &str) -> Option<String> {
    let requested = requested.to_lowercase();
    available
        .iter()
        .find(|language| {
            let language = language.to_lowercase();
            language == requested
                || language.starts_with(&format!("{requested} "))
                || requested.starts_with(&format!("{language} "))
                || (requested == "en" && language.contains("english"))
                || (requested == "english" && language == "en")
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(name: &str, part: Option<u32>, version: &str) -> RemoteArtifact {
        RemoteArtifact {
            product_id: 42,
            kind: ArtifactKind::Installer,
            name: name.into(),
            language: Some("English".into()),
            operating_system: Some("windows".into()),
            version: Some(version.into()),
            release_date: None,
            size_label: None,
            size_bytes: Some(100),
            part_number: part,
            part_count: part.map(|_| 2),
            download_path: format!("/{name}"),
            provider_group_id: None,
            provider_file_id: None,
            provider_category: None,
        }
    }

    #[test]
    fn groups_multipart_downloads_and_sums_size() {
        let groups = group_artifacts(&[
            artifact("Game (Part 1 of 2)", Some(1), "1.0"),
            artifact("Game (Part 2 of 2)", Some(2), "1.0"),
        ]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].artifacts.len(), 2);
        assert_eq!(groups[0].total_size, Some(200));
        assert_eq!(groups[0].product_id, 42);
    }

    #[test]
    fn keeps_versions_separate() {
        let groups =
            group_artifacts(&[artifact("Game", None, "1.0"), artifact("Game", None, "2.0")]);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn configured_language_is_the_default() {
        let available = vec!["Deutsch".into(), "English".into()];
        assert_eq!(
            default_languages(&available, Some("Deutsch")),
            ["Deutsch".to_owned()].into_iter().collect()
        );
    }

    #[test]
    fn preference_matching_supports_multiple_operating_systems_and_languages() {
        let group = group_artifacts(&[artifact("Game", None, "1.0")])
            .pop()
            .unwrap();
        let operating_systems = ["linux".to_owned(), "windows".to_owned()]
            .into_iter()
            .collect();
        let languages = ["Deutsch".to_owned(), "English".to_owned()]
            .into_iter()
            .collect();
        assert!(matches_preferences(&group, &operating_systems, &languages));
    }

    #[test]
    fn empty_language_selection_matches_any_language() {
        let groups = group_artifacts(&[artifact("Installer", None, "1.0")]);
        let operating_systems = ["windows".to_string()].into_iter().collect();
        assert!(matches_preferences(
            &groups[0],
            &operating_systems,
            &BTreeSet::new()
        ));
    }

    #[test]
    fn host_operating_system_has_first_priority() {
        assert_eq!(operating_system_rank(Some(std::env::consts::OS)), 0);
    }
}
