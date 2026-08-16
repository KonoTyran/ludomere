use ludomere::{
    config::{Config, GameLibrary},
    domain::{InstallationState, InstalledGame},
    installation,
    state::{InstallationOperationRecord, StateStore},
};
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, process::Command, time::Duration};

#[test]
fn recovers_an_interrupted_installation_in_a_fresh_process() {
    let root = std::env::temp_dir().join(format!(
        "gog-installation-recovery-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    run_helper("persist", &root);
    run_helper("recover", &root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn shutdown_returns_an_active_installer_to_the_persistent_queue() {
    let root = std::env::temp_dir().join(format!(
        "gog-installation-shutdown-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    run_helper("shutdown", &root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn shutdown_returns_an_active_uninstaller_to_the_persistent_queue() {
    let root = std::env::temp_dir().join(format!(
        "gog-uninstallation-shutdown-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    run_helper("shutdown-uninstall", &root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelling_recovered_queued_work_prevents_future_recovery() {
    let root = std::env::temp_dir().join(format!(
        "gog-installation-cancel-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    run_helper("persist", &root);
    run_helper("cancel-queued", &root);
    run_helper("confirm-cancelled", &root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "launched as a subprocess by the installation-recovery integration test"]
fn installation_recovery_helper_process() {
    let phase = std::env::var("LUDOMERE_INSTALL_TEST_PHASE").unwrap();
    let root = PathBuf::from(std::env::var_os("LUDOMERE_INSTALL_TEST_ROOT").unwrap());
    let product_id = 1449651388;
    let library = root.join("games");
    let mut config = Config::load_or_create().unwrap();
    config.game_libraries = vec![GameLibrary {
        id: "test".into(),
        name: "Test".into(),
        path: library.clone(),
        default: true,
    }];
    config.save().unwrap();
    let journal = library.join(".ludomere/staging/grim_dawn.operation.json");

    match phase.as_str() {
        "persist" => {
            let game = test_game(product_id, root.join("games/grim_dawn"));
            let plan = json!({
                "game": game,
                "additional_installers": [],
                "install_base": true,
                "interactive_prompts": false
            });
            StateStore::open()
                .unwrap()
                .upsert_installation_operation(&InstallationOperationRecord {
                    product_id,
                    operation: "install".into(),
                    state: "running".into(),
                    plan_json: serde_json::to_string(&plan).unwrap(),
                    message: Some("Running native installer".into()),
                    percentage: Some(37),
                    queue_position: Some(4),
                    created_at: 10,
                    updated_at: 20,
                    completed_at: None,
                })
                .unwrap();
        }
        "recover" => {
            let events = installation::subscribe_installation_events();
            assert_eq!(installation::recover_interrupted_operations().unwrap(), 1);
            let snapshot = installation::installation_operation_snapshot(product_id).unwrap();
            assert!(snapshot.queued);
            assert_eq!(snapshot.state, InstallationState::Pending);
            assert_eq!(snapshot.percentage, None);
            assert_eq!(
                snapshot.message.as_deref(),
                Some("Queued for resumed installation")
            );

            let event = events.try_recv().unwrap();
            match event {
                installation::InstallationManagerEvent::OperationRecovered(snapshot) => {
                    assert_eq!(snapshot.product_id, product_id);
                    assert!(snapshot.queued);
                }
                other => panic!("unexpected recovery event: {other:?}"),
            }

            let record: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
            assert_eq!(record["record"]["state"], "queued");
            assert_eq!(record["record"]["queue_position"], 4);
            assert!(record["record"]["percentage"].is_null());
            assert!(
                StateStore::open()
                    .unwrap()
                    .installation_operations()
                    .unwrap()
                    .is_empty()
            );
        }
        "shutdown" => {
            let installer = root.join("setup.sh");
            std::fs::write(&installer, "#!/bin/sh\nsleep 30\n").unwrap();
            let mut permissions = std::fs::metadata(&installer).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&installer, permissions).unwrap();

            let mut game = test_game(product_id, root.join("games/grim_dawn"));
            game.installer_files = vec![installer];
            assert!(installation::enqueue_installation(
                game,
                Vec::new(),
                true,
                false
            ));
            wait_for_operation(product_id, InstallationState::Installing);
            installation::shutdown();

            let record: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
            assert_eq!(record["record"]["state"], "queued");
            assert_eq!(
                record["record"]["message"],
                "Queued after application shutdown"
            );
            assert!(record["record"]["percentage"].is_null());
        }
        "shutdown-uninstall" => {
            let installation_directory = root.join("games/grim_dawn");
            std::fs::create_dir_all(&installation_directory).unwrap();
            let uninstaller = installation_directory.join("uninstall-grim-dawn.sh");
            std::fs::write(&uninstaller, "#!/bin/sh\nsleep 30\n").unwrap();
            let mut permissions = std::fs::metadata(&uninstaller).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&uninstaller, permissions).unwrap();

            let mut game = test_game(product_id, installation_directory);
            game.state = InstallationState::Installed;
            game.installed_at = Some(10);
            installation::write_installation_marker(
                &installation::installation_marker_from_game(&game, Vec::new()),
                &game.installation_directory,
            )
            .unwrap();
            let marker_directory = game.installation_directory.clone();
            assert!(installation::enqueue_uninstallation(game));
            wait_for_operation(product_id, InstallationState::Uninstalling);
            installation::shutdown();

            let record: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
            assert_eq!(record["record"]["operation"], "uninstall");
            assert_eq!(record["record"]["state"], "queued");
            assert_eq!(
                record["record"]["message"],
                "Queued after application shutdown"
            );
            assert!(
                installation::load_installation_marker(&marker_directory)
                    .unwrap()
                    .is_some()
            );
        }
        "cancel-queued" => {
            assert_eq!(installation::recover_interrupted_operations().unwrap(), 1);
            assert!(installation::cancel_operation(product_id));
            assert!(!journal.exists());
        }
        "confirm-cancelled" => {
            assert_eq!(installation::recover_interrupted_operations().unwrap(), 0);
            assert!(installation::installation_operation_snapshot(product_id).is_none());
        }
        other => panic!("unknown installation recovery phase {other}"),
    }
}

fn wait_for_operation(product_id: i64, state: InstallationState) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if installation::installation_operation_snapshot(product_id)
            .is_some_and(|snapshot| snapshot.state == state)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("installation operation did not reach {state:?}");
}

fn run_helper(phase: &str, root: &std::path::Path) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "installation_recovery_helper_process",
            "--nocapture",
        ])
        .env("XDG_DATA_HOME", root.join("state"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("LUDOMERE_INSTALL_TEST_PHASE", phase)
        .env("LUDOMERE_INSTALL_TEST_ROOT", root)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "installation recovery phase {phase} failed"
    );
}

fn test_game(product_id: i64, installation_directory: PathBuf) -> InstalledGame {
    InstalledGame {
        product_id,
        library_id: "test".into(),
        installed_version: Some("1.0".into()),
        installation_directory,
        installer_revision_id: Some(7),
        installer_job_id: None,
        installer_files: vec![PathBuf::from("/downloads/setup.sh")],
        installer_complete: true,
        installer_operating_system: Some("linux".into()),
        installer_language: Some("English".into()),
        compatibility: None,
        primary_executable: None,
        launch_arguments: Vec::new(),
        state: InstallationState::Pending,
        error: None,
        installed_at: None,
        verified_at: None,
        last_played_at: None,
        playtime_seconds: 0,
        created_at: 10,
        updated_at: 10,
    }
}
