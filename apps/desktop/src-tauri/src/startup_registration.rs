//! Reconcile the stored launch-at-login preference with the operating system.
//!
//! Development builds never mutate login items. Packaged builds use
//! `SMAppService.mainApp` on macOS 13+, the per-user Run key on Windows, and a
//! Desktop Entry on Linux. Failures are deliberately best-effort: the
//! preference remains stored, the app remains usable, and a later launch or
//! toggle retries it.

use tauri::Runtime;

#[cfg(target_os = "linux")]
use tauri::Manager;

use crate::store::AppSettings;

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationState {
    Unregistered,
    Enabled,
    RequiresApproval,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationAction {
    None,
    Enable,
    Disable,
    AwaitApproval,
}

#[cfg(any(target_os = "macos", test))]
fn action_for(desired: bool, state: RegistrationState) -> RegistrationAction {
    match (desired, state) {
        (true, RegistrationState::Enabled) | (false, RegistrationState::Unregistered) => {
            RegistrationAction::None
        }
        (true, RegistrationState::Unregistered) => RegistrationAction::Enable,
        (true, RegistrationState::RequiresApproval) => RegistrationAction::AwaitApproval,
        (false, RegistrationState::Enabled | RegistrationState::RequiresApproval) => {
            RegistrationAction::Disable
        }
    }
}

/// Whether a settings write should touch the platform registration.
///
/// A first-run toggle only records intent. The platform is changed once the
/// flow finishes, or when an already-onboarded reader changes the preference.
pub(crate) fn should_reconcile_after_save(previous: &AppSettings, saved: &AppSettings) -> bool {
    let finished_onboarding = !previous.onboarding_completed && saved.onboarding_completed;
    let preference_changed = previous.launch_at_login != saved.launch_at_login;
    saved.onboarding_completed && (finished_onboarding || preference_changed)
}

/// Apply the desired registration without making a settings write fail.
pub fn reconcile<R: Runtime>(app: &tauri::AppHandle<R>, desired: bool) {
    if !registration_is_active(cfg!(debug_assertions), cfg!(feature = "distribution")) {
        let _ = (app, desired);
        return;
    }
    reconcile_platform(app, desired);
}

fn registration_is_active(debug_assertions: bool, distribution: bool) -> bool {
    !debug_assertions && distribution
}

#[cfg(target_os = "macos")]
fn reconcile_platform<R: Runtime>(app: &tauri::AppHandle<R>, desired: bool) {
    // AppService is !Send/!Sync and ServiceManagement requires the application
    // thread. Settings writes can arrive on a command worker, so construct and
    // use the service inside the main-thread closure.
    if let Err(error) = app.run_on_main_thread(move || reconcile_macos(desired)) {
        ::tracing::error!(
            event = "launch_at_login_schedule_failed",
            error = %error
        );
    }
}

#[cfg(target_os = "macos")]
fn reconcile_macos(desired: bool) {
    use smappservice_rs::{AppService, ServiceManagementError, ServiceStatus, ServiceType};

    let service = AppService::new(ServiceType::MainApp);
    let state = match service.status() {
        ServiceStatus::Enabled => RegistrationState::Enabled,
        ServiceStatus::RequiresApproval => RegistrationState::RequiresApproval,
        ServiceStatus::NotRegistered | ServiceStatus::NotFound => RegistrationState::Unregistered,
    };

    match action_for(desired, state) {
        RegistrationAction::None => {}
        RegistrationAction::AwaitApproval => {
            ::tracing::warn!(event = "launch_at_login_requires_approval");
        }
        RegistrationAction::Enable => match service.register() {
            Ok(()) | Err(ServiceManagementError::AlreadyRegistered) => {}
            Err(error) => {
                ::tracing::warn!(event = "launch_at_login_enable_failed", error = %error);
            }
        },
        RegistrationAction::Disable => match service.unregister() {
            Ok(()) | Err(ServiceManagementError::JobNotFound) => {}
            Err(error) => {
                ::tracing::warn!(event = "launch_at_login_disable_failed", error = %error);
            }
        },
    }
}

#[cfg(target_os = "windows")]
fn reconcile_platform<R: Runtime>(app: &tauri::AppHandle<R>, desired: bool) {
    if let Err(error) = reconcile_windows(app, desired) {
        ::tracing::warn!(event = "launch_at_login_reconcile_failed", error = %error);
    }
}

#[cfg(target_os = "windows")]
fn reconcile_windows<R: Runtime>(app: &tauri::AppHandle<R>, desired: bool) -> anyhow::Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const RUN_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";
    let command = if desired {
        Some(windows_run_command(
            &std::env::current_exe()?.to_string_lossy(),
        ))
    } else {
        None
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    reconcile_windows_run(&hkcu, RUN_KEY, command.as_deref())?;
    let _ = app;
    Ok(())
}

#[cfg(target_os = "windows")]
fn reconcile_windows_run(
    root: &winreg::RegKey,
    path: &str,
    command: Option<&str>,
) -> std::io::Result<()> {
    use winreg::enums::{KEY_READ, KEY_SET_VALUE};

    let run = match root.open_subkey_with_flags(path, KEY_READ | KEY_SET_VALUE) {
        Ok(run) => run,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if command.is_none() {
                return Ok(());
            }
            root.create_subkey_with_flags(path, KEY_READ | KEY_SET_VALUE)?
                .0
        }
        Err(error) => return Err(error),
    };
    // Windows owns startup approval. Keep its records on every transition.
    reconcile_windows_run_value(&run, command)
}

