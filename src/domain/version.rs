use std::{cmp::Ordering, fmt, sync::Arc};

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct Version(Arc<str>);

impl Version {
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

impl fmt::Debug for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Version").field(&self.0).finish()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        super::version_cmp(self.as_str(), other.as_str())
            .then_with(|| self.as_str().cmp(other.as_str()))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_versions_are_absent() {
        assert_eq!(Version::new(""), None);
        assert_eq!(Version::new("  "), None);
        assert_eq!(Version::new("1.2").unwrap().as_str(), "1.2");
    }

    #[test]
    fn ordering_is_consistent_with_exact_equality() {
        let short = Version::new("1.0").unwrap();
        let padded = Version::new("1.00").unwrap();
        assert_ne!(short, padded);
        assert_ne!(short.cmp(&padded), Ordering::Equal);
    }
}
