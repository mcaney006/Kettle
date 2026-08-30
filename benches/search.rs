use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use kettle::{
    domain::{Package, PackageId, PackageKind, Version},
    infrastructure::homebrew::{CacheCatalogProvider, CatalogProvider},
    search::SearchIndex,
};

const SYNTHETIC_SIZE: usize = 16_291;

fn synthetic_catalog(size: usize) -> Vec<Package> {
    (0..size)
        .map(|index| {
            Package::catalog(
                PackageId::new(format!("package-name-{index}-tool"), PackageKind::Formula).unwrap(),
                Version::new(format!("1.{index}")),
                Some(format!(
                    "A realistic Homebrew utility description for package {index}"
                )),
            )
        })
        .collect()
}

fn benchmark_search(criterion: &mut Criterion) {
    let packages = CacheCatalogProvider::standard()
        .and_then(|provider| provider.load())
        .ok()
        .filter(|packages| packages.len() >= 10_000)
        .unwrap_or_else(|| synthetic_catalog(SYNTHETIC_SIZE));
    let ids: std::sync::Arc<[_]> = packages
        .iter()
        .map(|package| package.id().clone())
        .collect::<Vec<_>>()
        .into();
    let mut index = SearchIndex::default();
    index.rebuild(&packages);
    let mut group = criterion.benchmark_group("catalog_search");
    for query in ["pnt", "brew utility", "package-name-15999"] {
        group.bench_with_input(
            BenchmarkId::new(query, packages.len()),
            query,
            |bencher, query| {
                bencher.iter(|| index.rank(query, &ids));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, benchmark_search);
criterion_main!(benches);