#[cfg(target_os = "windows")]
fn reconcile_windows_run_value(run: &winreg::RegKey, command: Option<&str>) -> std::io::Result<()> {
    use winreg::types::ToRegValue;

    let current = match run.get_raw_value("antiburn") {
        Ok(current) => Some(current),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let desired = command.map(str::to_owned);
    if current == desired.as_ref().map(ToRegValue::to_reg_value) {
        return Ok(());
    }
    if let Some(command) = command {
        run.set_value("antiburn", &command)
    } else {
        match run.delete_value("antiburn") {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(any(target_os = "windows", test))]
fn windows_run_command(executable: &str) -> String {
    // Keep the background flag outside the quoted executable path.
    format!("\"{executable}\" --background")
}

#[cfg(target_os = "linux")]
fn reconcile_platform<R: Runtime>(app: &tauri::AppHandle<R>, desired: bool) {
    if let Err(error) = reconcile_linux(app, desired) {
        ::tracing::warn!(event = "launch_at_login_reconcile_failed", error = %error);
    }
}

#[cfg(target_os = "linux")]
fn reconcile_linux<R: Runtime>(app: &tauri::AppHandle<R>, desired: bool) -> anyhow::Result<()> {
    let file = app.path().config_dir()?.join("autostart/antiburn.desktop");
    if !desired {
        if let Err(error) = std::fs::remove_file(&file)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error.into());
        }
        return Ok(());
    }

    let executable = app
        .env()
        .appimage
        .map(std::path::PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)?;
    let executable = executable
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("application path is not valid UTF-8"))?;
    let contents = linux_desktop_entry(executable);

    if std::fs::read_to_string(&file)
        .as_deref()
        .is_ok_and(|current| current == contents.as_str())
    {
        return Ok(());
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, contents)?;
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn linux_desktop_entry(executable: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=antiburn\nComment=Start antiburn at login\nExec={} --background\nStartupNotify=false\nTerminal=false\n",
        desktop_exec_quote(executable)
    )
}

