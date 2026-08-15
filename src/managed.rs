use crate::{
    domain::{ArtifactKind, Game, LibraryFile, RemoteArtifact},
    state::{DownloadJobRecord, ManagedFileRecord, StateStore},
};
use anyhow::Result;
use std::{collections::HashMap, fs, path::Path};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RebuildSummary {
    pub files: usize,
    pub matched: usize,
    pub unmatched: usize,
    pub partials: usize,
    pub ignored: usize,
}

#[derive(Clone)]
struct Product {
    id: i64,
    artifacts: Vec<RemoteArtifact>,
}

/// Rebuilds the downloaded-file index using read-only inspection of `root`.
pub fn rebuild(store: &mut StateStore, root: &Path, games: &[Game]) -> Result<RebuildSummary> {
    let jobs = store.download_jobs()?;
    let games_by_slug = games
        .iter()
        .map(|game| (game.slug.as_str(), game))
        .collect::<HashMap<_, _>>();
    let mut records = Vec::new();
    let mut summary = RebuildSummary::default();

    if root.is_dir() {
        for product_entry in fs::read_dir(root)? {
            let product_entry = product_entry?;
            let product_path = product_entry.path();
            if !product_path.is_dir() {
                summary.ignored += 1;
                continue;
            }
            let slug = product_entry.file_name().to_string_lossy().into_owned();
            let Some(game) = games_by_slug.get(slug.as_str()) else {
                tracing::debug!(path = %product_path.display(), "ignored path outside recognized managed product layout");
                summary.ignored += 1;
                continue;
            };
            for kind_entry in fs::read_dir(&product_path)? {
                let kind_entry = kind_entry?;
                let kind_path = kind_entry.path();
                if kind_entry.file_name() == "dlc" {
                    visit_dlc_root(&kind_path, game, &jobs, &mut records, &mut summary)?;
                    continue;
                }
                let Some(kind) = kind_entry.file_name().to_str().and_then(parse_kind) else {
                    tracing::debug!(path = %kind_path.display(), "ignored path outside recognized managed artifact layout");
                    summary.ignored += 1;
                    continue;
                };
                visit_kind(
                    &kind_path,
                    &slug,
                    &Product {
                        id: game.product_id,
                        artifacts: game.remote_artifacts.clone(),
                    },
                    kind,
                    &jobs,
                    &mut records,
                    &mut summary,
                )?;
            }
        }
    }
    store.replace_managed_files(&records)?;
    Ok(summary)
}

fn visit_dlc_root(
    root: &Path,
    game: &Game,
    jobs: &[DownloadJobRecord],
    records: &mut Vec<ManagedFileRecord>,
    summary: &mut RebuildSummary,
) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let dlc_path = entry.path();
        let slug = entry.file_name().to_string_lossy().into_owned();
        let Some(dlc) = game.dlcs.iter().find(|dlc| dlc.slug == slug) else {
            summary.ignored += 1;
            continue;
        };
        if !dlc_path.is_dir() {
            summary.ignored += 1;
            continue;
        }
        let product = Product {
            id: dlc.product_id,
            artifacts: dlc.remote_artifacts.clone(),
        };
        for kind_entry in fs::read_dir(dlc_path)? {
            let kind_entry = kind_entry?;
            let kind_path = kind_entry.path();
            let Some(kind) = kind_entry.file_name().to_str().and_then(parse_kind) else {
                summary.ignored += 1;
                continue;
            };
            visit_kind(&kind_path, &slug, &product, kind, jobs, records, summary)?;
        }
    }
    Ok(())
}

