//! Launch requirements shared by terminal and desktop entry points.
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use entanglo_core::net::trust_store;

const CONTROLLER_SETTING: &str = "controller_mode";

/// Whether the user asked to share this computer's keyboard and mouse.
pub fn controller_mode_requested() -> bool {
    std::env::var("ENTANGLO_ENABLE_CONTROLLER").as_deref() == Ok("1")
        || trust_store::read_setting(CONTROLLER_SETTING).is_some()
}

/// Whether this running instance started with local input capture.
pub fn controller_mode_running() -> bool {
    std::env::var_os("ENTANGLO_RECEIVER_ONLY").is_none()
}

pub fn save_controller_mode(enabled: bool) -> std::io::Result<()> {
    if enabled {
        trust_store::write_setting(CONTROLLER_SETTING, "enabled")
    } else {
        trust_store::remove_setting(CONTROLLER_SETTING)
    }
}

/// Launches `executable` once this process has exited, so the replacement can
/// register the application name. The caller must quit afterwards.
pub fn spawn_replacement(executable: &Path) -> std::io::Result<()> {
    Command::new("sh")
        .args(["-c", "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; exec \"$2\"", "entanglo-restart", &std::process::id().to_string()])
        .arg(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn prepare() {
    // Receiving is the safe default; controller capture is an explicit opt-in.
    // A restarted instance inherits this environment, so it is decided afresh.
    if controller_mode_requested() {
        std::env::remove_var("ENTANGLO_RECEIVER_ONLY");
    } else {
        std::env::set_var("ENTANGLO_RECEIVER_ONLY", "1");
    }
    let needs_group = PathBuf::from("/dev/uinput").exists()
        && std::fs::OpenOptions::new().write(true).open("/dev/uinput").is_err();
    if needs_group && std::env::var_os("ENTANGLO_GROUP_REEXEC").is_none() {
        let username = std::env::var("USER").unwrap_or_default();
        let allowed = std::fs::read_to_string("/etc/group").unwrap_or_default().lines().any(|line| {
            let fields: Vec<_> = line.split(':').collect();
            fields.len() == 4 && fields[0] == "input" && fields[3].split(',').any(|user| user == username)
        });
        if allowed {
            let executable = std::env::current_exe().expect("resolve executable path");
            let command = std::iter::once(executable.to_string_lossy().into_owned())
                .chain(std::env::args().skip(1)).map(|arg| shell_quote(&arg)).collect::<Vec<_>>().join(" ");
            let error = Command::new("sg").args(["input", "-c", &format!("exec {command}")])
                .env("ENTANGLO_GROUP_REEXEC", "1").exec();
            eprintln!("Could not activate input group: {error}");
        }
    }
}

pub fn prepare_logs() {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"))
        .join("entanglo");
    if std::fs::create_dir_all(&state).is_ok() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
        let name = format!("session-{stamp}-{}.log", std::process::id());
        std::env::set_var("ENTANGLO_LOG_FILE", state.join(&name));
        for alias in ["last.log", "ultimo.log"] {
            let link = state.join(alias);
            if link.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) { let _ = std::fs::remove_file(&link); }
            let _ = std::os::unix::fs::symlink(&name, link);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::shell_quote;
    #[test]
    fn executable_paths_and_arguments_are_shell_quoted() {
        assert_eq!(shell_quote("/tmp/my app"), "'/tmp/my app'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote("$(false)"), "'$(false)'");
    }
}
