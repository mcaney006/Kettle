//! `SUDO_ASKPASS` helper: hands sudo a password fetched from 1Password.
//!
//! Why this and not a NOPASSWD sudoers rule: Homebrew escalates `/bin/cp`,
//! `/bin/rm`, `chmod`, `mkdir` and `/usr/sbin/installer` when installing casks.
//! A NOPASSWD rule wide enough to cover those is unrestricted root, not a
//! scoped exemption. `sudo -A` keeps sudo's authorisation exactly as it was and
//! only automates typing, which is the part that was actually annoying.
//!
//! Homebrew opts into this itself -- system_command.rb adds `-A` whenever
//! SUDO_ASKPASS is set -- so nothing here patches or wraps brew.
//!
//! Contract with sudo: the password goes to stdout and NOTHING else ever does.
//! Any failure means exit non-zero with an empty stdout, and sudo falls back to
//! prompting normally. Failing closed is the whole safety story here.

use std::io::Write;
use std::process::{Command, Stdio};

/// A file holding one line: a 1Password secret reference for the login
/// password, e.g. `op://Private/macOS/password`.
///
/// The reference is stored, never the secret. If this file is absent, the
/// helper does nothing and sudo prompts as usual -- so the feature is opt-in
/// and removing the file turns it off completely.
fn reference_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".config/kettle/sudo-secret"))
}

fn main() {
    // Never let a panic message reach stdout, where sudo would read it as a
    // password.
    std::panic::set_hook(Box::new(|_| {}));

    let Some(path) = reference_path() else {
        fail("no HOME set");
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        fail("no ~/.config/kettle/sudo-secret; run sudo normally");
    };
    let reference = raw.trim();

    // Only ever pass a 1Password reference to `op read`. Without this check a
    // stray file could turn this into an arbitrary-argument runner.
    if !reference.starts_with("op://") || reference.contains(['\n', '\0']) {
        fail("sudo-secret must be a single op:// reference");
    }

    let out = Command::new("op")
        .args(["read", "--no-newline", reference])
        .stdin(Stdio::null())
        // Inherit stderr so 1Password's own "please unlock" notices are visible.
        .stderr(Stdio::inherit())
        .output();

    let Ok(out) = out else {
        fail("could not run `op` (is the 1Password CLI installed?)");
    };
    if !out.status.success() || out.stdout.is_empty() {
        fail("op could not read that reference");
    }

    // The only write to stdout in the program.
    let mut stdout = std::io::stdout();
    if stdout.write_all(&out.stdout).is_err() || stdout.flush().is_err() {
        std::process::exit(1);
    }
}

/// Exit without writing to stdout, so sudo falls back to its own prompt.
fn fail(msg: &str) -> ! {
    eprintln!("kettle-askpass: {msg}");
    std::process::exit(1);
}
