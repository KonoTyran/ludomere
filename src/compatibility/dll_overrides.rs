use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, process::Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DllLoadOrder {
    Native,
    Builtin,
    NativeThenBuiltin,
    BuiltinThenNative,
    Disabled,
}

impl DllLoadOrder {
    pub const ALL: [Self; 5] = [
        Self::Native,
        Self::Builtin,
        Self::NativeThenBuiltin,
        Self::BuiltinThenNative,
        Self::Disabled,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "Native",
            Self::Builtin => "Builtin",
            Self::NativeThenBuiltin => "Native then Builtin",
            Self::BuiltinThenNative => "Builtin then Native",
            Self::Disabled => "Disabled",
        }
    }

    fn value(self) -> &'static str {
        match self {
            Self::Native => "n",
            Self::Builtin => "b",
            Self::NativeThenBuiltin => "n,b",
            Self::BuiltinThenNative => "b,n",
            Self::Disabled => "",
        }
    }
}

/// One basename per row, never a path or a Wine expression.
pub fn normalize_dll_overrides(
    rows: impl IntoIterator<Item = (String, DllLoadOrder)>,
) -> Result<BTreeMap<String, DllLoadOrder>> {
    let mut overrides = BTreeMap::new();
    for (name, order) in rows {
        ensure!(
            !name.chars().any(char::is_control),
            "DLL names cannot contain control characters."
        );
        let name = name.trim().to_ascii_lowercase();
        let name = name.strip_suffix(".dll").unwrap_or(&name);
        ensure!(
            !name.ends_with(".dll"),
            "Use a DLL name with at most one .dll suffix."
        );
        ensure!(
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
                && name.bytes().any(|byte| byte.is_ascii_alphanumeric()),
            "Enter one DLL name per row, such as dinput8 or dinput8.dll; paths and override expressions are not supported."
        );
        ensure!(
            overrides.insert(name.to_owned(), order).is_none(),
            "Each DLL may appear only once (names ignore case and the .dll suffix)."
        );
        ensure!(
            overrides.len() <= 128,
            "Use at most 128 DLL override rows per game."
        );
    }
    Ok(overrides)
}