fn visit_kind(
    root: &Path,
    slug: &str,
    product: &Product,
    kind: ArtifactKind,
    jobs: &[DownloadJobRecord],
    records: &mut Vec<ManagedFileRecord>,
    summary: &mut RebuildSummary,
) -> Result<()> {
    let mut pending = vec![(root.to_path_buf(), Vec::<String>::new())];
    while let Some((directory, components)) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let mut child_components = components.clone();
                child_components.push(entry.file_name().to_string_lossy().into_owned());
                if child_components.len() <= 2 {
                    pending.push((path, child_components));
                } else {
                    tracing::debug!(path = %path.display(), "ignored path nested beyond managed layout");
                    summary.ignored += 1;
                }
                continue;
            }
            if !path.is_file() {
                summary.ignored += 1;
                continue;
            }
            let filename = entry.file_name().to_string_lossy().into_owned();
            if filename.ends_with(".download") {
                summary.partials += 1;
                continue;
            }
            let (os, language) = match components.as_slice() {
                [] => (None, None),
                [one] => (Some(one.clone()), None),
                [one, two] => (Some(one.clone()), Some(two.clone())),
                _ => unreachable!(),
            };
            let size = entry.metadata()?.len();
            let artifact = associate(
                slug,
                product.id,
                kind,
                os.as_deref(),
                language.as_deref(),
                &filename,
                size,
                jobs,
                &product.artifacts,
            );
            let matched = artifact.is_some();
            summary.files += 1;
            if matched {
                summary.matched += 1;
            } else {
                summary.unmatched += 1;
            }
            records.push(ManagedFileRecord {
                path,
                product_id: product.id,
                product_slug: slug.to_owned(),
                kind,
                operating_system: os,
                language,
                filename,
                size,
                artifact_path: artifact.map(|value| value.download_path.clone()),
                matched,
                present: true,
                artifact_id: artifact.map(|artifact| {
                    format!(
                        "{}:{}:{}:{}",
                        artifact.product_id,
                        artifact.kind.as_str(),
                        artifact.download_path,
                        artifact.version.as_deref().unwrap_or_default()
                    )
                }),
                job_id: None,
                version: artifact.and_then(|artifact| artifact.version.clone()),
                expected_size: artifact.and_then(|artifact| artifact.size_bytes),
                gog_checksum: None,
                verified_at: None,
                revision_id: None,
                part_id: None,
                provider_file_id: artifact.and_then(|artifact| artifact.provider_file_id.clone()),
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn associate<'a>(
    slug: &str,
    product_id: i64,
    kind: ArtifactKind,
    os: Option<&str>,
    language: Option<&str>,
    filename: &str,
    size: u64,
    jobs: &'a [DownloadJobRecord],
    artifacts: &'a [RemoteArtifact],
) -> Option<&'a RemoteArtifact> {
    let retained_filename = jobs
        .iter()
        .filter(|job| job.product_id == product_id)
        .flat_map(|job| job.completed_files.iter().zip(job.artifacts.iter()))
        .find(|(path, artifact)| {
            path.file_name().and_then(|value| value.to_str()) == Some(filename)
                && artifact.kind == kind
        })
        .map(|(_, artifact)| artifact);
    let candidates = artifacts
        .iter()
        .filter(|artifact| artifact.product_id == product_id && artifact.kind == kind)
        .collect::<Vec<_>>();
    if let Some(retained) = retained_filename {
        return current_equivalent(retained, &candidates).or(Some(retained));
    }
    candidates
        .iter()
        .copied()
        .find(|artifact| filename_of(artifact) == filename && path_matches(artifact, os, language))
        .or_else(|| {
            candidates.iter().copied().find(|artifact| {
                filename_of(artifact) == filename && artifact.size_bytes == Some(size)
            })
        })
        .or_else(|| {
            let _ = slug;
            None
        })
}

fn current_equivalent<'a>(
    retained: &RemoteArtifact,
    current: &[&'a RemoteArtifact],
) -> Option<&'a RemoteArtifact> {
    if let Some(group_id) = retained.provider_group_id.as_deref()
        && let Some(candidate) = current.iter().copied().find(|candidate| {
            candidate.provider_category == retained.provider_category
                && candidate.provider_group_id.as_deref() == Some(group_id)
                && candidate.version == retained.version
                && candidate.part_number == retained.part_number
                && candidate.size_bytes == retained.size_bytes
        })
    {
        return Some(candidate);
    }

    let mut matches = current.iter().copied().filter(|candidate| {
        candidate.provider_category == retained.provider_category
            && candidate.version == retained.version
            && optional_matches(
                candidate.operating_system.as_deref(),
                retained.operating_system.as_deref(),
            )
            && optional_matches(candidate.language.as_deref(), retained.language.as_deref())
            && candidate.part_number == retained.part_number
            && candidate.part_count == retained.part_count
    });
    let candidate = matches.next()?;
    matches.next().is_none().then_some(candidate)
}

