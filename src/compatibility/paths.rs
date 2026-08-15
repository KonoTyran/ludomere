use super::{CompatibilityFailure, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};
const OWNERSHIP_FILE: &str = ".ludomere-managed.json";

pub fn prefix_relative(slug: &str) -> PathBuf {
    Path::new(crate::identity::MARKER_DIRECTORY)
        .join("compatibility")
        .join(slug)
}
pub fn prefix_path(library: &Path, slug: &str) -> PathBuf {
    library.join(prefix_relative(slug))
}
pub fn windows_destination(slug: &str) -> String {
    format!("L:\\{slug}")
}

/// Build an Inno Setup invocation. Automated installs are completely hidden;
/// interactive installs retain only the selected destination override so the
/// user can make every other installer-specific choice.
pub fn inno_setup_arguments(
    language: Option<&str>,
    destination: &str,
    interactive: bool,
) -> Vec<String> {
    let mut arguments = if interactive {
        Vec::new()
    } else {
        vec![
            "/VERYSILENT".to_owned(),
            "/SUPPRESSMSGBOXES".to_owned(),
            "/NORESTART".to_owned(),
            "/SP-".to_owned(),
        ]
    };
    if !interactive && let Some(language) = language.and_then(inno_language_name) {
        arguments.push(format!("/LANG={language}"));
    }
    arguments.push(format!("/DIR={destination}"));
    arguments
}

/// Run a GOG/Inno patch without prompting while allowing it to discover the
/// existing installation from the game's Wine prefix.
pub fn inno_patch_arguments(language: Option<&str>) -> Vec<String> {
    let mut arguments = vec![
        "/SILENT".to_owned(),
        "/SUPPRESSMSGBOXES".to_owned(),
        "/NORESTART".to_owned(),
        "/SP-".to_owned(),
    ];
    if let Some(language) = language.and_then(inno_language_name) {
        arguments.push(format!("/LANG={language}"));
    }
    arguments
}

fn inno_language_name(language: &str) -> Option<&'static str> {
    match language.trim().to_ascii_lowercase().as_str() {
        "en" | "en-us" | "en-gb" | "english" => Some("english"),
        "de" | "german" | "deutsch" => Some("german"),
        "fr" | "french" | "français" => Some("french"),
        "es" | "spanish" | "español" => Some("spanish"),
        "it" | "italian" | "italiano" => Some("italian"),
        "pl" | "polish" | "polski" => Some("polish"),
        "pt-br" | "brazilian portuguese" => Some("brazilianportuguese"),
        "ru" | "russian" | "русский" => Some("russian"),
        _ => None,
    }
}

pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug == "." || slug == ".." || slug.contains('/') || slug.contains('\\') {
        return Err(CompatibilityFailure::PrefixOwnershipAmbiguous(
            PathBuf::from(slug),
        ));
    }
    Ok(())
}
pub fn validate_library(library: &Path) -> Result<PathBuf> {
    fs::create_dir_all(library)
        .map_err(|_| CompatibilityFailure::LibraryUnavailable(library.into()))?;
    let library = fs::canonicalize(library)
        .map_err(|_| CompatibilityFailure::LibraryUnavailable(library.into()))?;
    let probe = library.join(".ludomere-umu-write-test");
    fs::write(&probe, b"")
        .map_err(|_| CompatibilityFailure::LibraryNotWritable(library.clone()))?;
    let _ = fs::remove_file(probe);
    Ok(library)
}
pub fn configure_library_drive(prefix: &Path, library: &Path) -> Result<()> {
    let prefix =
        fs::canonicalize(prefix).map_err(|_| CompatibilityFailure::PrefixMissing(prefix.into()))?;
    let library = fs::canonicalize(library)
        .map_err(|_| CompatibilityFailure::LibraryUnavailable(library.into()))?;
    let dosdevices = prefix.join("dosdevices");
    let real = fs::canonicalize(&dosdevices)
        .map_err(|_| CompatibilityFailure::PrefixCorrupt(prefix.clone()))?;
    if !real.starts_with(&prefix) {
        return Err(CompatibilityFailure::PrefixOwnershipAmbiguous(prefix));
    }
    let mapping = dosdevices.join("l:");
    if let Ok(existing) = fs::canonicalize(&mapping) {
        return if existing == library {
            Ok(())
        } else {
            Err(CompatibilityFailure::DriveMappingConflict)
        };
    }
    if fs::symlink_metadata(&mapping).is_ok() {
        return Err(CompatibilityFailure::DriveMappingConflict);
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&library, &mapping)
        .map_err(|_| CompatibilityFailure::DriveMappingFailed)?;
    #[cfg(not(unix))]
    return Err(CompatibilityFailure::DriveMappingFailed);
    if fs::canonicalize(mapping).ok().as_ref() != Some(&library) {
        return Err(CompatibilityFailure::DriveMappingMismatch);
    }
    Ok(())
}

