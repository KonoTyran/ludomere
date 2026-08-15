use serde::Deserialize;
use std::{collections::HashMap, sync::OnceLock};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchFixOperation {
    StartGogOnlineServices,
    AddBundledLibraryDirectories { filename_prefix: String },
    ClearBundledLibraryExecutableStack { filename_prefix: String },
    CreateBundledLibraryAlias { filename: String, alias: String },
    PreloadBundledLibrary { filename: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LaunchFixDefinition {
    pub id: String,
    pub title: String,
    pub description: String,
    pub operation: LaunchFixOperation,
}

#[derive(Debug, Deserialize)]
struct FixDatabase {
    fixes: Vec<LaunchFixDefinition>,
    #[serde(default)]
    defaults: Vec<String>,
    games: HashMap<String, Vec<String>>,
}

fn database() -> &'static FixDatabase {
    static DATABASE: OnceLock<FixDatabase> = OnceLock::new();
    DATABASE.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/resources/compatibility-fixes.json"
        )))
        .expect("bundled compatibility fix database must be valid")
    })
}

pub fn available_fixes() -> &'static [LaunchFixDefinition] {
    &database().fixes
}

pub fn recommended_fix_ids(product_id: i64) -> Vec<String> {
    let database = database();
    let mut recommended = database.defaults.clone();
    if let Some(game_fixes) = database.games.get(&product_id.to_string()) {
        recommended.extend(game_fixes.iter().cloned());
    }
    recommended.sort();
    recommended.dedup();
    recommended
}

pub fn effective_fixes(
    product_id: i64,
    overrides: &HashMap<String, bool>,
) -> Vec<LaunchFixDefinition> {
    let recommended = recommended_fix_ids(product_id);
    available_fixes()
        .iter()
        .filter(|fix| {
            overrides
                .get(&fix.id)
                .copied()
                .unwrap_or_else(|| recommended.contains(&fix.id))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_games_receive_only_global_defaults() {
        let fixes = effective_fixes(42, &HashMap::new());
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].id, "gog_online_services");
    }

    #[test]
    fn user_choices_override_recommendations() {
        let mut overrides = HashMap::new();
        overrides.insert("gog_online_services".into(), false);
        overrides.insert("native_galaxy_library_path".into(), true);
        let fixes = effective_fixes(1_224_667_888, &overrides);
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].id, "native_galaxy_library_path");
    }
}
