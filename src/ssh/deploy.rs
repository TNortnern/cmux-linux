use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

/// Packaged locations, root-owned, preferred over the per-user copy.
const PACKAGED_DAEMON_PATHS: [&str; 2] = [
    "/usr/lib/cmux/cmuxd-remote",   // .deb
    "/usr/lib64/cmux/cmuxd-remote", // .rpm
];

/// Path to the cmuxd-remote binary to deploy.
/// Looks in: /usr/lib/cmux, /usr/lib64/cmux, then
/// ~/.local/share/cmux/bin/cmuxd-remote-linux-amd64 (development install).
pub fn local_daemon_path() -> PathBuf {
    for candidate in PACKAGED_DAEMON_PATHS {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return path;
        }
    }
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|dir| dir.join("cmux/bin/cmuxd-remote-linux-amd64"))
        .unwrap_or_else(|| PathBuf::from(PACKAGED_DAEMON_PATHS[0]))
}

/// Base command for ssh or scp. Never prompts (BatchMode), gives up on an
/// unreachable host after 10s, and reads no stdin so it cannot hang on a prompt.
/// Callers add `--` and then the destination, so a target can never be read as an option.
pub(crate) fn ssh_base(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"])
        .stdin(Stdio::null());
    cmd
}

/// Deploy cmuxd-remote to remote host via scp.
/// Copies to ~/.local/bin/cmuxd-remote on the remote.
pub async fn deploy_remote(target: &str) -> Result<(), String> {
    crate::ssh::target::validate_target(target)?;

    let local_path = local_daemon_path();
    if !local_path.exists() {
        return Err(format!(
            "cmuxd-remote binary not found at {}. Run: ./scripts/install-cmuxd-remote.sh",
            local_path.display()
        ));
    }

    // Ensure remote directory exists
    let mkdir_status = ssh_base("ssh")
        .arg("--")
        .arg(target)
        .args(["mkdir", "-p", "~/.local/bin"])
        .status()
        .await
        .map_err(|e| format!("SSH mkdir failed: {e}"))?;
    if !mkdir_status.success() {
        return Err("Failed to create remote directory".to_string());
    }

    // scp the binary
    let remote_dest = format!("{target}:~/.local/bin/cmuxd-remote");
    let scp_status = ssh_base("scp")
        .arg("--")
        .arg(&local_path)
        .arg(&remote_dest)
        .status()
        .await
        .map_err(|e| format!("scp failed: {e}"))?;
    if !scp_status.success() {
        return Err(format!("Failed to deploy remote daemon to {target}"));
    }

    // Make executable
    let chmod_status = ssh_base("ssh")
        .arg("--")
        .arg(target)
        .args(["chmod", "+x", "~/.local/bin/cmuxd-remote"])
        .status()
        .await
        .map_err(|e| format!("SSH chmod failed: {e}"))?;
    if !chmod_status.success() {
        return Err("Failed to set executable permissions".to_string());
    }

    Ok(())
}
