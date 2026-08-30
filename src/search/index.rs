use super::{fold, score};
use crate::domain::{Package, PackageId};
use std::{collections::HashMap, sync::Arc};

struct SearchDocument {
    name: Arc<str>,
    description: Arc<str>,
    folded_name: Vec<u8>,
    folded_description: Vec<u8>,
}

#[derive(Default)]
pub struct SearchIndex {
    documents: HashMap<PackageId, SearchDocument>,
}

impl SearchIndex {
    pub fn rebuild<'a>(&mut self, packages: impl IntoIterator<Item = &'a Package>) {
        self.documents.clear();
        self.documents.extend(packages.into_iter().map(|package| {
            let name = package.id().name().shared();
            let description = package.shared_description().unwrap_or_default();
            (
                package.id().clone(),
                SearchDocument {
                    folded_name: fold(&name),
                    folded_description: fold(&description),
                    name,
                    description,
                },
            )
        }));
    }

    pub fn rank(&self, query: &str, candidates: &Arc<[PackageId]>) -> Arc<[PackageId]> {
        let needle = fold(query);
        if needle.is_empty() {
            return candidates.clone();
        }
        let mut scored: Vec<_> = candidates
            .iter()
            .enumerate()
            .filter_map(|(position, id)| {
                let document = self.documents.get(id)?;
                let name = score(&needle, &document.folded_name, document.name.as_bytes());
                let description = score(
                    &needle,
                    &document.folded_description,
                    document.description.as_bytes(),
                )
                .map(|value| value / 3 - 40);
                let best = match (name, description) {
                    (Some(left), Some(right)) => left.max(right),
                    (Some(value), None) | (None, Some(value)) => value,
                    (None, None) => return None,
                };
                Some((best, position, id.clone()))
            })
            .collect();
        scored.sort_unstable_by(|left, right| {
            right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1))
        });
        scored
            .into_iter()
            .map(|(_, _, id)| id)
            .collect::<Vec<_>>()
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_reuses_the_canonical_view() {
        let candidates: Arc<[PackageId]> = vec![
            PackageId::new("a", crate::domain::PackageKind::Formula).unwrap(),
            PackageId::new("a", crate::domain::PackageKind::Cask).unwrap(),
        ]
        .into();
        let ranked = SearchIndex::default().rank("", &candidates);
        assert!(Arc::ptr_eq(&ranked, &candidates));
    }
}
