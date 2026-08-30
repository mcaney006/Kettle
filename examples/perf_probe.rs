use kettle::{
    application::{PackageStore, View},
    infrastructure::homebrew::{
        CacheCatalogProvider, CatalogProvider, HomebrewBackend, SystemHomebrew, detect_prefix,
    },
};
use std::{hint::black_box, time::Instant};

fn percentile(samples: &mut [u128], percentile: usize) -> u128 {
    samples.sort_unstable();
    samples[(samples.len() - 1) * percentile / 100]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prefix = detect_prefix().ok_or("Homebrew not found")?;
    let backend = SystemHomebrew::new(prefix)?;

    let started = Instant::now();
    let installed = backend.installed()?;
    let installed_us = started.elapsed().as_micros();

    let started = Instant::now();
    let catalog = CacheCatalogProvider::standard()?.load()?;
    let catalog_ms = started.elapsed().as_millis();

    let started = Instant::now();
    let mut store = PackageStore::default();
    store.preview_catalog(&catalog);
    store.preview_installed(&installed);
    let projection_ms = started.elapsed().as_millis();

    let mut browse = Vec::with_capacity(1_000);
    for _ in 0..1_000 {
        let started = Instant::now();
        black_box(store.filtered(View::Browse, ""));
        browse.push(started.elapsed().as_nanos());
    }

    let mut search = Vec::with_capacity(500);
    for _ in 0..500 {
        let started = Instant::now();
        black_box(store.filtered(View::Browse, "git"));
        search.push(started.elapsed().as_nanos());
    }

    let started = Instant::now();
    let outdated = backend.outdated(&|| false)?;
    let outdated_ms = started.elapsed().as_millis();

    println!(
        "installed_count={} installed_scan_us={installed_us}",
        installed.len()
    );
    println!(
        "catalog_count={} catalog_load_ms={catalog_ms}",
        catalog.len()
    );
    println!("projection_ms={projection_ms}");
    println!(
        "browse_transition_p50_ns={} browse_transition_p95_ns={}",
        percentile(&mut browse.clone(), 50),
        percentile(&mut browse, 95)
    );
    println!(
        "search_git_p50_us={} search_git_p95_us={}",
        percentile(&mut search.clone(), 50) / 1_000,
        percentile(&mut search, 95) / 1_000
    );
    println!(
        "outdated_count={} outdated_ms={outdated_ms}",
        outdated.len()
    );
    Ok(())
}
