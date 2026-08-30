#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BrewAction {
    Install,
    Upgrade,
}

impl BrewAction {
    pub const fn command(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Install => "Install",
            Self::Upgrade => "Upgrade",
        }
    }

    pub const fn progressive(self) -> &'static str {
        match self {
            Self::Install => "Installing",
            Self::Upgrade => "Upgrading",
        }
    }

    pub const fn completed(self) -> &'static str {
        match self {
            Self::Install => "Installed",
            Self::Upgrade => "Upgraded",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_facing_grammar_is_explicit() {
        assert_eq!(BrewAction::Install.label(), "Install");
        assert_eq!(BrewAction::Install.progressive(), "Installing");
        assert_eq!(BrewAction::Install.completed(), "Installed");
        assert_eq!(BrewAction::Upgrade.label(), "Upgrade");
        assert_eq!(BrewAction::Upgrade.progressive(), "Upgrading");
        assert_eq!(BrewAction::Upgrade.completed(), "Upgraded");
    }
}
