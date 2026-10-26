//! Supervised session host: `fastty host install|uninstall|status`.
//!
//! The app already keeps sessions alive after quit by spawning a detached
//! `--fastty-session-host` child. This module adds OS-level supervision on
//! top: a launchd agent on macOS or a systemd user unit on Linux that
//! starts the host at login and restarts it after a crash. The instance
//! lock in `daemon.rs` still guarantees a single host, so the supervised
//! service and the detached child never run at the same time.

use std::path::PathBuf;

pub const LABEL: &str = "com.fastty.host";
pub const SERVICE_NAME: &str = "fastty-host.service";

const HOST_FLAG: &str = "--fastty-session-host";

/// Supervised host service management (`fastty host install|uninstall|status`).
pub fn run_host_service_command(mut args: impl Iterator<Item = String>) {
    let Some(command) = args.next() else {
        eprintln!("Usage: fastty host <install|uninstall|status>");
        std::process::exit(1);
    };
    let result = match command.as_str() {
        "install" => install(),
        "uninstall" => uninstall(),
        "status" => {
            status();
            std::process::exit(0);
        }
        other => {
            eprintln!("fastty host: unknown subcommand {other}\nUsage: fastty host <install|uninstall|status>");
            std::process::exit(1);
        }
    };
    if let Err(error) = result {
        eprintln!("fastty host {command}: {error}");
        std::process::exit(1);
    }
    std::process::exit(0);
}

fn launchd_plist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn systemd_unit_path() -> Option<PathBuf> {
    dirs::config_dir().map(|config| {
        config
            .join("systemd/user")
            .join(SERVICE_NAME)
    })
}

fn plist_contents(executable: &std::path::Path) -> String {
    let exe = executable.display();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>{HOST_FLAG}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ProcessType</key>
  <string>Background</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>FASTTY_SUPERVISED</key>
    <string>1</string>
  </dict>
  <key>StandardErrorPath</key>
  <string>/tmp/fastty-host.log</string>
</dict>
</plist>
"#
    )
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn systemd_unit_contents(executable: &std::path::Path) -> String {
    let exe = executable.display();
    format!(
        r#"[Unit]
Description=Fastty session host
After=graphical-session.target

[Service]
ExecStart={exe} {HOST_FLAG}
Restart=always
RestartSec=2

[Install]
WantedBy=default.target
"#
    )
}

fn run(command: &str, args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new(command)
        .args(args)
        .output()
        .map_err(|error| format!("{command} failed to start: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "{command} {} failed: {}{}",
        args.join(" "),
        stderr.trim(),
        stdout.trim()
    ))
}

fn atomic_write(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("creating {}: {error}", parent.display()))?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, contents).map_err(|error| format!("writing {}: {error}", temp.display()))?;
    std::fs::rename(&temp, path)
        .map_err(|error| format!("installing {}: {error}", path.display()))
}

/// Install and start the supervised host service.
pub fn install() -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate the fastty binary: {error}"))?;
    #[cfg(target_os = "macos")]
    {
        let Some(path) = launchd_plist_path() else {
            return Err("could not resolve the home directory".into());
        };
        atomic_write(&path, &plist_contents(&executable))?;
        let uid = unsafe { libc::getuid() };
        let _ = run("launchctl", &["bootout", &format!("gui/{uid}/{LABEL}")]);
        run(
            "launchctl",
            &["bootstrap", &format!("gui/{uid}"), &path.display().to_string()],
        )?;
        println!("Fastty host installed as a launchd agent: {}", path.display());
        println!("The host now starts at login and restarts if it crashes.");
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let Some(path) = systemd_unit_path() else {
            return Err("could not resolve the systemd user directory".into());
        };
        atomic_write(&path, &systemd_unit_contents(&executable))?;
        run("systemctl", &["--user", "daemon-reload"])?;
        run("systemctl", &["--user", "enable", "--now", SERVICE_NAME])?;
        println!("Fastty host installed as a systemd user unit: {}", path.display());
        println!("The host now starts at login and restarts if it crashes.");
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (executable, plist_contents, systemd_unit_contents);
        Err("fastty host: not supported on this platform yet".into())
    }
}

/// Stop and remove the supervised host service. Running sessions are lost.
pub fn uninstall() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let uid = unsafe { libc::getuid() };
        let _ = run("launchctl", &["bootout", &format!("gui/{uid}/{LABEL}")]);
        if let Some(path) = launchd_plist_path() {
            match std::fs::remove_file(&path) {
                Ok(()) => println!("Removed {}", path.display()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    println!("No launchd agent was installed.")
                }
                Err(error) => return Err(format!("removing {}: {error}", path.display())),
            }
        }
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = run("systemctl", &["--user", "disable", "--now", SERVICE_NAME]);
        if let Some(path) = systemd_unit_path() {
            match std::fs::remove_file(&path) {
                Ok(()) => println!("Removed {}", path.display()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    println!("No systemd unit was installed.")
                }
                Err(error) => return Err(format!("removing {}: {error}", path.display())),
            }
        }
        run("systemctl", &["--user", "daemon-reload"])?;
        Ok(())
    }
    #[cfg(not(unix))]
    Err("fastty host: not supported on this platform yet".into())
}

/// Report whether the service is installed and whether a live host answers
/// on the daemon socket.
pub fn status() {
    #[cfg(target_os = "macos")]
    let installed = launchd_plist_path().is_some_and(|path| path.exists());
    #[cfg(all(unix, not(target_os = "macos")))]
    let installed = systemd_unit_path().is_some_and(|path| path.exists());
    #[cfg(not(unix))]
    let installed = false;

    println!(
        "Service: {}",
        if installed {
            "installed"
        } else {
            "not installed (run: fastty host install)"
        }
    );
    match crate::session_host::probe_daemon() {
        Some(info) => println!("Host: live — {info}"),
        None => println!("Host: no live host on the daemon socket"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_carries_the_host_flag_and_label() {
        let plist = plist_contents(std::path::Path::new("/usr/local/bin/fastty"));
        assert!(plist.contains(LABEL));
        assert!(plist.contains("--fastty-session-host"));
        assert!(plist.contains("<string>/usr/local/bin/fastty</string>"));
        assert!(plist.contains("<true/>"));
    }

    #[test]
    fn systemd_unit_carries_restart_policy() {
        let unit = systemd_unit_contents(std::path::Path::new("/usr/bin/fastty"));
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("--fastty-session-host"));
        assert!(unit.contains("WantedBy=default.target"));
    }
}
