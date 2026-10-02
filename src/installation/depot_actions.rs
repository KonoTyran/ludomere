use super::depot_metadata::{DepotScriptAction, DepotScriptActionKind};
use crate::compatibility::{CompatibilityBackend, CompatibilityRunRequest, UmuProfile};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn materialize_support<F, C>(
    manifest: &crate::gog::depot_manifest::DepotManifest,
    operation_staging: &Path,
    fetch: F,
    cancelled: C,
) -> Result<Vec<PathBuf>>
where
    F: FnMut(
        &[crate::download::depot::ChunkWrite<'_>],
        &std::fs::File,
        &mut dyn FnMut(usize) -> Result<()>,
    ) -> Result<()>,
    C: FnMut() -> bool,
{
    if manifest.entries.iter().any(|entry| {
        !matches!(entry, crate::gog::depot_manifest::DepotEntry::File(file) if file.support)
    }) {
        bail!("support manifest contains a publishable depot entry");
    }
    let staging = support_staging(operation_staging)?;
    let journal = support_journal(operation_staging)?;
    crate::download::depot::materialize_streamed_controlled(
        manifest,
        &staging,
        &journal,
        &HashSet::new(),
        fetch,
        cancelled,
    )
}

pub(crate) fn pending_support_chunks(
    manifest: &crate::gog::depot_manifest::DepotManifest,
    operation_staging: &Path,
) -> Result<Vec<crate::gog::depot_manifest::DepotChunk>> {
    crate::download::depot::pending_chunks(
        manifest,
        &support_staging(operation_staging)?,
        &support_journal(operation_staging)?,
        &HashSet::new(),
    )
}

pub fn remove_support_staging(operation_staging: &Path) -> Result<()> {
    let staging = support_staging(operation_staging)?;
    if fs::symlink_metadata(&staging).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!("support staging directory is a symlink");
    }
    if staging.exists() {
        fs::remove_dir_all(staging)?;
    }
    let journal = support_journal(operation_staging)?;
    if journal.exists() {
        fs::remove_file(journal)?;
    }
    Ok(())
}

pub fn support_staging(operation_staging: &Path) -> Result<PathBuf> {
    if operation_staging.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("staging"))
        || operation_staging
            .extension()
            .and_then(|value| value.to_str())
            != Some("json")
    {
        bail!("invalid depot operation journal path");
    }
    reject_symlink_path(operation_staging)?;
    let library = operation_staging
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("depot journal has no library root")?;
    let slug = operation_staging
        .file_stem()
        .context("depot journal has no game slug")?;
    Ok(library.join(slug).join(".ludomere-support.part"))
}

fn support_journal(operation_staging: &Path) -> Result<PathBuf> {
    let parent = operation_staging
        .parent()
        .context("depot journal has no parent")?;
    let name = operation_staging
        .file_name()
        .context("depot journal has no file name")?;
    Ok(parent.join(format!("{}.support", name.to_string_lossy())))
}

#[derive(Clone)]
pub struct ActionContext {
    pub operation_id: Option<String>,
    pub product_id: i64,
    pub app: PathBuf,
    pub support: PathBuf,
    pub prefix: PathBuf,
    pub windows_app: String,
    pub profile: UmuProfile,
    pub log_path: PathBuf,
    pub galaxy_setup: Option<GalaxySetup>,
}

