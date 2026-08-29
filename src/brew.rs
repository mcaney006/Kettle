//! Homebrew data layer.
//!
//! Reads are done straight off disk (Cellar/Caskroom dirs + Homebrew's own JSON
//! catalog cache) so the UI can paint immediately -- shelling out to `brew` costs
//! a portable-Ruby boot, which is where the multi-second stalls come from.
//!
//! Writes (install/upgrade) DO go through `brew`, because only brew may mutate the
//! prefix. Success is judged by **exit status only**. This is deliberate: AutoBrew
//! treated any stderr output as failure, and `brew update` writes "Already
//! up-to-date." to stderr on success, so every cycle aborted. Never key off stderr.

use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn detect_prefix() -> Option<PathBuf> {
    ["/opt/homebrew", "/usr/local"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.join("bin/brew").is_file())
}

#[derive(Clone, Debug, Default)]
pub struct Pkg {
    pub name: String,
    pub installed: String,
    pub latest: String,
    pub desc: String,
    pub cask: bool,
    pub outdated: bool,
    pub pinned: bool,
    /// ASCII-folded search buffers, precomputed once at build time. Folding 16k
    /// names+descs on every keystroke was the single biggest waste in the UI.
    pub name_lc: Vec<u8>,
    pub desc_lc: Vec<u8>,
}

impl Pkg {
    fn fold(&mut self) {
        self.name_lc = crate::rank::fold(&self.name);
        self.desc_lc = crate::rank::fold(&self.desc);
    }
}

pub fn fold_all(v: &mut [Pkg]) {
    for p in v.iter_mut() {
        p.fold();
    }
}

/// Newest installed version dir. Ordered with Homebrew's semantics, not
/// lexically -- a plain sort puts "10.0" before "9.0".
fn newest_child(dir: &PathBuf) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .max_by(|a, b| crate::rank::version_cmp(a, b))
}