/// Apply only to the foreground Windows game command, after managed launch fixes.
pub fn apply_game_dll_overrides(
    command: &mut Command,
    overrides: &BTreeMap<String, DllLoadOrder>,
    defaults: &[(&str, DllLoadOrder)],
) -> Result<()> {
    if overrides.is_empty() && defaults.is_empty() {
        return Ok(());
    }
    let overrides =
        normalize_dll_overrides(overrides.iter().map(|(name, order)| (name.clone(), *order)))?;
    let environment = |name: &str| {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.map(ToOwned::to_owned))
            .unwrap_or_else(|| std::env::var_os(name))
    };
    // GE accepts the Steam token verbatim and ADD_CONFIG's name even with '=0'.
    // Its current launcher has no REMOVE_CONFIG counterpart. Abstain conservatively
    // for unparseable input rather than override an explicit runtime selection.
    let native_xinput = ["STEAM_COMPAT_CONFIG", "PROTON_ADD_CONFIG"]
        .into_iter()
        .any(|key| {
            environment(key).is_some_and(|value| {
                value.to_str().is_none_or(|text| {
                    text.split(',').any(|token| {
                        if key == "PROTON_ADD_CONFIG" {
                            token.split('=').next() == Some("usenativexinput13")
                        } else {
                            token == "usenativexinput13"
                        }
                    })
                })
            })
        });
    let inherited = environment("WINEDLLOVERRIDES");
    let inherited = inherited.as_deref().map(|value| value.to_str().context("Existing DLL overrides are not valid text; correct WINEDLLOVERRIDES before applying per-game choices.")).transpose()?.unwrap_or("");
    // Wine splits names on comma/space/tab and keeps the last definition of each key.
    // Keep the original expression intact, including grouped names and wildcard keys.
    let names = inherited
        .split(';')
        .filter_map(|clause| clause.split_once('='))
        .flat_map(|(names, _)| {
            names
                .split([',', ' ', '\t'])
                .filter(|name| !name.is_empty())
        })
        .collect::<Vec<_>>();
    let basename = |name: &str| {
        let name = name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(name)
            .trim_start_matches('*')
            .to_ascii_lowercase();
        name.strip_suffix(".dll").unwrap_or(&name).to_owned()
    };
    let mut clauses = Vec::new();
    for (name, order) in defaults {
        if !names.iter().any(|existing| basename(existing) == *name)
            && !overrides.contains_key(*name)
            && !(*name == "xinput1_3" && native_xinput)
        {
            // Wine consults env then registry at EACH key: qualified, wildcard, bare.
            clauses.push(format!("{name},*{name}={}", order.value()));
        }
    }
    if !inherited.is_empty() {
        clauses.push(inherited.to_owned());
    }
    for (name, order) in overrides {
        // A matching inherited wildcard/path key can outrank a plain basename in Wine.
        for existing in &names {
            if basename(existing) == name {
                clauses.push(format!("{existing}={}", order.value()));
            }
        }
        clauses.push(format!("{name},*{name}={}", order.value()));
    }
    command.env("WINEDLLOVERRIDES", clauses.join(";"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Inert reference fixture for GE11-7 Wine46b29104 loadorder.c:327-357,
    // 414-425,483-494: specificity first, environment then registry at each key.
    // This verifies command construction, not actual Wine/hardware behavior.
    fn wine_lookup(environment: &str, registry: &BTreeMap<&str, &str>, system: bool) -> String {
        let mut overrides = BTreeMap::new();
        for clause in environment.split(';') {
            if let Some((names, order)) = clause.split_once('=') {
                for name in names
                    .split([',', ' ', '\t'])
                    .filter(|name| !name.is_empty())
                {
                    let name = name.to_ascii_lowercase();
                    overrides.insert(name.strip_suffix(".dll").unwrap_or(&name).to_owned(), order);
                }
            }
        }
        let keys = if system {
            vec!["xinput1_3", "*xinput1_3"]
        } else {
            vec!["c:\\game\\xinput1_3", "*xinput1_3", "xinput1_3"]
        };
        for key in keys {
            if let Some(value) = overrides.get(key).or_else(|| registry.get(key)) {
                return value.to_string();
            }
        }
        "default".into()
    }

    #[test]
    fn dll_choices_cover_qualified_and_system_loads_despite_registry_wildcards() {
        let registry = BTreeMap::from([("*xinput1_3", "n")]);
        assert_eq!(
            wine_lookup("xinput1_3=b", &registry, false),
            "n",
            "the old bare override loses for qualified loads"
        );
        assert_eq!(wine_lookup("xinput1_3=b", &registry, true), "b");
        for order in DllLoadOrder::ALL {
            let mut command = Command::new("inert-not-executed");
            command.env("WINEDLLOVERRIDES", "unrelated=n");
            apply_game_dll_overrides(
                &mut command,
                &BTreeMap::from([("XINPUT1_3.DLL".into(), order)]),
                &[],
            )
            .unwrap();
            let value = command
                .get_envs()
                .find(|(key, _)| *key == "WINEDLLOVERRIDES")
                .unwrap()
                .1
                .unwrap()
                .to_str()
                .unwrap();
            assert_eq!(wine_lookup(value, &registry, false), order.value());
            assert_eq!(wine_lookup(value, &registry, true), order.value());
            assert!(value.starts_with("unrelated=n;"));
        }
        let mut automatic = Command::new("inert-not-executed");
        automatic
            .env_remove("STEAM_COMPAT_CONFIG")
            .env_remove("PROTON_ADD_CONFIG")
            .env("WINEDLLOVERRIDES", "");
        apply_game_dll_overrides(
            &mut automatic,
            &BTreeMap::new(),
            &[("xinput1_3", DllLoadOrder::Builtin)],
        )
        .unwrap();
        let value = automatic
            .get_envs()
            .find(|(key, _)| *key == "WINEDLLOVERRIDES")
            .unwrap()
            .1
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(wine_lookup(value, &registry, false), "b");
        assert_eq!(wine_lookup(value, &registry, true), "b");
        // Exact-path registry rules remain more specific; no arbitrary registry repair.
        let exact = BTreeMap::from([("c:\\game\\xinput1_3", "n"), ("*xinput1_3", "n")]);
        assert_eq!(wine_lookup(value, &exact, false), "n");
    }

    #[test]
    fn dll_managed_xinput_respects_runtime_flag_but_explicit_rows_remain_explicit() {
        for (key, value, skip) in [
            ("STEAM_COMPAT_CONFIG", "foo,usenativexinput13,bar", true),
            ("PROTON_ADD_CONFIG", "usenativexinput13", true),
            ("PROTON_ADD_CONFIG", "usenativexinput13=0", true),
            ("STEAM_COMPAT_CONFIG", "usenativexinput13=0", false),
            ("PROTON_ADD_CONFIG", "notusenativexinput13", false),
        ] {
            let mut command = Command::new("inert-not-executed");
            command
                .env_remove("STEAM_COMPAT_CONFIG")
                .env_remove("PROTON_ADD_CONFIG")
                .env(key, value)
                .env("WINEDLLOVERRIDES", "other=n")
                .env("PROTON_REMOVE_CONFIG", "usenativexinput13");
            apply_game_dll_overrides(
                &mut command,
                &BTreeMap::new(),
                &[
                    ("xinput1_3", DllLoadOrder::Builtin),
                    ("xinput1_2", DllLoadOrder::Builtin),
                ],
            )
            .unwrap();
            let composed = command
                .get_envs()
                .find(|(key, _)| *key == "WINEDLLOVERRIDES")
                .unwrap()
                .1
                .unwrap()
                .to_str()
                .unwrap();
            assert_eq!(composed.contains("xinput1_3"), !skip, "{key} {value}");
            assert!(composed.contains("xinput1_2,*xinput1_2=b"));
            assert!(composed.ends_with("other=n"));
            apply_game_dll_overrides(
                &mut command,
                &BTreeMap::from([("xinput1_3".into(), DllLoadOrder::Builtin)]),
                &[],
            )
            .unwrap();
            assert!(
                command
                    .get_envs()
                    .find(|(key, _)| *key == "WINEDLLOVERRIDES")
                    .unwrap()
                    .1
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .ends_with("xinput1_3,*xinput1_3=b")
            );
        }
        // Child env_remove is authoritative even when the parent supplied the flag.
        if std::env::var_os("LUDOMERE_XINPUT_FLAG_FIXTURE").is_none() {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "compatibility::dll_overrides::tests::dll_managed_xinput_respects_runtime_flag_but_explicit_rows_remain_explicit"])
                .env("LUDOMERE_XINPUT_FLAG_FIXTURE", "1")
                .env("STEAM_COMPAT_CONFIG", "usenativexinput13")
                .env("PROTON_ADD_CONFIG", "usenativexinput13=0");
            assert!(child.status().unwrap().success());
        } else {
            let mut command = Command::new("inert-not-executed");
            command
                .env_remove("STEAM_COMPAT_CONFIG")
                .env_remove("PROTON_ADD_CONFIG")
                .env("WINEDLLOVERRIDES", "");
            apply_game_dll_overrides(
                &mut command,
                &BTreeMap::new(),
                &[("xinput1_3", DllLoadOrder::Builtin)],
            )
            .unwrap();
            assert_eq!(
                command
                    .get_envs()
                    .find(|(key, _)| *key == "WINEDLLOVERRIDES")
                    .unwrap()
                    .1
                    .unwrap(),
                "xinput1_3,*xinput1_3=b"
            );
        }
    }

    #[test]
    fn dll_names_reject_expressions_paths_and_duplicates() {
        for name in [
            "",
            "..",
            "a;b",
            "a=b",
            "a,b",
            "*dinput8",
            "../dinput8",
            "C:\\a",
            "a\n",
            "a\0",
            "a b",
            "foo.dll.dll",
        ] {
            assert!(
                normalize_dll_overrides([(name.into(), DllLoadOrder::Native)]).is_err(),
                "{name:?}"
            );
        }
        assert!(
            normalize_dll_overrides([
                ("DINPUT8.DLL".into(), DllLoadOrder::Native),
                ("dinput8".into(), DllLoadOrder::Builtin)
            ])
            .is_err()
        );
        assert_eq!(
            normalize_dll_overrides([(" DINPUT8.DLL ".into(), DllLoadOrder::Disabled)]).unwrap()["dinput8"],
            DllLoadOrder::Disabled
        );
    }

    #[test]
    fn dll_composition_preserves_groups_and_explicit_choices_win() {
        let mut command = Command::new("inert-not-executed");
        command.env(
            "WINEDLLOVERRIDES",
            "winemenubuilder.exe=d;XINPUT1_3.dll,other=n;*dinput8=b;C:\\game\\dxgi.dll=n",
        );
        let overrides = BTreeMap::from([
            ("xinput1_3".into(), DllLoadOrder::Disabled),
            ("dinput8".into(), DllLoadOrder::NativeThenBuiltin),
            ("dxgi".into(), DllLoadOrder::BuiltinThenNative),
        ]);
        apply_game_dll_overrides(
            &mut command,
            &overrides,
            &[
                ("xinput1_3", DllLoadOrder::Builtin),
                ("xinput1_2", DllLoadOrder::Builtin),
            ],
        )
        .unwrap();
        let value = command
            .get_envs()
            .find(|(key, _)| *key == "WINEDLLOVERRIDES")
            .unwrap()
            .1
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(
            value,
            "xinput1_2,*xinput1_2=b;winemenubuilder.exe=d;XINPUT1_3.dll,other=n;*dinput8=b;C:\\game\\dxgi.dll=n;*dinput8=n,b;dinput8,*dinput8=n,b;C:\\game\\dxgi.dll=b,n;dxgi,*dxgi=b,n;XINPUT1_3.dll=;xinput1_3,*xinput1_3="
        );
        assert_eq!(
            DllLoadOrder::ALL.map(DllLoadOrder::value),
            ["n", "b", "n,b", "b,n", ""]
        );
        assert_eq!(command.get_args().count(), 0);
    }

    #[test]
    fn dll_defaults_respect_inherited_wildcard_and_empty_choices_do_nothing() {
        let mut command = Command::new("inert-not-executed");
        command.env("WINEDLLOVERRIDES", "*XINPUT1_3.dll=n;other=b");
        apply_game_dll_overrides(
            &mut command,
            &BTreeMap::new(),
            &[("xinput1_3", DllLoadOrder::Builtin)],
        )
        .unwrap();
        assert_eq!(
            command.get_envs().next().unwrap().1.unwrap(),
            "*XINPUT1_3.dll=n;other=b"
        );
        let mut empty = Command::new("inert-not-executed");
        apply_game_dll_overrides(&mut empty, &BTreeMap::new(), &[]).unwrap();
        assert_eq!(empty.get_envs().count(), 0);
        use std::os::unix::ffi::OsStringExt;
        empty.env("WINEDLLOVERRIDES", std::ffi::OsString::from_vec(vec![0xff]));
        apply_game_dll_overrides(&mut empty, &BTreeMap::new(), &[]).unwrap();
        assert!(
            apply_game_dll_overrides(
                &mut empty,
                &BTreeMap::from([("dinput8".into(), DllLoadOrder::Builtin)]),
                &[]
            )
            .is_err()
        );
    }
}
