use crate::domain::RemoteArtifact;
use std::path::{Path, PathBuf};

pub fn destination(
    root: &Path,
    game_slug: &str,
    dlc_slug: Option<&str>,
    artifacts: &[&RemoteArtifact],
) -> PathBuf {
    let first = artifacts[0];
    let slug = key(game_slug);
    let product_directory = if slug == "unknown" {
        first.product_id.to_string()
    } else {
        slug
    };
    let mut destination = root.join(product_directory);
    if let Some(dlc_slug) = dlc_slug {
        destination.push("dlc");
        destination.push(key(dlc_slug));
    }
    destination.push(first.kind.as_str());
    if let Some(operating_system) = first
        .operating_system
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        destination.push(key(operating_system));
    }
    if let Some(language) = first
        .language
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        destination.push(key(language));
    }
    destination
}

pub(super) fn staging_directory(
    destination: &Path,
    artifacts: &[RemoteArtifact],
    id: &str,
) -> PathBuf {
    let first = &artifacts[0];
    let levels =
        2 + usize::from(
            first
                .operating_system
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
        ) + usize::from(
            first
                .language
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
        );
    let mut root = destination;
    for _ in 0..levels {
        root = root.parent().unwrap_or(root);
    }
    if root.file_name().is_some_and(|name| name == "dlc") {
        root = root.parent().and_then(Path::parent).unwrap_or(root);
    }
    root.join(crate::identity::STAGING_DIRECTORY).join(key(id))
}

pub(super) fn key(value: &str) -> String {
    let value = value
        .chars()
        .filter_map(|character| {
            character
                .is_alphanumeric()
                .then(|| character.to_ascii_lowercase())
                .or_else(|| (character == '-' || character == '_').then_some(character))
        })
        .collect::<String>();
    if value.is_empty() {
        "unknown".into()
    } else {
        value
    }
}
