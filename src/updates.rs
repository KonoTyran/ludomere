use crate::{config::Config, domain::GamePreferences};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePolicy {
    pub auto_update_galaxy: bool,
    pub auto_download_offline_installer: bool,
    pub prune_superseded_installers: bool,
    pub galaxy_language: Option<String>,
}

impl UpdatePolicy {
    pub fn resolve(config: &Config, preferences: Option<&GamePreferences>) -> Self {
        Self {
            auto_update_galaxy: preferences
                .and_then(|value| value.auto_update_galaxy)
                .unwrap_or(config.auto_update_galaxy_installations),
            auto_download_offline_installer: preferences
                .and_then(|value| value.auto_download_offline_installer)
                .unwrap_or(config.auto_download_offline_installers),
            prune_superseded_installers: preferences
                .and_then(|value| value.prune_superseded_installers)
                .unwrap_or(config.prune_superseded_offline_installers),
            galaxy_language: preferences
                .and_then(|value| value.galaxy_language.clone())
                .or_else(|| config.installer_language.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_game_choices_override_global_defaults() {
        let config = Config {
            installer_language: Some("English".into()),
            ..Config::default()
        };
        let preferences = GamePreferences {
            product_id: 1,
            executable_path: None,
            launch_arguments: Vec::new(),
            compatibility: None,
            auto_update_galaxy: Some(false),
            auto_download_offline_installer: Some(true),
            prune_superseded_installers: None,
            galaxy_language: Some("de-DE".into()),
            created_at: 0,
            updated_at: 0,
        };

        let policy = UpdatePolicy::resolve(&config, Some(&preferences));
        assert!(!policy.auto_update_galaxy);
        assert!(policy.auto_download_offline_installer);
        assert!(!policy.prune_superseded_installers);
        assert_eq!(policy.galaxy_language.as_deref(), Some("de-DE"));
    }
}