#[cfg(any(target_os = "linux", test))]
fn desktop_exec_quote(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        if character == '%' {
            escaped.push('%');
        }
        if matches!(character, '"' | '`' | '$' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('"');
    escaped
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn reconcile_platform<R: Runtime>(_app: &tauri::AppHandle<R>, _desired: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_actions_cover_each_platform_state() {
        assert_eq!(
            action_for(true, RegistrationState::Unregistered),
            RegistrationAction::Enable
        );
        assert_eq!(
            action_for(true, RegistrationState::Enabled),
            RegistrationAction::None
        );
        assert_eq!(
            action_for(true, RegistrationState::RequiresApproval),
            RegistrationAction::AwaitApproval
        );
        assert_eq!(
            action_for(false, RegistrationState::Unregistered),
            RegistrationAction::None
        );
        assert_eq!(
            action_for(false, RegistrationState::Enabled),
            RegistrationAction::Disable
        );
        assert_eq!(
            action_for(false, RegistrationState::RequiresApproval),
            RegistrationAction::Disable
        );
    }

    #[test]
    fn only_distribution_release_builds_touch_login_items() {
        assert!(!registration_is_active(true, false));
        assert!(!registration_is_active(true, true));
        assert!(!registration_is_active(false, false));
        assert!(registration_is_active(false, true));
    }

    #[test]
    fn platform_commands_quote_paths_with_spaces_and_reserved_characters() {
        assert_eq!(
            windows_run_command(r"C:\Program Files\antiburn\antiburn.exe"),
            r#""C:\Program Files\antiburn\antiburn.exe" --background"#
        );
        assert!(
            linux_desktop_entry("/opt/100%real/anti burn/$stable`/antiburn")
                .contains("Exec=\"/opt/100%%real/anti burn/\\$stable\\`/antiburn\" --background\n")
        );
    }

    #[test]
    fn onboarding_defers_registration_until_the_flow_finishes() {
        let previous = AppSettings {
            launch_at_login: false,
            ..AppSettings::default()
        };
        let toggled = AppSettings {
            launch_at_login: true,
            ..previous.clone()
        };
        assert!(!should_reconcile_after_save(&previous, &toggled));

        let finished = AppSettings {
            onboarding_completed: true,
            ..toggled.clone()
        };
        assert!(should_reconcile_after_save(&toggled, &finished));
    }

    #[test]
    fn a_completed_install_reconciles_only_when_the_preference_changes() {
        let previous = AppSettings {
            onboarding_completed: true,
            ..AppSettings::default()
        };
        let unrelated = AppSettings {
            activity_window_days: 14,
            ..previous.clone()
        };
        assert!(!should_reconcile_after_save(&previous, &unrelated));

        let disabled = AppSettings {
            launch_at_login: false,
            ..previous.clone()
        };
        assert!(should_reconcile_after_save(&previous, &disabled));
        assert!(should_reconcile_after_save(&disabled, &previous));
    }
}

#[cfg(target_os = "windows")]
#[cfg(test)]
mod windows_tests {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_BINARY};

    struct TestRegistry {
        path: String,
        root: RegKey,
    }

    impl TestRegistry {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = format!(
                r"Software\antiburn-tests\{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("test clock follows the Unix epoch")
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            );
            let root = RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(&path)
                .expect("create isolated test registry")
                .0;
            Self { path, root }
        }

        fn run(&self) -> RegKey {
            self.root
                .create_subkey("Run")
                .expect("create test Run key")
                .0
        }
    }

    impl Drop for TestRegistry {
        fn drop(&mut self) {
            RegKey::predef(HKEY_CURRENT_USER)
                .delete_subkey_all(&self.path)
                .expect("remove isolated test registry");
        }
    }

    #[test]
    fn unchanged_run_value_needs_no_write_access() {
        let registry = TestRegistry::new();
        let command = windows_run_command(r"C:\Program Files\antiburn\antiburn.exe");
        registry.run().set_value("antiburn", &command).unwrap();
        let read_only = registry
            .root
            .open_subkey_with_flags("Run", KEY_READ)
            .unwrap();
        reconcile_windows_run_value(&read_only, Some(&command)).unwrap();
    }

    #[test]
    fn absent_run_key_and_value_are_registered_then_updated_once() {
        let registry = TestRegistry::new();
        let command = windows_run_command(r"C:\Program Files\antiburn\antiburn.exe");
        reconcile_windows_run(&registry.root, "Run", Some(&command)).unwrap();
        assert_eq!(
            registry.run().get_value::<String, _>("antiburn").unwrap(),
            command
        );
        let updated = windows_run_command(r"D:\Moved app\antiburn.exe");
        reconcile_windows_run(&registry.root, "Run", Some(&updated)).unwrap();
        let read_only = registry
            .root
            .open_subkey_with_flags("Run", KEY_READ)
            .unwrap();
        assert_eq!(
            read_only.get_value::<String, _>("antiburn").unwrap(),
            updated
        );
        reconcile_windows_run_value(&read_only, Some(&updated)).unwrap();
        registry.run().delete_value("antiburn").unwrap();
        reconcile_windows_run(&registry.root, "Run", Some(&updated)).unwrap();
        assert_eq!(
            registry.run().get_value::<String, _>("antiburn").unwrap(),
            updated
        );
    }

    #[test]
    fn disable_removes_only_our_value_and_absence_is_a_noop() {
        let registry = TestRegistry::new();
        reconcile_windows_run(&registry.root, "Run", None).unwrap();
        assert!(registry.root.open_subkey("Run").is_err());
        let run = registry.run();
        run.set_value("antiburn", &"old command").unwrap();
        run.set_value("another-app", &"keep me").unwrap();
        reconcile_windows_run(&registry.root, "Run", None).unwrap();
        assert_eq!(
            run.get_value::<String, _>("antiburn").unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(
            run.get_value::<String, _>("another-app").unwrap(),
            "keep me"
        );
        let read_only = registry
            .root
            .open_subkey_with_flags("Run", KEY_READ)
            .unwrap();
        reconcile_windows_run_value(&read_only, None).unwrap();
    }

    #[test]
    fn windows_disable_survives_launch_update_and_explicit_reenable() {
        let registry = TestRegistry::new();
        let approved = registry
            .root
            .create_subkey(r"StartupApproved\Run")
            .unwrap()
            .0;
        let disabled = winreg::RegValue {
            vtype: REG_BINARY,
            bytes: vec![3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8].into(),
        };
        approved.set_raw_value("antiburn", &disabled).unwrap();
        let command = windows_run_command(r"C:\antiburn\antiburn.exe");
        for command in [
            Some(command.as_str()),
            Some(command.as_str()),
            Some(r#""D:\Updated app\antiburn.exe" --background"#),
            None,
            Some(command.as_str()),
        ] {
            reconcile_windows_run(&registry.root, "Run", command).unwrap();
            assert_eq!(approved.get_raw_value("antiburn").unwrap(), disabled);
        }
    }

    #[test]
    fn registry_read_errors_do_not_overwrite_existing_values() {
        let registry = TestRegistry::new();
        let run = registry.run();
        run.set_value("antiburn", &42u32).unwrap();
        let write_only = registry
            .root
            .open_subkey_with_flags("Run", KEY_SET_VALUE)
            .unwrap();
        assert!(reconcile_windows_run_value(&write_only, Some("new command")).is_err());
        assert_eq!(run.get_value::<u32, _>("antiburn").unwrap(), 42);
        reconcile_windows_run(&registry.root, "Run", None).unwrap();
        assert_eq!(
            run.get_raw_value("antiburn").unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
}
