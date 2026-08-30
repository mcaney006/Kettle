use crate::domain::{BrewAction, PackageId};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum View {
    #[default]
    Outdated,
    Installed,
    Browse,
    Settings,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppAction {
    ChangeView(View),
    Refresh,
    Mutate {
        action: BrewAction,
        targets: Vec<PackageId>,
    },
    SearchChanged(String),
    CancelAuthentication,
    SignOut,
}
