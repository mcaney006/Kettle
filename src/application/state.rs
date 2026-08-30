use super::{SelectionModel, View};
use crate::{
    domain::{BrewAction, Package, PackageId},
    search::SearchIndex,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshStage {
    InstalledState,
    Catalog,
    Outdated,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OperationState {
    #[default]
    Idle,
    Refreshing(RefreshStage),
    Mutating {
        action: BrewAction,
        targets: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevicePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHubUser(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthFailure {
    Denied(String),
    Expired,
    Network(String),
    Keychain(String),
    Protocol(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum AuthState {
    #[default]
    SignedOut,
    RequestingDeviceCode,
    AwaitingApproval(DevicePrompt),
    SignedIn(GitHubUser),
    Failed(AuthFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Trace,
    Info,
    Success,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogSource {
    Homebrew,
    Catalog,
    GitHub,
    Application,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogEvent {
    pub level: LogLevel,
    pub source: LogSource,
    pub message: String,
}

#[derive(Default)]
pub struct PackageStore {
    packages: HashMap<PackageId, Package>,
    installed: Arc<[PackageId]>,
    catalog: Arc<[PackageId]>,
    outdated: Arc<[PackageId]>,
    search: SearchIndex,
}

impl PackageStore {
    pub fn preview_installed(&mut self, installed: &[Package]) {
        self.installed = sorted_ids(installed);
        for package in installed {
            self.packages
                .entry(package.id().clone())
                .and_modify(|current| current.merge_installed(package))
                .or_insert_with(|| package.clone());
        }
        self.search.rebuild(self.packages.values());
    }

    pub fn preview_catalog(&mut self, catalog: &[Package]) {
        self.catalog = sorted_ids(catalog);
        for package in catalog {
            self.packages
                .entry(package.id().clone())
                .and_modify(|current| current.merge_catalog(package))
                .or_insert_with(|| package.clone());
        }
        self.search.rebuild(self.packages.values());
    }

    pub fn preview_outdated(&mut self, outdated: &[Package]) {
        self.outdated = sorted_ids(outdated);
        for package in outdated {
            self.packages
                .entry(package.id().clone())
                .and_modify(|current| current.merge_outdated(package))
                .or_insert_with(|| package.clone());
        }
        self.search.rebuild(self.packages.values());
    }

    pub fn replace(
        &mut self,
        installed: Vec<Package>,
        catalog: Vec<Package>,
        outdated: Vec<Package>,
    ) {
        self.packages.clear();
        self.catalog = sorted_ids(&catalog);
        for package in catalog {
            self.packages.insert(package.id().clone(), package);
        }

        self.installed = sorted_ids(&installed);
        for package in installed {
            self.packages
                .entry(package.id().clone())
                .and_modify(|current| current.merge_installed(&package))
                .or_insert(package);
        }

        self.outdated = sorted_ids(&outdated);
        for package in outdated {
            self.packages
                .entry(package.id().clone())
                .and_modify(|current| current.merge_outdated(&package))
                .or_insert(package);
        }
        self.search.rebuild(self.packages.values());
    }

    pub fn package(&self, id: &PackageId) -> Option<&Package> {
        self.packages.get(id)
    }

    pub fn ids(&self, view: View) -> &[PackageId] {
        match view {
            View::Outdated => &self.outdated,
            View::Installed => &self.installed,
            View::Browse => &self.catalog,
            View::Settings => &[],
        }
    }

    pub fn filtered(&self, view: View, query: &str) -> Arc<[PackageId]> {
        let candidates = match view {
            View::Outdated => &self.outdated,
            View::Installed => &self.installed,
            View::Browse => &self.catalog,
            View::Settings => return Arc::default(),
        };
        self.search.rank(query, candidates)
    }
}

fn sorted_ids(packages: &[Package]) -> Arc<[PackageId]> {
    let mut ids: Vec<_> = packages
        .iter()
        .map(|package| package.id().clone())
        .collect();
    ids.sort();
    ids.into()
}

#[derive(Default)]
pub struct AppState {
    pub view: View,
    pub operation: OperationState,
    pub auth: AuthState,
    pub packages: PackageStore,
    pub selection: SelectionModel,
    pub query: String,
    logs: VecDeque<LogEvent>,
}

impl AppState {
    pub const MAX_LOG_EVENTS: usize = 2_000;

    pub fn refilter(&mut self) {
        self.selection
            .set_visible(self.packages.filtered(self.view, &self.query));
    }

    pub fn set_view(&mut self, view: View) {
        self.view = view;
        self.selection.clear();
        self.refilter();
    }

    pub fn push_log(&mut self, event: LogEvent) {
        if self.logs.len() == Self::MAX_LOG_EVENTS {
            self.logs.pop_front();
        }
        self.logs.push_back(event);
    }

    pub fn logs(&self) -> &VecDeque<LogEvent> {
        &self.logs
    }

    pub fn selected_targets(&self) -> Vec<PackageId> {
        let mut targets: Vec<_> = self
            .selection
            .selected()
            .iter()
            .filter(|id| {
                self.packages
                    .package(id)
                    .is_some_and(|package| !package.is_pinned())
            })
            .cloned()
            .collect();
        targets.sort();
        targets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PackageKind, UpdateState, Version};

    fn id(kind: PackageKind) -> PackageId {
        PackageId::new("shared", kind).unwrap()
    }

    #[test]
    fn namespace_is_preserved_across_every_overlay() {
        let formula = id(PackageKind::Formula);
        let cask = id(PackageKind::Cask);
        let mut store = PackageStore::default();
        store.replace(
            vec![Package::installed(formula.clone(), Version::new("1"), true)],
            vec![
                Package::catalog(
                    formula.clone(),
                    Version::new("2"),
                    Some("formula description"),
                ),
                Package::catalog(cask.clone(), Version::new("9"), Some("cask description")),
            ],
            vec![Package::outdated(
                cask.clone(),
                Version::new("8"),
                Version::new("9"),
                false,
            )],
        );

        let formula_package = store.package(&formula).unwrap();
        let cask_package = store.package(&cask).unwrap();
        assert!(formula_package.is_pinned());
        assert_eq!(formula_package.update_state(), UpdateState::Unknown);
        assert_eq!(formula_package.description(), Some("formula description"));
        assert!(!cask_package.is_pinned());
        assert_eq!(cask_package.update_state(), UpdateState::UpdateAvailable);
        assert_eq!(cask_package.description(), Some("cask description"));
        assert_eq!(store.ids(View::Outdated), &[cask]);
        assert_eq!(store.ids(View::Installed), &[formula]);
    }

    #[test]
    fn log_retention_is_a_real_ring_buffer() {
        let mut state = AppState::default();
        for index in 0..AppState::MAX_LOG_EVENTS + 5 {
            state.push_log(LogEvent {
                level: LogLevel::Info,
                source: LogSource::Application,
                message: index.to_string(),
            });
        }
        assert_eq!(state.logs().len(), AppState::MAX_LOG_EVENTS);
        assert_eq!(state.logs().front().unwrap().message, "5");
    }
}
