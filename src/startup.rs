//! Native login-start integration for supported desktop platforms.

#[cfg(target_os = "linux")]
mod linux {
    use anyhow::{Context, Result, bail};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const SERVICE: &str = "waft.service";

    pub fn install(base_dir: &Path) -> Result<()> {
        let service_path = service_path()?;
        let parent = service_path
            .parent()
            .context("systemd user directory has no parent")?;
        std::fs::create_dir_all(parent).context("Failed to create systemd user directory")?;
        let executable = std::env::current_exe().context("Failed to locate waft executable")?;
        std::fs::write(&service_path, render_unit(&executable, base_dir))
            .context("Failed to write systemd user service")?;
        run_systemctl(["--user", "daemon-reload"])?;
        run_systemctl(["--user", "enable", "--now", SERVICE])?;
        println!("Installed and started {SERVICE} login service.");
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", SERVICE])
            .status();
        let service_path = service_path()?;
        if service_path.exists() {
            std::fs::remove_file(&service_path).context("Failed to remove systemd user service")?;
        }
        run_systemctl(["--user", "daemon-reload"])?;
        println!("Removed {SERVICE} login service.");
        Ok(())
    }

    fn service_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home)
            .join(".config")
            .join("systemd")
            .join("user")
            .join(SERVICE))
    }

    fn run_systemctl<const N: usize>(args: [&str; N]) -> Result<()> {
        let output = Command::new("systemctl")
            .args(args)
            .output()
            .context("Failed to run systemctl")?;
        if !output.status.success() {
            bail!(
                "systemctl failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn render_unit(executable: &Path, base_dir: &Path) -> String {
        format!(
            "[Unit]\nDescription=waft file transfer daemon\nAfter=network-online.target\n\n[Service]\nExecStart={} daemon --dir {}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
            escape_exec_arg(&executable.to_string_lossy()),
            escape_exec_arg(&base_dir.to_string_lossy()),
        )
    }

    fn escape_exec_arg(value: &str) -> String {
        value
            .replace('%', "%%")
            .replace('\\', "\\\\")
            .replace(' ', "\\x20")
            .replace('\t', "\\x09")
            .replace('\n', "\\x0a")
    }

    #[cfg(test)]
    mod tests {
        use super::render_unit;
        use std::path::Path;

        #[test]
        fn unit_contains_escaped_daemon_arguments() {
            let unit = render_unit(Path::new("/opt/Wa ft/waft"), Path::new("/tmp/a%b"));
            assert!(unit.contains("/opt/Wa\\x20ft/waft daemon --dir /tmp/a%%b"));
            assert!(unit.contains("Restart=on-failure"));
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use anyhow::{Context, Result, bail};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const LABEL: &str = "dev.waft";

    pub fn install(base_dir: &Path) -> Result<()> {
        let plist_path = plist_path()?;
        let parent = plist_path
            .parent()
            .context("LaunchAgents directory has no parent")?;
        std::fs::create_dir_all(parent).context("Failed to create LaunchAgents directory")?;
        let executable = std::env::current_exe().context("Failed to locate waft executable")?;
        let plist = render_plist(&executable, base_dir);
        std::fs::write(&plist_path, plist).context("Failed to write launchd plist")?;

        let domain = launchd_domain()?;
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, LABEL])
            .status();
        run_launchctl(["bootstrap", &domain, path_arg(&plist_path).as_str()])?;
        run_launchctl(["kickstart", "-k", &format!("{domain}/{LABEL}")])?;
        println!("Installed and started {LABEL} login agent.");
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        let domain = launchd_domain()?;
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, LABEL])
            .status();
        let plist_path = plist_path()?;
        if plist_path.exists() {
            std::fs::remove_file(&plist_path).context("Failed to remove launchd plist")?;
        }
        println!("Removed {LABEL} login agent.");
        Ok(())
    }

    fn plist_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }

    fn launchd_domain() -> Result<String> {
        let output = Command::new("id")
            .arg("-u")
            .output()
            .context("Failed to determine current user ID")?;
        if !output.status.success() {
            bail!("id -u failed");
        }
        let uid = String::from_utf8(output.stdout)
            .context("Current user ID was not valid UTF-8")?
            .trim()
            .to_owned();
        Ok(format!("gui/{uid}"))
    }

    fn path_arg(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn run_launchctl<const N: usize>(args: [&str; N]) -> Result<()> {
        let output = Command::new("launchctl")
            .args(args)
            .output()
            .context("Failed to run launchctl")?;
        if !output.status.success() {
            bail!(
                "launchctl failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn render_plist(executable: &Path, base_dir: &Path) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>daemon</string>
        <string>--dir</string>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
"#,
            xml_escape(LABEL),
            xml_escape(&path_arg(executable)),
            xml_escape(&path_arg(base_dir)),
        )
    }

    fn xml_escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    #[cfg(test)]
    mod tests {
        use super::render_plist;
        use std::path::Path;

        #[test]
        fn plist_contains_escaped_daemon_arguments() {
            let plist = render_plist(Path::new("/Applications/Wa&ft/waft"), Path::new("/tmp/a<b"));
            assert!(plist.contains("/Applications/Wa&amp;ft/waft"));
            assert!(plist.contains("/tmp/a&lt;b"));
            assert!(plist.contains("<string>daemon</string>"));
            assert!(plist.contains("<true/>"));
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::{install, uninstall};

#[cfg(target_os = "linux")]
pub use linux::{install, uninstall};

#[cfg(target_os = "windows")]
mod windows {
    use anyhow::{Context, Result, bail};
    use std::path::Path;
    use std::process::Command;

    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "waft";

    pub fn install(base_dir: &Path) -> Result<()> {
        let executable = std::env::current_exe().context("Failed to locate waft executable")?;
        let command_line = format!(
            "{} daemon --dir {}",
            quote_arg(&executable.to_string_lossy()),
            quote_arg(&base_dir.to_string_lossy()),
        );
        run_reg([
            "ADD",
            RUN_KEY,
            "/v",
            VALUE_NAME,
            "/t",
            "REG_SZ",
            "/d",
            command_line.as_str(),
            "/f",
        ])?;
        println!("Installed {VALUE_NAME} login startup entry.");
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        run_reg(["DELETE", RUN_KEY, "/v", VALUE_NAME, "/f"])?;
        println!("Removed {VALUE_NAME} login startup entry.");
        Ok(())
    }

    fn quote_arg(value: &str) -> String {
        format!("\"{}\"", value.replace('"', "\\\""))
    }

    fn run_reg<const N: usize>(args: [&str; N]) -> Result<()> {
        let output = Command::new("reg.exe")
            .args(args)
            .output()
            .context("Failed to run reg.exe")?;
        if !output.status.success() {
            bail!(
                "reg.exe failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::quote_arg;

        #[test]
        fn quotes_paths_for_startup_command() {
            assert_eq!(
                quote_arg(r"C:\Program Files\waft\waft.exe"),
                r#""C:\Program Files\waft\waft.exe""#
            );
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows::{install, uninstall};