/// Scan Cellar + Caskroom. Pure filesystem: milliseconds, no Ruby.
pub fn scan_installed(prefix: &PathBuf) -> Vec<Pkg> {
    let mut out = Vec::new();
    for (sub, cask) in [("Cellar", false), ("Caskroom", true)] {
        let root = prefix.join(sub);
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        for e in rd.flatten() {
            let Ok(name) = e.file_name().into_string() else { continue };
            if name.starts_with('.') {
                continue;
            }
            let p = root.join(&name);
            if !p.is_dir() {
                continue;
            }
            out.push(Pkg {
                installed: newest_child(&p).unwrap_or_default(),
                name,
                cask,
                ..Default::default()
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    fold_all(&mut out);
    out
}

// ---- catalog (browse/search) ------------------------------------------------

#[derive(Deserialize)]
struct FormulaEntry {
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    stable_version: Option<String>,
}

#[derive(Deserialize)]
struct CaskEntry {
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(default)]
    formulae: HashMap<String, FormulaEntry>,
    #[serde(default)]
    casks: HashMap<String, CaskEntry>,
}

fn catalog_payload_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var("HOME").ok()?)
        .join("Library/Caches/Homebrew/api/internal");
    std::fs::read_dir(&dir).ok()?.flatten().find_map(|e| {
        let n = e.file_name().into_string().ok()?;
        (n.starts_with("packages.") && n.ends_with(".jws.json.payload")).then(|| e.path())
    })
}

/// Homebrew's catalog cache is JSON Lines: line 1 is the JWS signature header,
/// line 2 is the payload. Skip line 1, parse line 2.
/// Every failure here used to return an empty Vec, which is indistinguishable
/// from "Homebrew has no packages": Browse silently showed nothing and all
/// descriptions vanished, with no way to tell breakage from an empty result.
/// The caller needs to be able to say what went wrong.
pub fn load_catalog() -> Result<Vec<Pkg>, String> {
    let path = catalog_payload_path()
        .ok_or("no catalog cache found; run `brew update` to fetch it")?;
    let f = std::fs::File::open(&path).map_err(|e| format!("could not open catalog: {e}"))?;
    let mut r = BufReader::with_capacity(1 << 20, f);
    let mut discard = String::new();
    r.read_line(&mut discard)
        .map_err(|e| format!("could not read catalog header: {e}"))?;
    let mut body = String::new();
    r.read_to_string(&mut body)
        .map_err(|e| format!("could not read catalog: {e}"))?;
    // brew rewrites this file in place on update, so a read landing mid-write
    // parses as garbage. Say so rather than showing an empty catalog.
    let p = serde_json::from_str::<Payload>(body.trim())
        .map_err(|e| format!("could not parse catalog (brew may be updating it): {e}"))?;

    let mut out: Vec<Pkg> = Vec::with_capacity(p.formulae.len() + p.casks.len());
    for (name, e) in p.formulae {
        out.push(Pkg {
            name,
            latest: e.stable_version.unwrap_or_default(),
            desc: e.desc.unwrap_or_default(),
            cask: false,
            ..Default::default()
        });
    }
    for (name, e) in p.casks {
        out.push(Pkg {
            name,
            latest: e.version.unwrap_or_default(),
            desc: e.desc.unwrap_or_default(),
            cask: true,
            ..Default::default()
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    fold_all(&mut out);
    Ok(out)
}

// ---- outdated ---------------------------------------------------------------

#[derive(Deserialize)]
struct OutdatedItem {
    name: String,
    #[serde(default)]
    installed_versions: Vec<String>,
    #[serde(default)]
    current_version: String,
    #[serde(default)]
    pinned: bool,
}

#[derive(Deserialize)]
struct OutdatedDoc {
    #[serde(default)]
    formulae: Vec<OutdatedItem>,
    #[serde(default)]
    casks: Vec<OutdatedItem>,
}

fn base_command(prefix: &PathBuf) -> Command {
    let mut c = Command::new(prefix.join("bin/brew"));
    // A GUI app inherits a bare launchd PATH; brew needs the usual tools plus its
    // own prefix on PATH for anything it shells out to.
    let path = format!(
        "{}/bin:{}/sbin:/usr/bin:/bin:/usr/sbin:/sbin",
        prefix.display(),
        prefix.display()
    );
    c.env("PATH", path);
    c.env("HOMEBREW_NO_AUTO_UPDATE", "1");
    c.env("HOMEBREW_NO_ENV_HINTS", "1");
    c.env("HOMEBREW_NO_ANALYTICS", "1");
    // NOT `HOMEBREW_COLOR=0` -- Homebrew tests whether this variable is *set*,
    // not what it says, so "0" force-enables color and the log fills with raw
    // escape sequences (`\x1b[32m==>\x1b[0m`) rendered as literal text.
    c.env("HOMEBREW_NO_COLOR", "1");
    if let Some(helper) = askpass_helper() {
        // Homebrew adds `sudo -A` on its own once this is set (system_command.rb),
        // so a cask that needs admin rights is answered from 1Password instead of
        // stopping the whole batch on a prompt behind the window.
        c.env("SUDO_ASKPASS", helper);
    }
    c.current_dir("/");
    c
}

/// `kettle-askpass`, which ships beside the main binary inside the app bundle.
///
/// Returns None when it is missing (a plain `cargo run`, say), and then sudo
/// prompts exactly as it always did -- the helper is an optimisation, never a
/// requirement.
fn askpass_helper() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join("kettle-askpass");
    p.is_file().then_some(p)
}

/// Authoritative outdated list. We deliberately reuse brew's own comparison logic
/// rather than reimplementing PkgVersion ordering (epochs/revisions/rc tags are
/// easy to get subtly wrong).
pub fn outdated(prefix: &PathBuf) -> Result<Vec<Pkg>, String> {
    let out = base_command(prefix)
        .args(["outdated", "--json=v2"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run brew: {e}"))?;

    // Exit status only. stderr is noise (deprecation warnings, progress).
    if !out.status.success() {
        return Err(format!(
            "brew outdated exited {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let doc: OutdatedDoc = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("could not parse brew outdated JSON: {e}"))?;

    let mut v = Vec::new();
    for (items, cask) in [(doc.formulae, false), (doc.casks, true)] {
        for it in items {
            v.push(Pkg {
                name: it.name,
                installed: it.installed_versions.last().cloned().unwrap_or_default(),
                latest: it.current_version,
                cask,
                outdated: true,
                pinned: it.pinned,
                ..Default::default()
            });
        }
    }
    v.sort_by(|a, b| a.name.cmp(&b.name));
    fold_all(&mut v);
    Ok(v)
}

/// Build the brew invocations for a set of (name, is_cask) targets.
///
/// Formulae and casks must be separate commands. A token can exist as both --
/// `ant` is a formula in homebrew/core AND an installed cask here -- and a bare
/// name resolves to the formula, so `brew upgrade ant` dies with "ant not
/// installed" and takes the whole batch down with it.
pub fn plan(verb: &str, items: Vec<(String, bool)>) -> Vec<Vec<String>> {
    let (formulae, casks): (Vec<_>, Vec<_>) =
        items.into_iter().partition(|(_, is_cask)| !*is_cask);
    let mut out = Vec::new();
    for (list, cask) in [(formulae, false), (casks, true)] {
        if list.is_empty() {
            continue;
        }
        let mut args = vec![verb.to_string()];
        if cask {
            args.push("--cask".to_string());
        }
        args.extend(list.into_iter().map(|(n, _)| n));
        out.push(args);
    }
    out
}

// ---- streaming command ------------------------------------------------------

/// Run a brew subcommand, streaming merged output line-by-line to `on_line`.
/// Returns Ok(()) on exit status 0, Err(summary) otherwise -- never based on stderr.
pub fn run_stream<F: FnMut(String)>(
    prefix: &PathBuf,
    args: &[String],
    mut on_line: F,
) -> Result<(), String> {
    let mut child = base_command(prefix)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run brew: {e}"))?;

    // Drain stderr on its own thread so a chatty child can never deadlock on a
    // full pipe buffer while we're blocked reading stdout.
    let err = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let etx = tx.clone();
    let ejoin = std::thread::spawn(move || {
        if let Some(e) = err {
            for l in BufReader::new(e).lines().map_while(Result::ok) {
                let _ = etx.send(l);
            }
        }
    });
    let out = child.stdout.take();
    let ojoin = std::thread::spawn(move || {
        if let Some(o) = out {
            for l in BufReader::new(o).lines().map_while(Result::ok) {
                let _ = tx.send(l);
            }
        }
        // tx dropped here; rx ends once the stderr clone drops too.
    });

    for line in rx {
        on_line(line);
    }
    let _ = ojoin.join();
    let _ = ejoin.join();

    let status = child.wait().map_err(|e| format!("brew wait failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("brew exited with status {}", status.code().unwrap_or(-1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casks_and_formulae_are_separate_invocations() {
        let plan = plan(
            "upgrade",
            vec![
                ("ripgrep".into(), false),
                ("ant".into(), true),
                ("bat".into(), false),
            ],
        );
        assert_eq!(plan.len(), 2, "expected one formula cmd and one cask cmd");
        assert_eq!(plan[0], vec!["upgrade", "ripgrep", "bat"]);
        assert_eq!(plan[1], vec!["upgrade", "--cask", "ant"]);
        // The bug this guards: a bare cask name resolving to a formula.
        assert!(!plan[0].contains(&"ant".to_string()));
    }

    #[test]
    fn plan_skips_empty_groups() {
        assert_eq!(plan("install", vec![]).len(), 0);
        let only_casks = plan("install", vec![("firefox".into(), true)]);
        assert_eq!(only_casks, vec![vec!["install", "--cask", "firefox"]]);
    }

    /// Guards the bug this app exists to fix: a command that writes to stderr and
    /// exits 0 must be reported as SUCCESS.
    #[test]
    fn stderr_output_with_zero_exit_is_success() {
        let dir = std::env::temp_dir().join("kettle_test_prefix");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let brew = bin.join("brew");
        std::fs::write(
            &brew,
            "#!/bin/sh\necho 'Already up-to-date.' >&2\nexit 0\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&brew, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut seen = Vec::new();
        let r = run_stream(&dir, &["update".to_string()], |l| seen.push(l));
        assert!(r.is_ok(), "stderr output must not be treated as failure: {r:?}");
        assert!(seen.iter().any(|l| l.contains("Already up-to-date")));

        // And a genuine non-zero exit must still be an error.
        std::fs::write(&brew, "#!/bin/sh\necho fine\nexit 3\n").unwrap();
        let r2 = run_stream(&dir, &["update".to_string()], |_| {});
        assert!(r2.is_err(), "non-zero exit must be an error");

        std::fs::remove_dir_all(&dir).ok();
    }
}
