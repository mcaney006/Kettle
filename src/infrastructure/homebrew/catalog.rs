use super::super::InfrastructureError;
use crate::domain::{Package, PackageId, PackageKind, Version};
use serde::Deserialize;
use std::{
    collections::HashMap,
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime},
};

const FORMULA_API: &str = "https://formulae.brew.sh/api/formula.json";
const CASK_API: &str = "https://formulae.brew.sh/api/cask.json";

pub trait CatalogProvider: Send + Sync {
    fn load(&self) -> Result<Vec<Package>, InfrastructureError>;
}

pub struct ApiCatalogProvider {
    client: reqwest::blocking::Client,
}

impl ApiCatalogProvider {
    pub fn new() -> Result<Self, InfrastructureError> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .user_agent(concat!("Kettle/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(network_error)?;
        Ok(Self { client })
    }

    fn get(&self, url: &str) -> Result<String, InfrastructureError> {
        let response = self.client.get(url).send().map_err(network_error)?;
        if !response.status().is_success() {
            return Err(InfrastructureError::CatalogUnavailable {
                reason: format!("Homebrew API returned HTTP {}", response.status()),
            });
        }
        response.text().map_err(network_error)
    }
}

impl CatalogProvider for ApiCatalogProvider {
    fn load(&self) -> Result<Vec<Package>, InfrastructureError> {
        let formulae = self.get(FORMULA_API)?;
        let casks = self.get(CASK_API)?;
        parse_api_catalog(&formulae, &casks)
    }
}

fn network_error(error: reqwest::Error) -> InfrastructureError {
    if error.is_timeout() {
        InfrastructureError::NetworkTimeout
    } else {
        InfrastructureError::NetworkTransport(error)
    }
}

pub struct CacheCatalogProvider {
    cache_directory: PathBuf,
}

impl CacheCatalogProvider {
    pub fn new(cache_directory: impl Into<PathBuf>) -> Self {
        Self {
            cache_directory: cache_directory.into(),
        }
    }

    pub fn standard() -> Result<Self, InfrastructureError> {
        let home =
            std::env::var_os("HOME").ok_or_else(|| InfrastructureError::CatalogUnavailable {
                reason: "HOME is not set".to_owned(),
            })?;
        Ok(Self::new(
            PathBuf::from(home).join("Library/Caches/Homebrew/api/internal"),
        ))
    }

    fn payload_path(&self) -> Result<PathBuf, InfrastructureError> {
        let entries = std::fs::read_dir(&self.cache_directory).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                InfrastructureError::CatalogUnavailable {
                    reason: format!("{} does not exist", self.cache_directory.display()),
                }
            } else {
                InfrastructureError::filesystem(&self.cache_directory, source)
            }
        })?;
        entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_name().to_str().is_some_and(|name| {
                    name.starts_with("packages.") && name.ends_with(".jws.json.payload")
                })
            })
            .max_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).ok())
            .map(|entry| entry.path())
            .ok_or_else(|| InfrastructureError::CatalogUnavailable {
                reason: "Homebrew's package cache is missing".to_owned(),
            })
    }
}

