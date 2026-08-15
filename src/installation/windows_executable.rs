use serde::Deserialize;
use std::{
    cmp::Reverse,
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsExecutableCandidate {
    pub path: PathBuf,
    pub score: u16,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsExecutableDiscovery {
    pub selected: Option<PathBuf>,
    pub candidates: Vec<WindowsExecutableCandidate>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GogGameInfo {
    game_id: String,
    root_game_id: String,
    #[serde(default)]
    play_tasks: Vec<GogPlayTask>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GogPlayTask {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    is_primary: bool,
    path: String,
}

pub fn discover_windows_executable(
    directory: &Path,
    product_id: i64,
    game_name: &str,
) -> WindowsExecutableDiscovery {
    if let Some(primary) = gog_primary_executable(directory, product_id) {
        return WindowsExecutableDiscovery {
            selected: Some(primary.clone()),
            candidates: vec![WindowsExecutableCandidate {
                path: primary,
                score: u16::MAX,
                reason: "GOG primary play task".into(),
            }],
        };
    }

    let mut candidates = Vec::new();
    collect_candidates(directory, directory, 0, game_name, &mut candidates);
    candidates.sort_by_key(|candidate| {
        let depth = candidate
            .path
            .strip_prefix(directory)
            .map_or(usize::MAX, |path| path.components().count());
        (Reverse(candidate.score), depth, candidate.path.clone())
    });
    let selected = match candidates.as_slice() {
        [first, rest @ ..]
            if first.score >= 90
                && rest
                    .first()
                    .is_none_or(|second| first.score.saturating_sub(second.score) >= 15) =>
        {
            Some(first.path.clone())
        }
        _ => None,
    };
    WindowsExecutableDiscovery {
        selected,
        candidates,
    }
}

fn gog_primary_executable(directory: &Path, product_id: i64) -> Option<PathBuf> {
    let info_path = directory.join(format!("goggame-{product_id}.info"));
    let info: GogGameInfo = serde_json::from_slice(&fs::read(info_path).ok()?).ok()?;
    let expected = product_id.to_string();
    if info.game_id != expected || info.root_game_id != expected {
        return None;
    }
    info.play_tasks
        .iter()
        .filter(|task| {
            task.kind.eq_ignore_ascii_case("FileTask") && task.category.eq_ignore_ascii_case("game")
        })
        .filter_map(|task| validated_payload_path(directory, &task.path).map(|path| (task, path)))
        .min_by_key(|(task, _)| !task.is_primary)
        .map(|(_, path)| path)
}

fn validated_payload_path(directory: &Path, relative: &str) -> Option<PathBuf> {
    let relative = PathBuf::from(relative.replace('\\', "/"));
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    let root = fs::canonicalize(directory).ok()?;
    let candidate = fs::canonicalize(directory.join(relative)).ok()?;
    (candidate.starts_with(root) && candidate.is_file()).then_some(candidate)
}

fn collect_candidates(
    root: &Path,
    directory: &Path,
    depth: u8,
    game_name: &str,
    output: &mut Vec<WindowsExecutableCandidate>,
) {
    if depth > 7 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_candidates(root, &path, depth + 1, game_name, output);
        } else if let Some(candidate) = rank_candidate(root, path, game_name) {
            output.push(candidate);
        }
    }
}

fn rank_candidate(
    root: &Path,
    path: PathBuf,
    game_name: &str,
) -> Option<WindowsExecutableCandidate> {
    let filename = path.file_name()?.to_str()?;
    if !path.is_file() || !filename.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let normalized_stem = normalized(stem);
    let normalized_game = normalized(game_name);
    let initials = initialism(game_name);
    let lower_path = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .to_ascii_lowercase();
    if [
        "unins",
        "uninstall",
        "crash",
        "report",
        "setup",
        "redist",
        "dxsetup",
        "config",
        "vcredist",
        "editor",
        "compiler",
        "repair",
        "server",
        "viewer",
        "creator",
        "benchmark",
        "tool",
    ]
    .iter()
    .any(|term| normalized_stem.contains(term))
    {
        return None;
    }

    let (mut score, reason) = if !normalized_game.is_empty() && normalized_stem == normalized_game {
        (100, "name matches the game".to_owned())
    } else if normalized_game.len() >= 4 && normalized_stem.contains(&normalized_game) {
        (85, "name contains the game title".to_owned())
    } else if initials.len() >= 2
        && (normalized_stem == initials || normalized_stem.starts_with(&initials))
    {
        (80, "name matches the game initials".to_owned())
    } else if normalized_stem == "game"
        || normalized_stem == "start"
        || normalized_stem == "launcher"
    {
        (55, "generic game launcher name".to_owned())
    } else {
        (20, "unrecognized executable".to_owned())
    };
    if lower_path
        .split('/')
        .any(|part| matches!(part, "x64" | "win64" | "64bit" | "amd64"))
        || normalized_stem.contains("x64")
        || normalized_stem.contains("64bit")
    {
        score += 20;
    }
    Some(WindowsExecutableCandidate {
        path,
        score,
        reason,
    })
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn initialism(value: &str) -> String {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ludomere-windows-executable-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn gog_primary_play_task_wins() {
        let root = temp_root("gog");
        fs::create_dir_all(root.join("x64")).unwrap();
        fs::write(root.join("x64/Grim Dawn.exe"), b"exe").unwrap();
        fs::write(root.join("AifEditor.exe"), b"exe").unwrap();
        fs::write(
            root.join("goggame-42.info"),
            r#"{"gameId":"42","rootGameId":"42","playTasks":[{"type":"FileTask","category":"game","isPrimary":true,"path":"x64/Grim Dawn.exe"}]}"#,
        ).unwrap();
        let result = discover_windows_executable(&root, 42, "Grim Dawn");
        assert_eq!(
            result.selected,
            Some(fs::canonicalize(root.join("x64/Grim Dawn.exe")).unwrap())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fallback_prefers_matching_64_bit_executable_and_excludes_tools() {
        let root = temp_root("ranking");
        fs::create_dir_all(root.join("x64")).unwrap();
        fs::write(root.join("Grim Dawn.exe"), b"exe").unwrap();
        fs::write(root.join("x64/Grim Dawn.exe"), b"exe").unwrap();
        fs::write(root.join("AifEditor.exe"), b"exe").unwrap();
        let result = discover_windows_executable(&root, 42, "grim_dawn");
        assert_eq!(result.selected, Some(root.join("x64/Grim Dawn.exe")));
        assert!(
            result
                .candidates
                .iter()
                .all(|candidate| !candidate.path.ends_with("AifEditor.exe"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn low_confidence_candidates_require_user_selection() {
        let root = temp_root("ambiguous");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("one.exe"), b"exe").unwrap();
        fs::write(root.join("two.exe"), b"exe").unwrap();
        let result = discover_windows_executable(&root, 42, "Different Game");
        assert_eq!(result.selected, None);
        assert_eq!(result.candidates.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
