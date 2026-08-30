use super::{
    catalog::{ApiCatalogProvider, CacheCatalogProvider, CatalogProvider},
    installed,
    process::{self, CommandSpec, ProcessEvent},
};
use crate::{
    domain::{BrewAction, Package, PackageId, PackageKind, Version},
    infrastructure::{InfrastructureError, privilege::validated_askpass_helper},
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPlan {
    pub action: BrewAction,
    pub kind: PackageKind,
    pub targets: Vec<PackageId>,
}

impl CommandPlan {
    pub fn arguments(&self) -> Vec<OsString> {
        let mut arguments = vec![OsString::from(self.action.command())];
        if self.kind == PackageKind::Cask {
            arguments.push(OsString::from("--cask"));
        }
        arguments.extend(
            self.targets
                .iter()
                .map(|id| OsString::from(id.name().as_str())),
        );
        arguments
    }
}

pub fn plan_commands(
    action: BrewAction,
    targets: impl IntoIterator<Item = PackageId>,
) -> Vec<CommandPlan> {
    let (formulae, casks): (Vec<_>, Vec<_>) = targets
        .into_iter()
        .partition(|id| id.kind() == PackageKind::Formula);
    [(PackageKind::Formula, formulae), (PackageKind::Cask, casks)]
        .into_iter()
        .filter_map(|(kind, mut targets)| {
            if targets.is_empty() {
                None
            } else {
                targets.sort();
                Some(CommandPlan {
                    action,
                    kind,
                    targets,
                })
            }
        })
        .collect()
}

pub trait HomebrewBackend: Send + Sync {
    fn prefix(&self) -> &Path;
    fn installed(&self) -> Result<Vec<Package>, InfrastructureError>;
    fn catalog(&self, cancelled: &dyn Fn() -> bool) -> Result<Vec<Package>, InfrastructureError>;
    fn outdated(&self, cancelled: &dyn Fn() -> bool) -> Result<Vec<Package>, InfrastructureError>;
    fn execute(
        &self,
        plan: &CommandPlan,
        cancelled: &dyn Fn() -> bool,
        on_event: &mut dyn FnMut(ProcessEvent),
    ) -> Result<(), InfrastructureError>;
}

pub fn execute_plans(
    backend: &dyn HomebrewBackend,
    plans: &[CommandPlan],
    cancelled: &dyn Fn() -> bool,
    mut on_plan: impl FnMut(&CommandPlan),
    on_event: &mut dyn FnMut(ProcessEvent),
) -> Result<(), InfrastructureError> {
    let mut first_error = None;
    for plan in plans {
        on_plan(plan);
        if let Err(error) = backend.execute(plan, cancelled, on_event) {
            if matches!(error, InfrastructureError::Cancelled) {
                return Err(error);
            }
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub struct SystemHomebrew {
    prefix: PathBuf,
    catalog: CacheCatalogProvider,
    api_catalog: ApiCatalogProvider,
}

impl SystemHomebrew {
    pub fn new(prefix: PathBuf) -> Result<Self, InfrastructureError> {
        Ok(Self {
            prefix,
            catalog: CacheCatalogProvider::standard()?,
            api_catalog: ApiCatalogProvider::new()?,
        })
    }

    fn command(
        &self,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<CommandSpec, InfrastructureError> {
        let mut env = HashMap::from([
            (
                OsString::from("PATH"),
                OsString::from(format!(
                    "{}/bin:{}/sbin:/usr/bin:/bin:/usr/sbin:/sbin",
                    self.prefix.display(),
                    self.prefix.display()
                )),
            ),
            (
                OsString::from("HOMEBREW_NO_AUTO_UPDATE"),
                OsString::from("1"),
            ),
            (OsString::from("HOMEBREW_NO_ENV_HINTS"), OsString::from("1")),
            (OsString::from("HOMEBREW_NO_ANALYTICS"), OsString::from("1")),
            (OsString::from("HOMEBREW_NO_COLOR"), OsString::from("1")),
        ]);
        if let Ok(executable) = std::env::current_exe()
            && let Some(helper) = validated_askpass_helper(&executable)?
        {
            env.insert(OsString::from("SUDO_ASKPASS"), helper.into_os_string());
        }
        Ok(CommandSpec {
            program: self.prefix.join("bin/brew"),
            args: args.into_iter().collect(),
            env,
            current_dir: PathBuf::from("/"),
        })
    }
}

impl HomebrewBackend for SystemHomebrew {
    fn prefix(&self) -> &Path {
        &self.prefix
    }

    fn installed(&self) -> Result<Vec<Package>, InfrastructureError> {
        installed::discover(&self.prefix)
    }

    fn catalog(&self, cancelled: &dyn Fn() -> bool) -> Result<Vec<Package>, InfrastructureError> {
        match self.catalog.load() {
            Ok(packages) => Ok(packages),
            Err(cache) => {
                if cancelled() {
                    return Err(InfrastructureError::Cancelled);
                }
                self.api_catalog
                    .load()
                    .map_err(|fallback| InfrastructureError::CatalogFallback {
                        cache: Box::new(cache),
                        fallback: Box::new(fallback),
                    })
            }
        }
    }

    fn outdated(&self, cancelled: &dyn Fn() -> bool) -> Result<Vec<Package>, InfrastructureError> {
        let spec = self.command(["outdated", "--json=v2"].into_iter().map(OsString::from))?;
        let output = process::run(&spec, cancelled, |_| {})?;
        let document: OutdatedDocument =
            serde_json::from_str(&output.stdout).map_err(|source| InfrastructureError::Json {
                context: "brew outdated --json=v2",
                source,
            })?;
        Ok(document.into_packages())
    }

    fn execute(
        &self,
        plan: &CommandPlan,
        cancelled: &dyn Fn() -> bool,
        on_event: &mut dyn FnMut(ProcessEvent),
    ) -> Result<(), InfrastructureError> {
        let spec = self.command(plan.arguments())?;
        process::run(&spec, cancelled, on_event).map(|_| ())
    }
}

pub fn detect_prefix() -> Option<PathBuf> {
    [PathBuf::from("/opt/homebrew"), PathBuf::from("/usr/local")]
        .into_iter()
        .find(|prefix| prefix.join("bin/brew").is_file())
}

#[derive(Deserialize)]
struct OutdatedItem {
    name: String,
    #[serde(default)]
    installed_versions: Vec<String>,
    current_version: Option<String>,
    #[serde(default)]
    pinned: bool,
}

#[derive(Deserialize)]
struct OutdatedDocument {
    #[serde(default)]
    formulae: Vec<OutdatedItem>,
    #[serde(default)]
    casks: Vec<OutdatedItem>,
}

impl OutdatedDocument {
    fn into_packages(self) -> Vec<Package> {
        let mut packages = Vec::with_capacity(self.formulae.len() + self.casks.len());
        for (items, kind) in [
            (self.formulae, PackageKind::Formula),
            (self.casks, PackageKind::Cask),
        ] {
            packages.extend(items.into_iter().filter_map(|item| {
                Some(Package::outdated(
                    PackageId::new(item.name, kind)?,
                    item.installed_versions.last().and_then(Version::new),
                    item.current_version.as_deref().and_then(Version::new),
                    item.pinned,
                ))
            }));
        }
        packages.sort_by(|left, right| left.id().cmp(right.id()));
        packages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn id(kind: PackageKind) -> PackageId {
        PackageId::new("shared", kind).unwrap()
    }

    #[test]
    fn formula_and_cask_commands_are_namespace_explicit() {
        for action in [BrewAction::Install, BrewAction::Upgrade] {
            let plans = plan_commands(action, [id(PackageKind::Cask), id(PackageKind::Formula)]);
            assert_eq!(plans.len(), 2);
            assert_eq!(
                plans[0].arguments(),
                [action.command(), "shared"].map(OsString::from)
            );
            assert_eq!(
                plans[1].arguments(),
                [action.command(), "--cask", "shared"].map(OsString::from)
            );
        }
    }

    struct FailingFormulaBackend {
        attempted: Mutex<Vec<PackageKind>>,
    }

    impl HomebrewBackend for FailingFormulaBackend {
        fn prefix(&self) -> &Path {
            Path::new("/")
        }

        fn installed(&self) -> Result<Vec<Package>, InfrastructureError> {
            Ok(Vec::new())
        }

        fn catalog(&self, _: &dyn Fn() -> bool) -> Result<Vec<Package>, InfrastructureError> {
            Ok(Vec::new())
        }

        fn outdated(&self, _: &dyn Fn() -> bool) -> Result<Vec<Package>, InfrastructureError> {
            Ok(Vec::new())
        }

        fn execute(
            &self,
            plan: &CommandPlan,
            _: &dyn Fn() -> bool,
            _: &mut dyn FnMut(ProcessEvent),
        ) -> Result<(), InfrastructureError> {
            self.attempted.lock().unwrap().push(plan.kind);
            if plan.kind == PackageKind::Formula {
                Err(InfrastructureError::NonZeroExit {
                    program: PathBuf::from("brew"),
                    code: Some(1),
                    stdout: String::new(),
                    stderr: "formula failed".to_owned(),
                })
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn independent_namespaces_are_attempted_after_one_fails() {
        let backend = FailingFormulaBackend {
            attempted: Mutex::new(Vec::new()),
        };
        let plans = plan_commands(
            BrewAction::Upgrade,
            [id(PackageKind::Formula), id(PackageKind::Cask)],
        );
        assert!(execute_plans(&backend, &plans, &|| false, |_| {}, &mut |_| {}).is_err());
        assert_eq!(
            *backend.attempted.lock().unwrap(),
            vec![PackageKind::Formula, PackageKind::Cask]
        );
    }
}