#[derive(Clone)]
pub struct GalaxySetup {
    pub executable: String,
    pub arguments: String,
    pub language: String,
    pub build_id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryCommand {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub script: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryArguments {
    root: String,
    subkey: String,
    #[serde(default)]
    value_name: String,
    #[serde(default, alias = "ValueData")]
    value_data: String,
    #[serde(default)]
    value_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IniArguments {
    filename: String,
    section: String,
    key_name: String,
    key_value: serde_json::Value,
    #[serde(default)]
    conditions: serde_json::Value,
}

#[derive(Deserialize)]
struct SupportArguments {
    #[serde(default)]
    source: Option<String>,
    target: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    conditions: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SdbArguments {
    sdb_file: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteArguments {
    executable: String,
    #[serde(default, alias = "argument", alias = "parameters")]
    arguments: String,
    working_dir: Option<String>,
}

pub fn execute_actions(
    backend: &dyn CompatibilityBackend,
    context: &ActionContext,
    actions: &[DepotScriptAction],
    uninstall: bool,
) -> Result<()> {
    execute_actions_controlled(backend, context, actions, uninstall, &|| false)
}

pub(crate) fn execute_actions_controlled(
    backend: &dyn CompatibilityBackend,
    context: &ActionContext,
    actions: &[DepotScriptAction],
    uninstall: bool,
    stopped: &impl Fn() -> bool,
) -> Result<()> {
    for action in actions
        .iter()
        .filter(|action| action.uninstall == uninstall)
    {
        anyhow::ensure!(!stopped(), "GOG setup cancelled");
        crate::compatibility::append_step_log(
            &context.log_path,
            &format!(
                "GOG script action: {} {} ({:?})",
                if uninstall { "uninstall" } else { "install" },
                action.name,
                action.kind
            ),
        )?;
        let result = match action.kind {
            DepotScriptActionKind::SetRegistry => {
                execute_registry(backend, context, action, stopped)
            }
            DepotScriptActionKind::SetIni => execute_ini(context, action),
            DepotScriptActionKind::SupportData => execute_support(context, action),
            DepotScriptActionKind::SavePath => validate_save_path(context, action),
            DepotScriptActionKind::InstallSdb => execute_sdb(backend, context, action, stopped),
            DepotScriptActionKind::Execute => execute_program(backend, context, action, stopped),
        };
        if let Err(error) = result {
            crate::compatibility::append_step_log(
                &context.log_path,
                &format!("GOG script action failed: {}: {error:#}", action.name),
            )?;
            return Err(error)
                .with_context(|| format!("Required GOG setup action {:?} failed", action.name));
        }
        crate::compatibility::append_step_log(
            &context.log_path,
            &format!("GOG script action complete: {}", action.name),
        )?;
    }
    Ok(())
}

pub fn registry_command(
    context: &ActionContext,
    action: &DepotScriptAction,
) -> Result<RegistryCommand> {
    if action.kind != DepotScriptActionKind::SetRegistry {
        bail!("GOG setup action is not a registry operation");
    }
    let value: RegistryArguments =
        serde_json::from_value(action.arguments.clone()).context("parsing GOG registry action")?;
    let root = match value.root.to_ascii_uppercase().as_str() {
        "HKEY_LOCAL_MACHINE" | "HKLM" => "HKLM",
        "HKEY_CURRENT_USER" | "HKCU" => "HKCU",
        _ => bail!("unsupported GOG registry root"),
    };
    let subkey = expand_text(context, &value.subkey);
    validate_registry_component(&subkey)?;
    let value_name = expand_text(context, &value.value_name);
    if value_name.contains(['\0', '\r', '\n']) {
        bail!("unsafe GOG registry value name");
    }
    let key = format!(r"{root}\{}", subkey.replace('/', r"\"));
    let name = if value_name.is_empty() {
        "@".into()
    } else {
        format!(
            "\"{}\"",
            value_name.replace('\\', "\\\\").replace('"', "\\\"")
        )
    };
    let key_only =
        value.value_name.is_empty() && value.value_data.is_empty() && value.value_type.is_empty();
    let data = if action.uninstall {
        "-".into()
    } else if key_only {
        String::new()
    } else {
        registry_script_value(&value.value_type, &expand_text(context, &value.value_data))?
    };
    Ok(RegistryCommand {
        executable: context.prefix.join("drive_c/windows/regedit.exe"),
        arguments: vec!["/S".into(), r"C:\ludomere-registry.reg".into()],
        script: if key_only && action.uninstall {
            format!("Windows Registry Editor Version 5.00\r\n\r\n[-{key}]\r\n")
        } else if key_only {
            format!("Windows Registry Editor Version 5.00\r\n\r\n[{key}]\r\n")
        } else {
            format!("Windows Registry Editor Version 5.00\r\n\r\n[{key}]\r\n{name}={data}\r\n")
        },
    })
}

fn execute_registry(
    backend: &dyn CompatibilityBackend,
    context: &ActionContext,
    action: &DepotScriptAction,
    stopped: &impl Fn() -> bool,
) -> Result<()> {
    let command = registry_command(context, action)?;
    let script = context.prefix.join("drive_c/ludomere-registry.reg");
    log_path(context, "registry script", &script)?;
    fs::write(&script, &command.script)?;
    let result = run(
        backend,
        context,
        command.executable,
        command.arguments,
        None,
        stopped,
    );
    let _ = fs::remove_file(script);
    result
}

fn registry_script_value(kind: &str, value: &str) -> Result<String> {
    match registry_type(kind)? {
        "REG_SZ" => Ok(format!(
            "\"{}\"",
            value.replace('\\', "\\\\").replace('"', "\\\"")
        )),
        "REG_DWORD" => Ok(format!(
            "dword:{:08x}",
            parse_registry_integer(value)? as u32
        )),
        "REG_QWORD" => Ok(format!(
            "hex(b):{}",
            parse_registry_integer(value)?
                .to_le_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(",")
        )),
        "REG_EXPAND_SZ" => Ok(format!("hex(2):{}", utf16_hex(value, 1))),
        "REG_MULTI_SZ" => Ok(format!("hex(7):{}", utf16_hex(value, 2))),
        "REG_BINARY" => {
            let compact = value
                .chars()
                .filter(|character| character.is_ascii_hexdigit())
                .collect::<String>();
            if compact.len() % 2 != 0 {
                bail!("invalid GOG binary registry value");
            }
            Ok(format!(
                "hex:{}",
                compact
                    .as_bytes()
                    .chunks(2)
                    .map(|pair| std::str::from_utf8(pair).unwrap().to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(",")
            ))
        }
        _ => unreachable!(),
    }
}

fn utf16_hex(value: &str, terminators: usize) -> String {
    value
        .encode_utf16()
        .chain(std::iter::repeat_n(0, terminators))
        .flat_map(u16::to_le_bytes)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_registry_integer(value: &str) -> Result<u64> {
    if let Some(value) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Ok(u64::from_str_radix(value, 16)?)
    } else {
        Ok(value.parse()?)
    }
}

fn execute_ini(context: &ActionContext, action: &DepotScriptAction) -> Result<()> {
    let value: IniArguments =
        serde_json::from_value(action.arguments.clone()).context("parsing GOG INI action")?;
    let path = resolve_path(context, &value.filename)?;
    log_path(context, "INI target", &path)?;
    reject_symlink_path(&path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let scalar = match value.key_value {
        serde_json::Value::String(value) => value,
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        _ => bail!("unsupported GOG INI value"),
    };
    let original = fs::read_to_string(&path).unwrap_or_default();
    if condition_only_once(&value.conditions)
        && ini_has_key(&original, &value.section, &value.key_name)
    {
        return Ok(());
    }
    let updated = set_ini_value(&original, &value.section, &value.key_name, &scalar)?;
    fs::write(path, updated)?;
    Ok(())
}

fn ini_has_key(input: &str, section: &str, key: &str) -> bool {
    let heading = format!("[{section}]");
    let mut in_section = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed.eq_ignore_ascii_case(&heading);
        } else if in_section
            && line
                .split_once('=')
                .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key))
        {
            return true;
        }
    }
    false
}

fn execute_support(context: &ActionContext, action: &DepotScriptAction) -> Result<()> {
    let value: SupportArguments = serde_json::from_value(action.arguments.clone())
        .context("parsing GOG support-data action")?;
    if !conditions_supported(&value.conditions) {
        bail!("unsupported GOG support-data condition");
    }
    let target = resolve_path(context, &value.target)?;
    log_path(context, "support-data target", &target)?;
    reject_symlink_path(&target)?;
    if condition_only_once(&value.conditions) && target.exists() {
        return Ok(());
    }
    match (value.kind.as_str(), value.source) {
        ("folder", None) => fs::create_dir_all(target)?,
        ("folder", Some(source)) => {
            let source = resolve_path(context, &source)?;
            log_path(context, "support-data source", &source)?;
            copy_tree(&source, &target, value.overwrite)?
        }
        ("file", Some(source)) => {
            let source = resolve_path(context, &source)?;
            log_path(context, "support-data source", &source)?;
            copy_file(&source, &target, value.overwrite)?
        }
        _ => bail!("unsupported GOG support-data shape"),
    }
    Ok(())
}

fn validate_save_path(context: &ActionContext, action: &DepotScriptAction) -> Result<()> {
    let path = action
        .arguments
        .get("savePath")
        .and_then(serde_json::Value::as_str)
        .context("GOG save-path action has no path")?;
    let path = resolve_path(context, path)?;
    log_path(context, "save path", &path)
}

fn execute_sdb(
    backend: &dyn CompatibilityBackend,
    context: &ActionContext,
    action: &DepotScriptAction,
    stopped: &impl Fn() -> bool,
) -> Result<()> {
    let value: SdbArguments =
        serde_json::from_value(action.arguments.clone()).context("parsing GOG SDB action")?;
    let file = resolve_path(context, &value.sdb_file)?;
    log_path(context, "SDB file", &file)?;
    let executable = context.prefix.join("drive_c/windows/system32/sdbinst.exe");
    run(
        backend,
        context,
        executable,
        vec![file.to_string_lossy().into_owned()],
        None,
        stopped,
    )
}

fn execute_program(
    backend: &dyn CompatibilityBackend,
    context: &ActionContext,
    action: &DepotScriptAction,
    stopped: &impl Fn() -> bool,
) -> Result<()> {
    let value: ExecuteArguments =
        serde_json::from_value(action.arguments.clone()).context("parsing GOG execute action")?;
    let executable = resolve_executable_path(context, &value.executable)?;
    let mut working = value
        .working_dir
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(|path| resolve_path(context, path))
        .transpose()?;
    let arguments = if let Some(setup) = context.galaxy_setup.as_ref().filter(|setup| {
        !action.uninstall
            && value.executable.replace('\\', "/") == setup.executable.replace('\\', "/")
    }) {
        working.get_or_insert_with(|| context.app.clone());
        galaxy_setup_arguments(context, setup, &value.arguments)?
    } else {
        shell_words::split(&value.arguments).context("parsing GOG execute arguments")?
    };
    log_path(context, "executable", &executable)?;
    if let Some(path) = &working {
        log_path(context, "working directory", path)?;
    }
    run(backend, context, executable, arguments, working, stopped)
}

pub(crate) fn galaxy_setup_arguments(
    context: &ActionContext,
    setup: &GalaxySetup,
    explicit: &str,
) -> Result<Vec<String>> {
    if !explicit.is_empty() && !setup.arguments.is_empty() && explicit != setup.arguments {
        bail!("GOG setup script and repository specify conflicting wrapper arguments");
    }
    let mut arguments = windows_arguments(if explicit.is_empty() {
        &setup.arguments
    } else {
        explicit
    })?;
    for argument in &mut arguments {
        *argument = expand_text(context, argument);
        if argument.contains('{') || argument.contains('}') {
            bail!("Unsupported GOG wrapper argument variable");
        }
        if [
            "/dir=",
            "/lang=",
            "/language=",
            "/lang-code=",
            "/productid=",
            "/buildid=",
            "/versionname=",
        ]
        .iter()
        .any(|prefix| argument.to_ascii_lowercase().starts_with(prefix))
        {
            bail!("GOG wrapper arguments conflict with the selected installation context");
        }
    }
    let defaults = crate::compatibility::inno_setup_arguments(
        Some(&setup.language),
        &context.windows_app,
        false,
    );
    if let Some(language) = defaults
        .iter()
        .find_map(|argument| argument.strip_prefix("/LANG="))
    {
        arguments.push(format!("/Language={language}"));
    }
    arguments.extend(defaults);
    arguments.extend([
        format!("/lang-code={}", setup.language),
        format!("/ProductId={}", context.product_id),
        "/galaxyclient".into(),
        format!("/buildId={}", setup.build_id),
        format!("/versionName={}", setup.version),
        "/nodesktopshorctut".into(),
        "/nodesktopshortcut".into(),
    ]);
    Ok(arguments)
}

// Windows command-line backslashes are literal except immediately before a
// double quote. Never interpret these publisher-supplied arguments as a shell.
pub(crate) fn windows_arguments(value: &str) -> Result<Vec<String>> {
    if value.contains(['\0', '\r', '\n']) {
        bail!("Invalid control character in GOG wrapper arguments");
    }
    let mut chars = value.chars().peekable();
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    while let Some(character) = chars.next() {
        match character {
            '\\' => {
                let mut count = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    count += 1;
                }
                if chars.peek() == Some(&'"') {
                    current.extend(std::iter::repeat_n('\\', count / 2));
                    chars.next();
                    if count % 2 == 0 {
                        if quoted && chars.peek() == Some(&'"') {
                            chars.next();
                            current.push('"');
                        } else {
                            quoted = !quoted;
                        }
                    } else {
                        current.push('"');
                    }
                } else {
                    current.extend(std::iter::repeat_n('\\', count));
                }
                started = true;
            }
            '"' => {
                if quoted && chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    quoted = !quoted;
                }
                started = true;
            }
            ' ' | '\t' if !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(character);
                started = true;
            }
        }
    }
    if quoted {
        bail!("Unclosed quote in GOG wrapper arguments");
    }
    if started {
        arguments.push(current);
    }
    Ok(arguments)
}

fn resolve_executable_path(context: &ActionContext, value: &str) -> Result<PathBuf> {
    if value.starts_with('{') {
        return resolve_path(context, value);
    }
    let normalized = value.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.as_bytes().get(1) == Some(&b':')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        bail!("unsafe GOG executable path: {value:?}");
    }
    // Bare setup executables may be supplied by the support depot. Explicit
    // path variables above retain their declared root.
    for root in [&context.app, &context.support] {
        let candidate = root.join(&normalized);
        let mut cursor = PathBuf::new();
        let mut missing = false;
        for component in candidate.components() {
            cursor.push(component.as_os_str());
            let metadata = match fs::symlink_metadata(&cursor) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    missing = true;
                    break;
                }
                Err(error) => {
                    return Err(error).context("inspecting GOG setup executable path");
                }
            };
            if metadata.file_type().is_symlink()
                || if cursor == candidate {
                    !metadata.is_file()
                } else {
                    !metadata.is_dir()
                }
            {
                bail!("GOG setup executable path contains a link or unexpected file type");
            }
        }
        if !missing {
            return Ok(candidate);
        }
    }
    bail!(
        "Required GOG setup executable {value:?} is missing from both the game directory and downloaded support files. Resume the Depot operation to restore required files."
    )
}

