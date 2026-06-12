use std::process::Command;

fn main() {
    // Only configure git hooks locally if we are not in a CI environment and a .git directory exists
    if std::env::var("CI").is_err() && std::path::Path::new(".git").exists() {
        let status = Command::new("git")
            .args(["config", "--local", "core.hooksPath", ".githooks"])
            .status();

        if let Ok(s) = status
            && s.success()
        {
            println!("cargo:rerun-if-changed=.githooks");
        }
    }
}
