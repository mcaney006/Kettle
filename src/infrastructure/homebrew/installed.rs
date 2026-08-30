use super::super::InfrastructureError;
use crate::domain::{Package, PackageId, PackageKind, Version};
use std::{collections::HashSet, io, path::Path};

pub(crate) fn discover(prefix: &Path) -> Result<Vec<Package>, InfrastructureError> {
    let pinned = pinned_formulae(prefix)?;
    let mut packages = Vec::new();
    for (directory, kind) in [
        ("Cellar", PackageKind::Formula),
        ("Caskroom", PackageKind::Cask),
    ] {
        let root = prefix.join(directory);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => return Err(InfrastructureError::filesystem(root, source)),
        };
        for entry in entries {
            let entry = entry.map_err(|source| InfrastructureError::filesystem(&root, source))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || !entry.path().is_dir() {
                continue;
            }
            let Some(id) = PackageId::new(name, kind) else {
                continue;
            };
            let version = newest_child(&entry.path())?;
            let is_pinned = kind == PackageKind::Formula && pinned.contains(name);
            packages.push(Package::installed(id, version, is_pinned));
        }
    }
    packages.sort_by(|left, right| left.id().cmp(right.id()));
    Ok(packages)
}

fn pinned_formulae(prefix: &Path) -> Result<HashSet<String>, InfrastructureError> {
    let root = prefix.join("var/homebrew/pinned");
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(source) => return Err(InfrastructureError::filesystem(root, source)),
    };
    let mut pinned = HashSet::new();
    for entry in entries {
        let entry = entry.map_err(|source| InfrastructureError::filesystem(&root, source))?;
        if let Some(name) = entry.file_name().to_str() {
            pinned.insert(name.to_owned());
        }
    }
    Ok(pinned)
}

fn newest_child(directory: &Path) -> Result<Option<Version>, InfrastructureError> {
    let entries = std::fs::read_dir(directory)
        .map_err(|source| InfrastructureError::filesystem(directory, source))?;
    let mut newest = None;
    for entry in entries {
        let entry = entry.map_err(|source| InfrastructureError::filesystem(directory, source))?;
        if !entry
            .file_type()
            .map_err(|source| InfrastructureError::filesystem(entry.path(), source))?
            .is_dir()
        {
            continue;
        }
        let Some(name) = entry.file_name().to_str().and_then(Version::new) else {
            continue;
        };
        if newest.as_ref().is_none_or(|current| name > *current) {
            newest = Some(name);
        }
    }
    Ok(newest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_is_isolated_and_pins_only_formulae() {
        let temp = tempfile::tempdir().unwrap();
        for path in [
            "Cellar/shared/1.9",
            "Cellar/shared/1.10",
            "Caskroom/shared/99",
            "var/homebrew/pinned/shared",
        ] {
            std::fs::create_dir_all(temp.path().join(path)).unwrap();
        }
        let packages = discover(temp.path()).unwrap();
        let formula = packages
            .iter()
            .find(|package| package.id().kind() == PackageKind::Formula)
            .unwrap();
        let cask = packages
            .iter()
            .find(|package| package.id().kind() == PackageKind::Cask)
            .unwrap();
        assert_eq!(formula.installed_version().unwrap().as_str(), "1.10");
        assert!(formula.is_pinned());
        assert!(!cask.is_pinned());
    }
}