fn run(
    backend: &dyn CompatibilityBackend,
    context: &ActionContext,
    executable: PathBuf,
    arguments: Vec<String>,
    working_directory: Option<PathBuf>,
    stopped: &impl Fn() -> bool,
) -> Result<()> {
    let request = CompatibilityRunRequest {
        prefix: context.prefix.clone(),
        profile: context.profile.clone(),
        executable,
        arguments,
        working_directory,
        log_path: context.log_path.clone(),
        background: true,
    };
    super::dependency_setup::run_tracked(
        context.operation_id.as_deref(),
        stopped,
        "GOG game setup",
        &context.log_path,
        || backend.run_executable(request).map_err(Into::into),
    )
}

#[cfg(test)]
fn setup_exit_error(context: &ActionContext, status: std::process::ExitStatus) -> anyhow::Error {
    let detail = super::runtime_logs::installation_tail(&context.log_path)
        .unwrap_or_else(|_| "The installation log could not be read.".into());
    anyhow::anyhow!(
        "Required GOG setup process failed ({status}). Log: {}\n{detail}",
        context.log_path.display()
    )
}

fn resolve_path(context: &ActionContext, value: &str) -> Result<PathBuf> {
    let roots = [
        ("{supportDir}", context.support.to_owned()),
        (
            "{userappdata}",
            context
                .prefix
                .join("drive_c/users/steamuser/AppData/Roaming"),
        ),
        (
            "{localappdata}",
            context.prefix.join("drive_c/users/steamuser/AppData/Local"),
        ),
        (
            "{userdocs}",
            context.prefix.join("drive_c/users/steamuser/Documents"),
        ),
        (
            "{usersavedgames}",
            context.prefix.join("drive_c/users/steamuser/Saved Games"),
        ),
        ("{app}", context.app.to_owned()),
    ];
    let (token, root) = roots
        .into_iter()
        .find(|(token, _)| {
            value.eq(*token)
                || value.starts_with(&format!("{token}/"))
                || value.starts_with(&format!(r"{token}\"))
        })
        .with_context(|| format!("unsupported GOG setup path variable: {value:?}"))?;
    let suffix = value[token.len()..]
        .trim_start_matches(['/', '\\'])
        .replace('\\', "/");
    let boundary = if root.starts_with(&context.prefix) {
        context.prefix.clone()
    } else {
        root.clone()
    };
    let mut resolved = root;
    for part in suffix.split('/').filter(|part| !part.is_empty()) {
        match part {
            "." => {}
            ".." if resolved != boundary => {
                resolved.pop();
            }
            ".." => bail!("GOG setup path escapes its managed root: {value:?}"),
            _ => resolved.push(part),
        }
    }
    if !resolved.starts_with(&boundary) {
        bail!("GOG setup path escapes its managed root: {value:?}");
    }
    Ok(resolved)
}

fn expand_text(context: &ActionContext, value: &str) -> String {
    value
        .replace("{productID}", &context.product_id.to_string())
        .replace("{app}", &context.windows_app)
}

fn log_path(context: &ActionContext, kind: &str, path: &Path) -> Result<()> {
    Ok(crate::compatibility::append_step_log(
        &context.log_path,
        &format!("GOG setup {kind}: {}", path.display()),
    )?)
}

fn reject_symlink_path(path: &Path) -> Result<()> {
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component.as_os_str());
        if fs::symlink_metadata(&cursor).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            bail!("GOG setup path crosses a symlink");
        }
    }
    Ok(())
}