impl CatalogProvider for CacheCatalogProvider {
    fn load(&self) -> Result<Vec<Package>, InfrastructureError> {
        let path = self.payload_path()?;
        let mut last_parse_error = None;
        for attempt in 0..3 {
            let bytes = read_stable(&path)?;
            let body = bytes
                .splitn(2, |byte| *byte == b'\n')
                .nth(1)
                .unwrap_or_default();
            match serde_json::from_slice::<CachePayload>(body) {
                Ok(payload) if !payload.formulae.is_empty() || !payload.casks.is_empty() => {
                    return Ok(payload.into_packages());
                }
                Ok(_) => {
                    return Err(InfrastructureError::CatalogUnavailable {
                        reason: "Homebrew's package cache contains no formulae or casks".to_owned(),
                    });
                }
                Err(error) => last_parse_error = Some(error),
            }
            if attempt < 2 {
                thread::sleep(Duration::from_millis(25));
            }
        }
        Err(InfrastructureError::CatalogMalformed {
            path,
            source: last_parse_error.expect("three parse attempts always record an error"),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileStamp {
    device: u64,
    inode: u64,
    len: u64,
    modified: Option<SystemTime>,
}

fn stamp(path: &Path) -> Result<FileStamp, InfrastructureError> {
    let metadata =
        std::fs::metadata(path).map_err(|source| InfrastructureError::filesystem(path, source))?;
    Ok(FileStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn read_stable(path: &Path) -> Result<Vec<u8>, InfrastructureError> {
    read_stable_with(|| {
        let before = stamp(path)?;
        let bytes =
            std::fs::read(path).map_err(|source| InfrastructureError::filesystem(path, source))?;
        let after = stamp(path)?;
        Ok((before, bytes, after))
    })
}

fn read_stable_with(
    mut read: impl FnMut() -> Result<(FileStamp, Vec<u8>, FileStamp), InfrastructureError>,
) -> Result<Vec<u8>, InfrastructureError> {
    for attempt in 0..3 {
        let (before, bytes, after) = read()?;
        if before == after && bytes.len() as u64 == after.len {
            return Ok(bytes);
        }
        if attempt < 2 {
            thread::sleep(Duration::from_millis(10));
        }
    }
    Err(InfrastructureError::CatalogUnavailable {
        reason: "Homebrew replaced its package cache while Kettle was reading it".to_owned(),
    })
}

#[derive(Deserialize)]
struct CacheFormula {
    desc: Option<String>,
    stable_version: Option<String>,
}

#[derive(Deserialize)]
struct CacheCask {
    desc: Option<String>,
    version: Option<String>,
}

#[derive(Deserialize)]
struct CachePayload {
    #[serde(default)]
    formulae: HashMap<String, CacheFormula>,
    #[serde(default)]
    casks: HashMap<String, CacheCask>,
}

#[derive(Deserialize)]
struct ApiVersions {
    stable: Option<String>,
}

#[derive(Deserialize)]
struct ApiFormula {
    name: String,
    desc: Option<String>,
    versions: ApiVersions,
}

#[derive(Deserialize)]
struct ApiCask {
    token: String,
    desc: Option<String>,
    version: Option<String>,
}

fn parse_api_catalog(formulae: &str, casks: &str) -> Result<Vec<Package>, InfrastructureError> {
    let formulae: Vec<ApiFormula> =
        serde_json::from_str(formulae).map_err(|source| InfrastructureError::Json {
            context: "Homebrew formula API",
            source,
        })?;
    let casks: Vec<ApiCask> =
        serde_json::from_str(casks).map_err(|source| InfrastructureError::Json {
            context: "Homebrew cask API",
            source,
        })?;
    if formulae.is_empty() || casks.is_empty() {
        return Err(InfrastructureError::CatalogUnavailable {
            reason: "Homebrew API returned an incomplete catalog".to_owned(),
        });
    }
    let mut packages = Vec::with_capacity(formulae.len() + casks.len());
    packages.extend(formulae.into_iter().filter_map(|formula| {
        Some(Package::catalog(
            PackageId::new(formula.name, PackageKind::Formula)?,
            formula.versions.stable.as_deref().and_then(Version::new),
            formula.desc,
        ))
    }));
    packages.extend(casks.into_iter().filter_map(|cask| {
        Some(Package::catalog(
            PackageId::new(cask.token, PackageKind::Cask)?,
            cask.version.as_deref().and_then(Version::new),
            cask.desc,
        ))
    }));
    packages.sort_by(|left, right| left.id().cmp(right.id()));
    Ok(packages)
}

impl CachePayload {
    fn into_packages(self) -> Vec<Package> {
        let mut packages = Vec::with_capacity(self.formulae.len() + self.casks.len());
        packages.extend(self.formulae.into_iter().filter_map(|(name, formula)| {
            Some(Package::catalog(
                PackageId::new(name, PackageKind::Formula)?,
                formula.stable_version.as_deref().and_then(Version::new),
                formula.desc,
            ))
        }));
        packages.extend(self.casks.into_iter().filter_map(|(name, cask)| {
            Some(Package::catalog(
                PackageId::new(name, PackageKind::Cask)?,
                cask.version.as_deref().and_then(Version::new),
                cask.desc,
            ))
        }));
        packages.sort_by(|left, right| left.id().cmp(right.id()));
        packages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(body: &str) -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("packages.test.jws.json.payload"),
            format!("signature\n{body}"),
        )
        .unwrap();
        temp
    }

    #[test]
    fn malformed_cache_is_an_error_not_an_empty_catalog() {
        let temp = cache("not json");
        let error = CacheCatalogProvider::new(temp.path()).load().unwrap_err();
        assert!(matches!(
            error,
            InfrastructureError::CatalogMalformed { .. }
        ));
    }

    #[test]
    fn formula_and_cask_descriptions_do_not_cross_join() {
        let temp = cache(
            r#"{"formulae":{"shared":{"desc":"formula","stable_version":"1"}},"casks":{"shared":{"desc":"cask","version":"2"}}}"#,
        );
        let packages = CacheCatalogProvider::new(temp.path()).load().unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].description(), Some("formula"));
        assert_eq!(packages[1].description(), Some("cask"));
    }

    #[test]
    fn replacement_race_retries_a_bounded_number_of_times() {
        let old = FileStamp {
            device: 1,
            inode: 1,
            len: 3,
            modified: None,
        };
        let new = FileStamp {
            device: 1,
            inode: 2,
            len: 4,
            modified: None,
        };
        let mut calls = 0;
        let bytes = read_stable_with(|| {
            calls += 1;
            if calls == 1 {
                Ok((old, b"old".to_vec(), new))
            } else {
                Ok((new, b"good".to_vec(), new))
            }
        })
        .unwrap();
        assert_eq!(bytes, b"good");
        assert_eq!(calls, 2);
    }

    #[test]
    fn supported_api_parser_requires_both_namespaces() {
        let formulae = r#"[{"name":"shared","desc":"formula","versions":{"stable":"1"}}]"#;
        let casks = r#"[{"token":"shared","desc":"cask","version":"2"}]"#;
        let packages = parse_api_catalog(formulae, casks).unwrap();
        assert_eq!(packages.len(), 2);
        assert_ne!(packages[0].id(), packages[1].id());
        assert!(parse_api_catalog(formulae, "[]").is_err());
        assert!(parse_api_catalog("not json", casks).is_err());
    }
}