fn filename_of(artifact: &RemoteArtifact) -> &str {
    artifact
        .download_path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(&artifact.name)
}

fn path_matches(artifact: &RemoteArtifact, os: Option<&str>, language: Option<&str>) -> bool {
    optional_matches(artifact.operating_system.as_deref(), os)
        && optional_matches(artifact.language.as_deref(), language)
}

fn optional_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected.is_none()
        || actual.is_none()
        || expected
            .zip(actual)
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn parse_kind(value: &str) -> Option<ArtifactKind> {
    match value {
        "installer" => Some(ArtifactKind::Installer),
        "patch" => Some(ArtifactKind::Patch),
        "extra" => Some(ArtifactKind::Extra),
        _ => None,
    }
}

pub fn apply_to_games(games: &mut [Game], records: &[ManagedFileRecord]) {
    for game in games {
        apply_product(
            game.product_id,
            &mut game.installers,
            &mut game.patches,
            &mut game.extras,
            records,
        );
        game.disk_usage = records
            .iter()
            .filter(|record| record.product_id == game.product_id && record.present)
            .map(|record| record.size)
            .sum();
        for dlc in &mut game.dlcs {
            let mut patches = Vec::new();
            apply_product(
                dlc.product_id,
                &mut dlc.installers,
                &mut patches,
                &mut dlc.extras,
                records,
            );
            dlc.disk_usage = records
                .iter()
                .filter(|record| record.product_id == dlc.product_id && record.present)
                .map(|record| record.size)
                .sum();
        }
    }
}

pub fn set_locations(games: &mut [Game], root: &Path) {
    for game in games {
        game.location = root.join(&game.slug);
        for dlc in &mut game.dlcs {
            dlc.location = game.location.join("dlc").join(&dlc.slug);
        }
    }
}