fn copy_file(source: &Path, target: &Path, overwrite: bool) -> Result<()> {
    reject_symlink_path(source)?;
    reject_symlink_path(target)?;
    if target.exists() && !overwrite {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, target)?;
    Ok(())
}

fn copy_tree(source: &Path, target: &Path, overwrite: bool) -> Result<()> {
    reject_symlink_path(source)?;
    reject_symlink_path(target)?;
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            bail!("GOG support data contains a symlink");
        }
        let destination = target.join(entry.file_name());
        if metadata.is_dir() {
            copy_tree(&entry.path(), &destination, overwrite)?;
        } else {
            copy_file(&entry.path(), &destination, overwrite)?;
        }
    }
    Ok(())
}

fn conditions_supported(value: &serde_json::Value) -> bool {
    value.is_null()
        || value.as_str().is_some_and(str::is_empty)
        || value.as_array().is_some_and(|values| {
            values
                .iter()
                .all(|value| matches!(value.as_str(), Some("onlyOnce" | "onVersionChange")))
        })
}

fn condition_only_once(value: &serde_json::Value) -> bool {
    value.as_array().is_some_and(|values| {
        values
            .iter()
            .any(|value| value.as_str() == Some("onlyOnce"))
    })
}

fn set_ini_value(input: &str, section: &str, key: &str, value: &str) -> Result<String> {
    if [section, key, value]
        .iter()
        .any(|value| value.contains(['\0', '\r', '\n']))
    {
        bail!("unsafe GOG INI value");
    }
    let heading = format!("[{section}]");
    let mut lines = input.lines().map(str::to_owned).collect::<Vec<_>>();
    let start = lines
        .iter()
        .position(|line| line.trim().eq_ignore_ascii_case(&heading));
    if let Some(start) = start {
        let end = lines[start + 1..]
            .iter()
            .position(|line| line.trim_start().starts_with('['))
            .map_or(lines.len(), |offset| start + 1 + offset);
        if let Some(index) = (start + 1..end).find(|index| {
            lines[*index]
                .split_once('=')
                .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key))
        }) {
            lines[index] = format!("{key}={value}");
        } else {
            lines.insert(end, format!("{key}={value}"));
        }
    } else {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend([heading, format!("{key}={value}")]);
    }
    Ok(lines.join("\n") + "\n")
}