/// Verify the durable postconditions of UMU/Proton prefix creation.
///
/// UMU's documented `umu-run ""` command creates the prefix and then exits
/// unsuccessfully because there is no executable to launch. Prefix creation
/// must therefore be judged by its filesystem result, not that exit status.
pub fn validate_prefix_structure(prefix: &Path) -> Result<()> {
    let directories = [prefix.join("drive_c"), prefix.join("dosdevices")];
    let registries = [
        prefix.join("system.reg"),
        prefix.join("user.reg"),
        prefix.join("userdef.reg"),
    ];
    if directories.iter().all(|path| path.is_dir()) && registries.iter().all(|path| path.is_file())
    {
        Ok(())
    } else {
        Err(CompatibilityFailure::PrefixInitializationFailed)
    }
}

pub fn write_ownership(prefix: &Path, slug: &str) -> Result<()> {
    validate_slug(slug)?;
    let value = serde_json::json!({"schema_version":1,"slug":slug,"managed_by_ludomere":true});
    fs::write(
        prefix.join(OWNERSHIP_FILE),
        serde_json::to_vec_pretty(&value)
            .map_err(|_| CompatibilityFailure::PrefixOwnershipAmbiguous(prefix.into()))?,
    )
    .map_err(Into::into)
}
pub fn validate_ownership(prefix: &Path, slug: &str) -> Result<()> {
    let bytes = fs::read(prefix.join(OWNERSHIP_FILE))
        .map_err(|_| CompatibilityFailure::PrefixOwnershipAmbiguous(prefix.into()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| CompatibilityFailure::PrefixOwnershipAmbiguous(prefix.into()))?;
    if value.get("managed_by_ludomere").and_then(|v| v.as_bool()) != Some(true)
        || value.get("slug").and_then(|v| v.as_str()) != Some(slug)
    {
        return Err(CompatibilityFailure::PrefixOwnershipAmbiguous(
            prefix.into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefix_is_library_relative() {
        assert_eq!(
            prefix_relative("grim-dawn"),
            PathBuf::from(".ludomere/compatibility/grim-dawn")
        );
    }

    #[test]
    fn validates_a_complete_proton_prefix() {
        let temp =
            std::env::temp_dir().join(format!("ludomere-prefix-validation-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir(&temp).unwrap();
        assert!(validate_prefix_structure(&temp).is_err());
        fs::create_dir(temp.join("drive_c")).unwrap();
        fs::create_dir(temp.join("dosdevices")).unwrap();
        for name in ["system.reg", "user.reg", "userdef.reg"] {
            fs::write(temp.join(name), "").unwrap();
        }
        assert!(validate_prefix_structure(&temp).is_ok());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn inno_setup_is_fully_unattended_by_default() {
        let arguments = inno_setup_arguments(Some("English"), "L:\\grim_dawn", false);
        assert!(arguments.contains(&"/VERYSILENT".to_owned()));
        assert!(arguments.contains(&"/SUPPRESSMSGBOXES".to_owned()));
        assert!(arguments.contains(&"/LANG=english".to_owned()));
        assert!(arguments.contains(&"/DIR=L:\\grim_dawn".to_owned()));
        assert!(!arguments.contains(&"/SILENT".to_owned()));
    }

    #[test]
    fn interactive_inno_setup_only_overrides_the_destination() {
        let arguments = inno_setup_arguments(Some("English"), "L:\\grim_dawn", true);
        assert_eq!(arguments, ["/DIR=L:\\grim_dawn"]);
    }

    #[test]
    fn inno_patch_is_unattended_without_overriding_the_installed_destination() {
        let arguments = inno_patch_arguments(Some("English"));
        assert!(arguments.contains(&"/SILENT".to_owned()));
        assert!(arguments.contains(&"/LANG=english".to_owned()));
        assert!(
            !arguments
                .iter()
                .any(|argument| argument.starts_with("/DIR="))
        );
    }
}
