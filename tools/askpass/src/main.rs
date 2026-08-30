//! Fail-closed `SUDO_ASKPASS` helper backed by a 1Password secret reference.

use std::{
    fs::Metadata,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const OP_CANDIDATES: [&str; 2] = ["/opt/homebrew/bin/op", "/usr/local/bin/op"];

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    if let Err(message) = run() {
        fail(message);
    }
}

fn run() -> Result<(), &'static str> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    let owner = std::fs::metadata(&home)
        .map_err(|_| "HOME metadata is unavailable")?
        .uid();
    let path = home.join(".config/kettle/sudo-secret");
    validate_reference_file(&path, owner)?;

    let raw = std::fs::read_to_string(&path).map_err(|_| "sudo-secret is unreadable")?;
    let reference = raw.trim_end();
    validate_reference(reference)?;

    let op = resolve_op(owner)?;
    let output = Command::new(op)
        .args(["read", "--no-newline", reference])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|_| "could not execute 1Password CLI")?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err("1Password did not return a password");
    }

    let mut secret = output.stdout;
    let result = std::io::stdout()
        .write_all(&secret)
        .and_then(|()| std::io::stdout().flush());
    secret.fill(0);
    result.map_err(|_| "sudo closed the askpass pipe")
}

fn validate_reference(reference: &str) -> Result<(), &'static str> {
    if reference.len() > 1_024
        || reference.contains(char::is_control)
        || reference.trim() != reference
    {
        return Err("sudo-secret must contain one bounded reference");
    }
    let Some(components) = reference.strip_prefix("op://") else {
        return Err("sudo-secret must contain an op:// reference");
    };
    let component_count = components.split('/').count();
    if !(3..=4).contains(&component_count) || components.split('/').any(str::is_empty) {
        return Err("sudo-secret must identify a vault, item, and field");
    }
    Ok(())
}

fn validate_reference_file(path: &Path, owner: u32) -> Result<(), &'static str> {
    let parent = path.parent().ok_or("sudo-secret has no parent")?;
    let parent_metadata =
        std::fs::symlink_metadata(parent).map_err(|_| "sudo-secret directory is unavailable")?;
    if !parent_metadata.is_dir()
        || parent_metadata.uid() != owner
        || parent_metadata.permissions().mode() & 0o022 != 0
    {
        return Err("sudo-secret directory has unsafe ownership or permissions");
    }

    let metadata = std::fs::symlink_metadata(path).map_err(|_| "sudo-secret is unavailable")?;
    if !metadata.file_type().is_file()
        || metadata.uid() != owner
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err("sudo-secret has unsafe type, ownership, or permissions");
    }
    Ok(())
}

fn resolve_op(owner: u32) -> Result<PathBuf, &'static str> {
    OP_CANDIDATES
        .iter()
        .map(Path::new)
        .find_map(|candidate| {
            let canonical = candidate.canonicalize().ok()?;
            let metadata = std::fs::metadata(&canonical).ok()?;
            executable_is_safe(&canonical, &metadata, owner, Path::new("/")).then_some(canonical)
        })
        .ok_or("trusted 1Password CLI was not found")
}

fn executable_is_safe(path: &Path, metadata: &Metadata, owner: u32, root: &Path) -> bool {
    if !(metadata.is_file()
        && metadata.permissions().mode() & 0o111 != 0
        && metadata.permissions().mode() & 0o022 == 0
        && (metadata.uid() == owner || metadata.uid() == 0))
    {
        return false;
    }
    let mut parent = path.parent();
    while let Some(directory) = parent {
        if directory == root {
            break;
        }
        let Ok(metadata) = std::fs::symlink_metadata(directory) else {
            return false;
        };
        if !metadata.is_dir()
            || (metadata.uid() != owner && metadata.uid() != 0)
            || metadata.permissions().mode() & 0o022 != 0
        {
            return false;
        }
        parent = directory.parent();
    }
    true
}

fn fail(message: &str) -> ! {
    eprintln!("kettle-askpass: {message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_structured_single_line_references() {
        assert!(validate_reference("op://Private/macOS/password").is_ok());
        assert!(validate_reference("op://Private/macOS/login/password").is_ok());
        for invalid in [
            "password",
            "op://vault/item",
            "op://vault//field",
            "op://vault/item/field\nextra",
            " op://vault/item/field",
        ] {
            assert!(validate_reference(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn rejects_loose_reference_file_permissions() {
        let root = std::env::temp_dir().join(format!("kettle-askpass-{}", std::process::id()));
        let directory = root.join("kettle");
        let path = directory.join("sudo-secret");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&path, "op://Private/macOS/password").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let owner = std::fs::metadata(&root).unwrap().uid();
        assert!(validate_reference_file(&path, owner).is_ok());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(validate_reference_file(&path, owner).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn executable_requires_trusted_ownership_and_parent_permissions() {
        let root = std::env::temp_dir().join(format!("kettle-op-{}", std::process::id()));
        let directory = root.join("bin");
        let path = directory.join("op");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, "test").unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let owner = metadata.uid();
        assert!(executable_is_safe(&path, &metadata, owner, &root));

        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!executable_is_safe(
            &path,
            &std::fs::metadata(&path).unwrap(),
            owner,
            &root
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
