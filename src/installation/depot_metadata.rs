use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use super::marker::{InstallationMarker, InstalledLaunch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotLaunchTask {
    pub path: String,
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub launcher: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepotScriptActionKind {
    SetRegistry,
    SetIni,
    SupportData,
    SavePath,
    InstallSdb,
    Execute,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DepotScriptAction {
    pub name: String,
    pub kind: DepotScriptActionKind,
    pub arguments: Value,
    pub uninstall: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameInfo {
    game_id: String,
    root_game_id: String,
    #[serde(default)]
    play_tasks: Vec<PlayTask>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayTask {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    is_primary: bool,
    #[serde(default)]
    path: String,
    #[serde(default)]
    arguments: String,
    working_dir: Option<String>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    os_bitness: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Script {
    product_id: String,
    #[serde(default)]
    actions: Vec<RawAction>,
}

#[derive(Deserialize)]
struct RawAction {
    #[serde(default)]
    name: String,
    #[serde(default)]
    languages: Vec<String>,
    install: Option<RawOperation>,
    uninstall: Option<RawOperation>,
}

#[derive(Deserialize)]
struct RawOperation {
    action: String,
    #[serde(default)]
    arguments: Value,
}

pub fn primary_launch_task(
    bytes: &[u8],
    product_id: i64,
    language: &str,
    bitness: Option<&str>,
) -> Result<DepotLaunchTask> {
    let info: GameInfo = serde_json::from_slice(bytes).context("parsing GOG game info")?;
    let expected = product_id.to_string();
    if info.game_id != expected || info.root_game_id != expected {
        bail!("GOG game info identity does not match the base product");
    }
    let task = info
        .play_tasks
        .iter()
        .filter(|task| task.kind.eq_ignore_ascii_case("FileTask"))
        .filter(|task| {
            matches!(
                task.category.to_ascii_lowercase().as_str(),
                "game" | "launcher"
            )
        })
        .filter(|task| matches_selection(&task.languages, language))
        .filter(|task| bitness.is_none_or(|value| matches_selection(&task.os_bitness, value)))
        .min_by_key(|task| !task.is_primary)
        .context("GOG game info contains no matching launch task")?;
    validate_relative(&task.path)?;
    if let Some(directory) = task
        .working_dir
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        validate_relative(directory)?;
    }
    Ok(DepotLaunchTask {
        path: task.path.replace('\\', "/"),
        arguments: shell_words::split(&task.arguments).context("parsing GOG launch arguments")?,
        working_directory: task
            .working_dir
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| value.replace('\\', "/")),
        launcher: task.category.eq_ignore_ascii_case("launcher"),
    })
}

pub fn set_marker_launch(
    marker: &mut InstallationMarker,
    bytes: &[u8],
    language: &str,
    bitness: Option<&str>,
) -> Result<()> {
    let task = primary_launch_task(bytes, marker.product_id, language, bitness)?;
    marker.launch = Some(InstalledLaunch {
        executable: task.path,
        arguments: task.arguments,
        working_directory: task.working_directory,
    });
    marker.validate()
}

pub fn script_actions(
    bytes: &[u8],
    product_id: i64,
    language: &str,
) -> Result<Vec<DepotScriptAction>> {
    let script: Script = serde_json::from_slice(bytes).context("parsing GOG setup script")?;
    if script.product_id != product_id.to_string() {
        bail!("GOG setup script identity does not match its product");
    }
    let mut output = Vec::new();
    for action in script
        .actions
        .into_iter()
        .filter(|action| matches_selection(&action.languages, language))
    {
        if let Some(operation) = action.install {
            output.push(parse_operation(&action.name, operation, false)?);
        }
        if let Some(operation) = action.uninstall {
            output.push(parse_operation(&action.name, operation, true)?);
        }
    }
    Ok(output)
}

fn parse_operation(
    name: &str,
    operation: RawOperation,
    uninstall: bool,
) -> Result<DepotScriptAction> {
    let kind = match operation.action.as_str() {
        "setRegistry" => DepotScriptActionKind::SetRegistry,
        "setIni" => DepotScriptActionKind::SetIni,
        "supportData" => DepotScriptActionKind::SupportData,
        "savePath" => DepotScriptActionKind::SavePath,
        "installSDB" => DepotScriptActionKind::InstallSdb,
        "Execute" => DepotScriptActionKind::Execute,
        _ => bail!("unsupported required GOG setup action"),
    };
    if !operation.arguments.is_object() {
        bail!("GOG setup action arguments are not an object");
    }
    Ok(DepotScriptAction {
        name: name.to_owned(),
        kind,
        arguments: operation.arguments,
        uninstall,
    })
}

fn matches_selection(values: &[String], selected: &str) -> bool {
    let selected_base = selected.split(['-', '_']).next().unwrap_or(selected);
    values.is_empty()
        || values.iter().any(|value| {
            value == "*"
                || value.eq_ignore_ascii_case(selected)
                || value
                    .split(['-', '_'])
                    .next()
                    .is_some_and(|base| base.eq_ignore_ascii_case(selected_base))
        })
}

fn validate_relative(path: &str) -> Result<()> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.as_bytes().get(1) == Some(&b':')
        || normalized
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("unsafe GOG task path");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_primary_launch_task_and_preserves_arguments() {
        let info = br#"{"gameId":"42","rootGameId":"42","playTasks":[
          {"type":"URLTask","isPrimary":true,"url":"https://example.invalid"},
          {"type":"FileTask","category":"game","isPrimary":true,"path":"bin\\game.exe","arguments":"--name \"two words\"","workingDir":"bin","languages":["en-US"],"osBitness":["64"]}
        ]}"#;
        let task = primary_launch_task(info, 42, "en-US", Some("64")).unwrap();
        assert_eq!(task.path, "bin/game.exe");
        assert_eq!(task.arguments, ["--name", "two words"]);
        assert_eq!(task.working_directory.as_deref(), Some("bin"));
        assert!(!task.launcher);
        assert!(primary_launch_task(info, 42, "en", Some("64")).is_ok());

        let mut marker = crate::installation::marker::from_game(
            &crate::domain::InstalledGame {
                product_id: 42,
                library_id: "library".into(),
                installed_version: None,
                installation_directory: "/library/game".into(),
                installer_revision_id: None,
                installer_job_id: None,
                installer_files: Vec::new(),
                installer_complete: true,
                installer_operating_system: Some("windows".into()),
                installer_language: Some("en-US".into()),
                compatibility: None,
                primary_executable: None,
                launch_arguments: Vec::new(),
                state: crate::domain::InstallationState::Installed,
                error: None,
                installed_at: Some(1),
                verified_at: None,
                last_played_at: None,
                playtime_seconds: 0,
                created_at: 1,
                updated_at: 1,
            },
            Vec::new(),
        );
        set_marker_launch(&mut marker, info, "en-US", Some("64")).unwrap();
        let launch = marker.launch.unwrap();
        assert_eq!(launch.executable, "bin/game.exe");
        assert_eq!(launch.arguments, ["--name", "two words"]);
    }

    #[test]
    fn parses_observed_actions_and_rejects_unknown_required_actions() {
        let script = br#"{"productId":"9","actions":[
          {"name":"dlc","languages":["*"],"install":{"action":"setRegistry","arguments":{"root":"HKEY_LOCAL_MACHINE"}}},
          {"name":"other","languages":["de-DE"],"install":{"action":"setIni","arguments":{}}}
        ]}"#;
        let actions = script_actions(script, 9, "en-US").unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, DepotScriptActionKind::SetRegistry);
        let unsupported = br#"{"productId":"9","actions":[{"install":{"action":"futureAction","arguments":{}}}]}"#;
        assert!(script_actions(unsupported, 9, "en-US").is_err());
    }

    #[test]
    fn rejects_wrong_identity_and_unsafe_launch_paths() {
        assert!(
            primary_launch_task(
                br#"{"gameId":"1","rootGameId":"1","playTasks":[]}"#,
                2,
                "en-US",
                None
            )
            .is_err()
        );
        assert!(primary_launch_task(br#"{"gameId":"2","rootGameId":"2","playTasks":[{"type":"FileTask","category":"game","path":"../game.exe"}]}"#, 2, "en-US", None).is_err());
    }
}