fn apply_product(
    product_id: i64,
    installers: &mut Vec<LibraryFile>,
    patches: &mut Vec<LibraryFile>,
    extras: &mut Vec<LibraryFile>,
    records: &[ManagedFileRecord],
) {
    installers.clear();
    patches.clear();
    extras.clear();
    for record in records
        .iter()
        .filter(|record| record.product_id == product_id && record.present)
    {
        let file = LibraryFile {
            name: if record.matched {
                record.filename.clone()
            } else {
                format!(
                    "{} — Local file — not matched to current GOG manifest",
                    record.filename
                )
            },
            path: record.path.clone(),
            size: record.size,
        };
        match record.kind {
            ArtifactKind::Installer => installers.push(file),
            ArtifactKind::Patch => patches.push(file),
            ArtifactKind::Extra => extras.push(file),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Dlc, DownloadCategory, Platforms};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> (PathBuf, StateStore, Vec<Game>) {
        let root = std::env::temp_dir().join(format!(
            "gog-managed-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let store = StateStore::open_at(&root.join("state.sqlite3")).unwrap();
        let artifact = |id, kind, path: &str| RemoteArtifact {
            product_id: id,
            kind,
            name: path.rsplit('/').next().unwrap().into(),
            language: Some("en".into()),
            operating_system: Some("linux".into()),
            version: None,
            release_date: None,
            size_label: None,
            size_bytes: Some(4),
            part_number: None,
            part_count: None,
            download_path: path.into(),
            provider_group_id: None,
            provider_file_id: None,
            provider_category: None,
        };
        let dlc = Dlc {
            product_id: 2,
            owned: true,
            slug: "bonus-pack".into(),
            title: "Bonus".into(),
            platforms: Platforms {
                linux: true,
                ..Default::default()
            },
            remote_artifacts: vec![artifact(2, ArtifactKind::Installer, "/bonus.sh")],
            ..Default::default()
        };
        let game = Game {
            product_id: 1,
            slug: "base-game".into(),
            title: "Base".into(),
            platforms: Platforms {
                linux: true,
                ..Default::default()
            },
            dlcs: vec![dlc],
            remote_artifacts: vec![
                artifact(1, ArtifactKind::Installer, "/setup.sh"),
                artifact(1, ArtifactKind::Patch, "/patch.bin"),
            ],
            ..Default::default()
        };
        (root, store, vec![game])
    }

    #[test]
    fn maps_a_retained_file_to_the_current_revision_of_the_same_gog_slot() {
        let artifact = |path: &str, file_id: &str| RemoteArtifact {
            product_id: 1,
            kind: ArtifactKind::Installer,
            name: "Dungeon Keeper Gold".into(),
            language: Some("English".into()),
            operating_system: Some("windows".into()),
            version: Some("1.01_fix".into()),
            release_date: None,
            size_label: None,
            size_bytes: Some(296_700_000),
            part_number: Some(1),
            part_count: Some(1),
            download_path: path.into(),
            provider_group_id: Some("installer_windows_en".into()),
            provider_file_id: Some(file_id.into()),
            provider_category: Some(DownloadCategory::Installer),
        };
        let retained = artifact("/old-downlink", "old-file");
        let current = artifact("/current-downlink", "current-file");

        assert_eq!(
            current_equivalent(&retained, &[&current]).map(|value| value.download_path.as_str()),
            Some("/current-downlink")
        );
    }

    #[test]
    fn does_not_promote_a_retained_file_when_gog_reuses_the_slot_for_new_bytes() {
        let artifact = |version: &str, size| RemoteArtifact {
            product_id: 1,
            kind: ArtifactKind::Installer,
            name: "Grim Dawn".into(),
            language: Some("English".into()),
            operating_system: Some("windows".into()),
            version: Some(version.into()),
            release_date: None,
            size_label: None,
            size_bytes: Some(size),
            part_number: Some(1),
            part_count: Some(3),
            download_path: "/reused-downlink".into(),
            provider_group_id: Some("installer_windows_en".into()),
            provider_file_id: Some("reused-file-id".into()),
            provider_category: Some(DownloadCategory::Installer),
        };
        let retained = artifact("1.3.0.5", 3_900_000_000);
        let current = artifact("1.3.0.6", 4_200_000_000);

        assert!(current_equivalent(&retained, &[&current]).is_none());
    }

    #[test]
    fn does_not_guess_when_multiple_current_slots_share_fallback_metadata() {
        let artifact = |path: &str| RemoteArtifact {
            product_id: 1,
            kind: ArtifactKind::Installer,
            name: "Installer".into(),
            language: Some("English".into()),
            operating_system: Some("windows".into()),
            version: Some("1.0".into()),
            release_date: None,
            size_label: None,
            size_bytes: None,
            part_number: None,
            part_count: None,
            download_path: path.into(),
            provider_group_id: None,
            provider_file_id: None,
            provider_category: Some(DownloadCategory::Installer),
        };
        let retained = artifact("/old");
        let first = artifact("/current-32-bit");
        let second = artifact("/current-64-bit");

        assert!(current_equivalent(&retained, &[&first, &second]).is_none());
    }

    #[test]
    fn indexes_native_layout_dlc_unknown_extras_and_partials_without_changing_files() {
        let (root, mut store, games) = fixture();
        let paths = [
            root.join("base-game/installer/linux/en/setup.sh"),
            root.join("base-game/patch/linux/en/patch.bin"),
            root.join("base-game/extra/readme.exe"),
            root.join("base-game/dlc/bonus-pack/installer/linux/en/bonus.sh"),
            root.join("base-game/installer/linux/en/pending.bin.download"),
        ];
        for path in &paths {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"data").unwrap();
        }
        let summary = rebuild(&mut store, &root, &games).unwrap();
        assert_eq!(
            (
                summary.files,
                summary.matched,
                summary.unmatched,
                summary.partials
            ),
            (4, 3, 1, 1)
        );
        let indexed = store.managed_files().unwrap();
        assert!(
            indexed
                .iter()
                .any(|file| file.product_id == 2 && file.filename == "bonus.sh")
        );
        assert!(
            indexed
                .iter()
                .any(|file| !file.matched && file.filename == "readme.exe")
        );
        assert!(
            !indexed
                .iter()
                .any(|file| file.filename.ends_with(".download"))
        );
        for path in &paths {
            assert_eq!(fs::read(path).unwrap(), b"data");
        }
        let again = rebuild(&mut store, &root, &games).unwrap();
        assert_eq!(again.files, 4);
        assert_eq!(
            store
                .managed_files()
                .unwrap()
                .iter()
                .filter(|file| file.present)
                .count(),
            4
        );
        fs::remove_dir_all(root).unwrap();
    }
}