fn validate_registry_component(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with(['/', '\\'])
        || value.contains(['\0', '\r', '\n'])
        || value
            .replace('\\', "/")
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("unsafe GOG registry subkey");
    }
    Ok(())
}

fn registry_type(value: &str) -> Result<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "string" | "sz" => Ok("REG_SZ"),
        "expandstring" | "expand_sz" => Ok("REG_EXPAND_SZ"),
        "dword" => Ok("REG_DWORD"),
        "qword" => Ok("REG_QWORD"),
        "multistring" | "multi_sz" => Ok("REG_MULTI_SZ"),
        "binary" => Ok("REG_BINARY"),
        _ => bail!("unsupported GOG registry value type"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct CaptureBackend(std::sync::Mutex<Vec<CompatibilityRunRequest>>);
    impl CompatibilityBackend for CaptureBackend {
        fn status(
            &self,
        ) -> crate::compatibility::Result<crate::compatibility::CompatibilityBackendStatus>
        {
            panic!("not an invocation probe")
        }
        fn initialize_prefix(
            &self,
            _: crate::compatibility::InitializePrefixRequest,
        ) -> crate::compatibility::Result<crate::compatibility::CompatibilityPrefix> {
            panic!("no prefix creation")
        }
        fn run_executable(
            &self,
            request: CompatibilityRunRequest,
        ) -> crate::compatibility::Result<crate::compatibility::CompatibilityProcess> {
            self.0.lock().unwrap().push(request);
            Err(crate::compatibility::CompatibilityFailure::GameLaunchRejected)
        }
        fn stop(
            &self,
            _: &mut crate::compatibility::CompatibilityProcess,
        ) -> crate::compatibility::Result<()> {
            panic!("no process exists")
        }
    }

    #[test]
    fn captured_gungeon_wrapper_uses_selected_context_without_affecting_other_execute_actions() {
        let directory = tempfile::tempdir().unwrap();
        let mut context = context(directory.path(), &UmuProfile::fallback());
        context.product_id = 1456912569;
        context.windows_app = r"L:\Game With Spaces".into();
        fs::create_dir_all(&context.support).unwrap();
        fs::create_dir_all(&context.app).unwrap();
        let name = "galaxy_enter_the_gungeon_2.11.0.13.exe";
        fs::write(context.support.join(name), b"inert").unwrap();
        context.galaxy_setup = Some(GalaxySetup {
            executable: name.into(),
            arguments: String::new(),
            language: "en-US".into(),
            build_id: "52278062065653360".into(),
            version: "2.1.9".into(),
        });
        let actions = super::super::depot_metadata::script_actions(
            br#"{"productId":"1456912569","actions":[{"name":"Install the game","languages":["*"],"install":{"action":"Execute","arguments":{"executable":"galaxy_enter_the_gungeon_2.11.0.13.exe","runAsAdmin":true}}}]}"#,
            context.product_id, "en-US",
        ).unwrap();
        let backend = CaptureBackend::default();
        let error = execute_actions(&backend, &context, &actions, false).unwrap_err();
        assert!(error.to_string().contains("Install the game"));
        let request = backend.0.lock().unwrap().pop().unwrap();
        assert_eq!(request.executable, context.support.join(name));
        assert_eq!(request.working_directory, Some(context.app.clone()));
        for flag in [
            "/VERYSILENT",
            "/SP-",
            r"/DIR=L:\Game With Spaces",
            "/LANG=english",
            "/Language=english",
            "/lang-code=en-US",
            "/ProductId=1456912569",
            "/galaxyclient",
            "/buildId=52278062065653360",
            "/versionName=2.1.9",
        ] {
            assert!(
                request.arguments.iter().any(|argument| argument == flag),
                "{flag}"
            );
        }
        assert!(
            fs::read_to_string(&context.log_path)
                .unwrap()
                .contains("GOG setup working directory:")
        );
        for field in ["arguments", "argument", "parameters"] {
            let mut ordinary = actions[0].clone();
            ordinary.arguments = json!({"executable":"other.exe","workingDir":"{app}"});
            ordinary.arguments[field] = json!("--custom \"two words\"");
            fs::write(context.app.join("other.exe"), b"inert").unwrap();
            execute_actions(&backend, &context, &[ordinary], false).unwrap_err();
            let request = backend.0.lock().unwrap().pop().unwrap();
            assert_eq!(request.arguments, ["--custom", "two words"]);
            assert_eq!(request.working_directory, Some(context.app.clone()));
        }
        let mut uninstall = actions[0].clone();
        uninstall.uninstall = true;
        execute_actions(&backend, &context, &[uninstall], true).unwrap_err();
        assert!(
            backend
                .0
                .lock()
                .unwrap()
                .pop()
                .unwrap()
                .arguments
                .is_empty()
        );
    }

    #[test]
    fn wrapper_arguments_preserve_windows_quotes_paths_and_validate_context() {
        assert_eq!(
            windows_arguments(r#""a b c" d e"#).unwrap(),
            ["a b c", "d", "e"]
        );
        assert_eq!(
            windows_arguments(r#""ab\"c" "\\" d"#).unwrap(),
            ["ab\"c", r"\", "d"]
        );
        assert_eq!(
            windows_arguments(r#""a""b" "" C:\path\file.exe"#).unwrap(),
            ["a\"b", "", r"C:\path\file.exe"]
        );
        assert_eq!(
            windows_arguments(r#"a\\\"b c d"#).unwrap(),
            [r#"a\"b"#, "c", "d"]
        );
        assert!(windows_arguments("\"unterminated").is_err());
        assert!(windows_arguments("a\0b").is_err());
        let mut context = context(Path::new("/library"), &UmuProfile::fallback());
        context.windows_app = r"L:\Game With Spaces".into();
        let mut setup = GalaxySetup {
            executable: "setup.exe".into(),
            arguments: r#"/custom="{app}\file.ini" /id={productID}"#.into(),
            language: "en-US".into(),
            build_id: "build".into(),
            version: "1".into(),
        };
        let args = galaxy_setup_arguments(&context, &setup, "").unwrap();
        assert_eq!(
            &args[..2],
            &[r"/custom=L:\Game With Spaces\file.ini", "/id=1812959072"]
        );
        assert!(galaxy_setup_arguments(&context, &setup, "/different").is_err());
        for invalid in ["/DIR=Z:\\escape", "/custom={unknown}"] {
            setup.arguments = invalid.into();
            assert!(galaxy_setup_arguments(&context, &setup, "").is_err());
        }
    }

    #[test]
    fn setup_failure_retains_exit_status_and_sanitized_log_details() {
        use std::os::unix::process::ExitStatusExt;
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path(), &UmuProfile::fallback());
        fs::write(&context.log_path, b"setup: required data missing\nAuthorization: secret-canary\nhttps://example.test/file?token=secret-canary\n").unwrap();
        let error =
            setup_exit_error(&context, std::process::ExitStatus::from_raw(7 << 8)).to_string();
        assert!(error.contains("exit status: 7"));
        assert!(error.contains("required data missing"));
        assert!(error.contains(&context.log_path.to_string_lossy().to_string()));
        assert!(!error.contains("secret-canary"));
        assert!(!error.contains("https://"));
    }

    fn context(root: &Path, profile: &UmuProfile) -> ActionContext {
        ActionContext {
            operation_id: None,
            product_id: 1812959072,
            app: root.join("game"),
            support: root.join("support"),
            prefix: root.join("prefix"),
            windows_app: r"L:\grim_dawn".into(),
            profile: profile.clone(),
            log_path: root.join("log"),
            galaxy_setup: None,
        }
    }
    fn action(uninstall: bool) -> DepotScriptAction {
        DepotScriptAction {
            name: "registryDLC".into(),
            kind: DepotScriptActionKind::SetRegistry,
            arguments: json!({"root":"HKEY_LOCAL_MACHINE","subkey":"Software/Crate Entertainment/Grim Dawn/GOG/DLC","valueData":"1","valueName":"{productID}","valueType":"dword"}),
            uninstall,
        }
    }
    #[test]
    fn builds_grim_dawn_dlc_add_and_inverse_delete_without_a_shell() {
        let root = Path::new("/library");
        let profile = UmuProfile::fallback();
        let context = context(root, &profile);
        let add = registry_command(&context, &action(false)).unwrap();
        assert!(add.script.contains(r#""1812959072"=dword:00000001"#));
        assert!(
            registry_command(&context, &action(true))
                .unwrap()
                .script
                .contains(r#""1812959072"=-"#)
        );
    }
    #[test]
    fn updates_ini_without_destroying_other_sections() {
        let input = "[Video]\nMode=1\n\n[Audio]\nVolume=5\n";
        assert_eq!(
            set_ini_value(input, "Video", "Mode", "2").unwrap(),
            "[Video]\nMode=2\n\n[Audio]\nVolume=5\n"
        );
        assert!(ini_has_key(input, "video", "mode"));
        assert!(!ini_has_key(input, "Video", "Missing"));
    }
    #[test]
    fn rejects_unsafe_or_unknown_registry_inputs() {
        let root = Path::new("/library");
        let profile = UmuProfile::fallback();
        let context = context(root, &profile);
        let mut invalid = action(false);
        invalid.arguments["subkey"] = json!("../escape");
        assert!(registry_command(&context, &invalid).is_err());
        invalid = action(false);
        invalid.arguments["root"] = json!("HKEY_CLASSES_ROOT");
        assert!(registry_command(&context, &invalid).is_err());
    }

    #[test]
    fn resolves_doom_saved_games_path_inside_the_prefix() {
        let root = Path::new("/library");
        let profile = UmuProfile::fallback();
        let context = context(root, &profile);
        assert_eq!(
            resolve_path(&context, "{usersavedgames}/id Software/DOOM",).unwrap(),
            root.join("prefix/drive_c/users/steamuser/Saved Games/id Software/DOOM")
        );
        assert_eq!(
            resolve_path(
                &context,
                "{userdocs}/../Saved Games/Nightdive Studios/DOOM 64",
            )
            .unwrap(),
            root.join("prefix/drive_c/users/steamuser/Saved Games/Nightdive Studios/DOOM 64")
        );
        assert!(
            resolve_path(
                &context,
                "{userdocs}/../../../../../../../../outside-prefix",
            )
            .is_err()
        );
        assert!(resolve_path(&context, "{app}/../outside-game").is_err());
    }

    #[test]
    fn accepts_observed_legacy_registry_and_execute_shapes() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let profile = UmuProfile::fallback();
        let context = context(root, &profile);
        fs::create_dir_all(&context.app).unwrap();
        fs::write(context.app.join("legacy_setup.exe"), b"inert").unwrap();
        let mut registry = action(false);
        registry.arguments = json!({
            "root": "HKLM",
            "subkey": "Software\\LucasArts\\KOTOR2",
            "valueName": "Path",
            "ValueData": "{app}",
            "valueType": "string"
        });
        assert!(
            registry_command(&context, &registry)
                .unwrap()
                .script
                .contains("L:\\\\grim_dawn")
        );

        registry.arguments = json!({
            "root": "HKEY_CURRENT_USER",
            "subkey": "Software\\Example",
            "deleteSubkeys": true
        });
        let script = registry_command(&context, &registry).unwrap().script;
        assert!(script.contains("[HKCU\\Software\\Example]"));
        assert!(!script.contains("@="));

        assert_eq!(
            resolve_executable_path(&context, "legacy_setup.exe").unwrap(),
            root.join("game/legacy_setup.exe")
        );
        let execute: ExecuteArguments = serde_json::from_value(json!({
            "executable": "legacy_setup.exe",
            "argument": "--quiet",
            "workingDir": ""
        }))
        .unwrap();
        assert_eq!(execute.arguments, "--quiet");
    }

    #[test]
    fn resolves_materialized_support_executable_after_cleanup_and_resume() {
        use crate::gog::depot_manifest::{DepotChunk, DepotEntry, DepotFile, DepotManifest};
        use std::os::unix::fs::FileExt;

        let directory = tempfile::tempdir().unwrap();
        let operation = directory.path().join(".ludomere/staging/game.json");
        let mut context = context(directory.path(), &UmuProfile::fallback());
        context.support = support_staging(&operation).unwrap();
        let name = "galaxy_enter_the_gungeon_2.11.0.13.exe";
        let bytes = b"inert support executable fixture";
        let digest = format!("{:x}", md5::compute(bytes));
        let manifest = DepotManifest {
            generation: 2,
            small_files_containers: Vec::new(),
            entries: vec![DepotEntry::File(DepotFile {
                path: name.into(),
                size: bytes.len() as u64,
                executable: true,
                support: true,
                md5: Some(digest.clone()),
                sha256: None,
                small_file: None,
                chunks: vec![DepotChunk {
                    compressed_md5: digest.clone(),
                    compressed_size: bytes.len() as u64,
                    md5: digest,
                    size: bytes.len() as u64,
                }],
            })],
        };
        let mut fetched = 0;
        for attempt in 0..3 {
            assert!(resolve_executable_path(&context, name).is_err());
            assert_eq!(
                pending_support_chunks(&manifest, &operation).unwrap().len(),
                1
            );
            materialize_support(
                &manifest,
                &operation,
                |jobs, file, progress| {
                    for (index, job) in jobs.iter().enumerate() {
                        file.write_all_at(bytes, job.offset)?;
                        progress(index)?;
                        fetched += 1;
                    }
                    Ok(())
                },
                || false,
            )
            .unwrap();
            assert_eq!(
                resolve_executable_path(&context, name).unwrap(),
                context.support.join(name)
            );
            assert_eq!(fs::read(context.support.join(name)).unwrap(), bytes);
            if attempt == 1 {
                // Manual deletion leaves the completion journal behind.
                fs::remove_file(context.support.join(name)).unwrap();
                assert!(support_journal(&operation).unwrap().is_file());
            } else {
                // Failed finalization removes support staging; Resume must restore it.
                remove_support_staging(&operation).unwrap();
            }
        }
        assert_eq!(fetched, 3);
    }

    #[test]
    fn executable_fallback_preserves_roots_and_rejects_unsafe_candidates() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path(), &UmuProfile::fallback());
        fs::create_dir_all(&context.app).unwrap();
        fs::create_dir_all(&context.support).unwrap();
        fs::write(context.support.join("setup.exe"), b"support").unwrap();
        assert_eq!(
            resolve_executable_path(&context, "setup.exe").unwrap(),
            context.support.join("setup.exe")
        );
        assert_eq!(
            resolve_executable_path(&context, "{app}/setup.exe").unwrap(),
            context.app.join("setup.exe")
        );
        if unsafe { libc::geteuid() } != 0 {
            fs::set_permissions(&context.app, fs::Permissions::from_mode(0o000)).unwrap();
            let inaccessible = resolve_executable_path(&context, "setup.exe");
            fs::set_permissions(&context.app, fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(
                inaccessible
                    .unwrap_err()
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
        }
        fs::write(context.app.join("setup.exe"), b"app").unwrap();
        assert_eq!(
            resolve_executable_path(&context, "setup.exe").unwrap(),
            context.app.join("setup.exe")
        );
        fs::remove_file(context.app.join("setup.exe")).unwrap();
        symlink(
            context.support.join("setup.exe"),
            context.app.join("setup.exe"),
        )
        .unwrap();
        assert!(resolve_executable_path(&context, "setup.exe").is_err());
        fs::remove_file(context.app.join("setup.exe")).unwrap();
        fs::create_dir(context.app.join("setup.exe")).unwrap();
        assert!(resolve_executable_path(&context, "setup.exe").is_err());
        for unsafe_path in ["../setup.exe", "/setup.exe", r"C:\setup.exe", "SETUP.EXE"] {
            assert!(resolve_executable_path(&context, unsafe_path).is_err());
        }
        symlink(&context.support, context.app.join("linked")).unwrap();
        assert!(resolve_executable_path(&context, "linked/setup.exe").is_err());
        fs::remove_file(context.support.join("setup.exe")).unwrap();
        symlink(
            context.app.join("absent.exe"),
            context.support.join("missing.exe"),
        )
        .unwrap();
        assert!(resolve_executable_path(&context, "missing.exe").is_err());
    }

    #[test]
    fn logs_resolved_paths_and_action_failures() {
        let root =
            std::env::temp_dir().join(format!("ludomere-depot-action-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let profile = UmuProfile::fallback();
        let context = context(&root, &profile);
        let actions = [
            DepotScriptAction {
                name: "valid save".into(),
                kind: DepotScriptActionKind::SavePath,
                arguments: json!({"savePath":"{app}/saves"}),
                uninstall: false,
            },
            DepotScriptAction {
                name: "unsafe save".into(),
                kind: DepotScriptActionKind::SavePath,
                arguments: json!({"savePath":"{app}/../escape"}),
                uninstall: false,
            },
        ];
        assert!(
            execute_actions(
                &crate::compatibility::UmuBackend::default(),
                &context,
                &actions,
                false
            )
            .is_err()
        );
        let log = fs::read_to_string(&context.log_path).unwrap();
        assert!(log.contains(&format!(
            "GOG setup save path: {}",
            root.join("game/saves").display()
        )));
        assert!(log.contains(
            "GOG script action failed: unsafe save: GOG setup path escapes its managed root"
        ));
        assert!(log.contains(r#""{app}/../escape""#));
        fs::remove_dir_all(root).unwrap();
    }
}
