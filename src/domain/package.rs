use super::Version;
use std::{fmt, sync::Arc};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PackageKind {
    Formula,
    Cask,
}

impl PackageKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Formula => "formula",
            Self::Cask => "cask",
        }
    }
}

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageName(Arc<str>);

impl PackageName {
    pub fn new(value: impl AsRef<str>) -> Option<Self> {
        let value = value.as_ref().trim();
        (!value.is_empty()).then(|| Self(Arc::from(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn shared(&self) -> Arc<str> {
        self.0.clone()
    }
}

impl fmt::Debug for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PackageName").field(&self.0).finish()
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageId {
    name: PackageName,
    kind: PackageKind,
}

impl PackageId {
    pub fn new(name: impl AsRef<str>, kind: PackageKind) -> Option<Self> {
        Some(Self {
            name: PackageName::new(name)?,
            kind,
        })
    }

    pub fn name(&self) -> &PackageName {
        &self.name
    }

    pub const fn kind(&self) -> PackageKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UpdateState {
    #[default]
    Unknown,
    Current,
    UpdateAvailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Package {
    id: PackageId,
    installed: Option<Version>,
    latest: Option<Version>,
    description: Option<Arc<str>>,
    update: UpdateState,
    pinned: bool,
}

impl Package {
    pub fn catalog(
        id: PackageId,
        latest: Option<Version>,
        description: Option<impl AsRef<str>>,
    ) -> Self {
        Self {
            id,
            installed: None,
            latest,
            description: description
                .map(|value| value.as_ref().trim().to_owned())
                .filter(|value| !value.is_empty())
                .map(Arc::from),
            update: UpdateState::Unknown,
            pinned: false,
        }
    }

    pub fn installed(id: PackageId, installed: Option<Version>, pinned: bool) -> Self {
        Self {
            id,
            installed,
            latest: None,
            description: None,
            update: UpdateState::Unknown,
            pinned,
        }
    }

    pub fn outdated(
        id: PackageId,
        installed: Option<Version>,
        latest: Option<Version>,
        pinned: bool,
    ) -> Self {
        Self {
            id,
            installed,
            latest,
            description: None,
            update: UpdateState::UpdateAvailable,
            pinned,
        }
    }

    pub fn id(&self) -> &PackageId {
        &self.id
    }

    pub fn installed_version(&self) -> Option<&Version> {
        self.installed.as_ref()
    }

    pub fn latest_version(&self) -> Option<&Version> {
        self.latest.as_ref()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub(crate) fn shared_description(&self) -> Option<Arc<str>> {
        self.description.clone()
    }

    pub const fn update_state(&self) -> UpdateState {
        self.update
    }

    pub const fn is_pinned(&self) -> bool {
        self.pinned
    }

    pub fn is_installed(&self) -> bool {
        self.installed.is_some()
    }

    pub(crate) fn merge_catalog(&mut self, other: &Self) {
        debug_assert_eq!(self.id, other.id);
        self.latest.clone_from(&other.latest);
        self.description = other.description.clone();
        self.reconcile_versions();
    }

    pub(crate) fn merge_installed(&mut self, other: &Self) {
        debug_assert_eq!(self.id, other.id);
        self.installed.clone_from(&other.installed);
        self.pinned = other.pinned;
        self.reconcile_versions();
    }

    pub(crate) fn merge_outdated(&mut self, other: &Self) {
        debug_assert_eq!(self.id, other.id);
        self.installed.clone_from(&other.installed);
        self.latest.clone_from(&other.latest);
        self.pinned = other.pinned;
        self.update = UpdateState::UpdateAvailable;
    }

    fn reconcile_versions(&mut self) {
        self.update = match (&self.installed, &self.latest) {
            (Some(installed), Some(latest)) if installed == latest => UpdateState::Current,
            _ => UpdateState::Unknown,
        };
    }
}
