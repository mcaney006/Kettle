//! GitHub sign-in via OAuth Device Flow.
//!
//! Device Flow is the right grant for a desktop app: there is no client secret,
//! because an app shipped to users cannot keep one. The client id below is
//! public by design.
//!
//! Two things sign-in buys:
//!   - Authenticated GitHub calls are rate limited at 5000/hour instead of 60.
//!   - `read:user` is enough to see which languages you actually work in, which
//!     is what drives recommendations.
//!
//! We deliberately do NOT request `repo`. That scope is read *and write* access
//! to every private repository you have, which is an absurd price for guessing
//! which CLI tools you might like.
//!
//! HTTPS goes through `/usr/bin/curl` rather than a TLS crate: it ships with
//! macOS, it is C, and it keeps a password-adjacent code path free of a large
//! dependency tree.

use serde::Deserialize;
use std::process::{Command, Stdio};

pub const CLIENT_ID: &str = "Ov23liiLF3n6sxqOh6O9";
const SCOPE: &str = "read:user";

const KEYCHAIN_SERVICE: &str = "com.kettle.app.github";
const KEYCHAIN_ACCOUNT: &str = "oauth-token";

#[derive(Debug, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    /// The short code the user types into the browser, e.g. "ABCD-1234".
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds GitHub requires us to wait between polls. Polling faster earns a
    /// `slow_down` error, so this is a floor, not a suggestion.
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

/// POST a form and return the body. `Accept: application/json` matters --
/// without it GitHub answers OAuth endpoints in form-urlencoded.
fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<String, String> {
    let mut cmd = Command::new("/usr/bin/curl");
    cmd.args(["-sS", "--fail-with-body", "-X", "POST", "-H", "Accept: application/json"]);
    for (k, v) in fields {
        cmd.arg("--data-urlencode").arg(format!("{k}={v}"));
    }
    cmd.arg(url);
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "network request failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Step 1: ask GitHub for a device code and the code the user must type.
pub fn request_device_code() -> Result<DeviceCode, String> {
    let body = post_form(
        "https://github.com/login/device/code",
        &[("client_id", CLIENT_ID), ("scope", SCOPE)],
    )?;
    // A disabled Device Flow fails here, and the message is otherwise opaque.
    if body.contains("device_flow_disabled") {
        return Err("Device Flow is not enabled on the GitHub OAuth app".into());
    }
    serde_json::from_str(&body).map_err(|e| format!("unexpected reply from GitHub: {e}"))
}

/// Outcome of one poll. Separated from an error so the caller can keep waiting
/// without treating "not yet" as a failure.
pub enum Poll {
    Pending,
    /// GitHub asked us to back off; the new interval is in seconds.
    SlowDown(u64),
    Token(String),
}

/// Step 2: exchange the device code for a token, once the user has approved.
pub fn poll_once(device_code: &str) -> Result<Poll, String> {
    let body = post_form(
        "https://github.com/login/oauth/access_token",
        &[
            ("client_id", CLIENT_ID),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ],
    )?;
    let r: TokenResponse =
        serde_json::from_str(&body).map_err(|e| format!("unexpected reply from GitHub: {e}"))?;
    if let Some(t) = r.access_token {
        return Ok(Poll::Token(t));
    }
    match r.error.as_deref() {
        // The user simply has not finished in the browser yet.
        Some("authorization_pending") => Ok(Poll::Pending),
        Some("slow_down") => Ok(Poll::SlowDown(5)),
        Some("expired_token") => Err("the code expired; start again".into()),
        Some("access_denied") => Err("sign-in was declined".into()),
        Some(e) => Err(e.replace('_', " ")),
        None => Err("GitHub returned neither a token nor an error".into()),
    }
}

/// Who the stored token belongs to. Doubles as a validity check: a revoked
/// token fails here, which is how we notice we are no longer signed in.
pub fn whoami(token: &str) -> Result<String, String> {
    let out = Command::new("/usr/bin/curl")
        .args([
            "-sS",
            "--fail-with-body",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
        ])
        .arg(format!("Authorization: Bearer {token}"))
        .arg("https://api.github.com/user")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        return Err("token rejected by GitHub".into());
    }
    let u: User = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("unexpected reply from GitHub: {e}"))?;
    Ok(u.login)
}

pub fn open_in_browser(url: &str) {
    let _ = Command::new("/usr/bin/open").arg(url).spawn();
}

// ---- keychain ---------------------------------------------------------------
//
// ponytail: the token is passed to `security` as an argv, so it is briefly
// visible to `ps` for other processes of the same user. `security
// add-generic-password` has no way to take a secret on stdin, so avoiding this
// means calling SecItemAdd through the Security framework directly. That is the
// upgrade path if this ever holds anything more valuable than a read:user token.

pub fn store_token(token: &str) -> Result<(), String> {
    let st = Command::new("/usr/bin/security")
        .args(["add-generic-password", "-U", "-a", KEYCHAIN_ACCOUNT, "-s", KEYCHAIN_SERVICE, "-w"])
        .arg(token)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("could not run security: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err("could not save the token to your keychain".into())
    }
}

pub fn load_token() -> Option<String> {
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-a", KEYCHAIN_ACCOUNT, "-s", KEYCHAIN_SERVICE, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!t.is_empty()).then_some(t)
}

pub fn delete_token() {
    let _ = Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-a", KEYCHAIN_ACCOUNT, "-s", KEYCHAIN_SERVICE])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scope is the security-relevant constant in this file. `repo` would
    /// hand a Homebrew client write access to every private repository.
    #[test]
    fn scope_stays_minimal() {
        assert_eq!(SCOPE, "read:user");
        assert!(!SCOPE.contains("repo"), "must never request repo scope");
        assert!(!SCOPE.contains("write"), "must never request write scope");
    }

    /// Device Flow has no client secret; one appearing here would mean someone
    /// wired up the wrong grant type, and would put a real secret in the repo.
    /// Scoped to the code above this module, or it matches its own assertion.
    #[test]
    fn no_client_secret_in_the_code() {
        let src = include_str!("github.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("client_secret"), "device flow uses no client secret");
        // A GitHub OAuth secret is 40 hex characters; catch one pasted inline.
        assert!(
            !code
                .split(|c: char| !c.is_ascii_hexdigit())
                .any(|w| w.len() == 40),
            "a 40-char hex string in this file is almost certainly a leaked secret"
        );
    }
}
