# Kettle

Kettle is a native macOS frontend for Homebrew, written in Rust with GPUI and rendered through Metal. It provides a dense, virtualized view of installed, outdated, and available formulae and casks without embedding a browser, running a localhost service, or paying Ruby startup cost merely to paint the first window.

Homebrew remains authoritative for every mutation. Kettle reads installed state directly from the Homebrew prefix for a fast preview, reads Homebrew's local API cache behind a validated provider, and uses supported Homebrew commands for outdated state, catalog fallback, installs, and upgrades. Formulae and casks are always identified and executed separately, including when both namespaces contain the same textual name.

## Requirements

- macOS 11 or newer
- Homebrew installed at `/opt/homebrew` or `/usr/local`
- Xcode Command Line Tools (`xcode-select --install`)
- Rust 1.88.0 or newer; `rust-toolchain.toml` pins the dependency-tested baseline and required components

Kettle is tested for both `aarch64-apple-darwin` and `x86_64-apple-darwin`. It is intentionally a macOS application; other platforms are not supported.

## Build and run

```sh
git clone https://github.com/mcaney006/Kettle.git
cd Kettle
cargo run --release
```

A normal `cargo build` or `cargo run` is the local developer path. It does not create an application bundle, sign a public release, or notarize anything.

## Architecture

```text
src/
├── main.rs                  # three-line composition entry point
├── lib.rs
├── domain/                  # PackageId, PackageKind, versions, update state, BrewAction
├── application/             # application state machines, selection, cancellation, actions
├── infrastructure/
│   ├── homebrew/            # discovery, catalog provider, planning, process execution
│   ├── github/              # device OAuth, HTTP transport, Keychain token store
│   └── privilege/           # bundled askpass helper validation
├── search/                  # folded search projection and scorer
└── ui/                      # GPUI composition, system theme, real text input
```

The canonical package identity is `(canonical name, PackageKind)`. Maps, overlays, selection, search, and command plans use that complete identity; joins by name alone are not allowed. Versions and descriptions use explicit optional state. Homebrew's `outdated --json=v2` result is authoritative for update availability; the local comparator is used only to choose among installed version directories.

`HomebrewBackend`, `CatalogProvider`, `OAuthTransport`, and `TokenStore` are the narrow external boundaries used by tests. UI rendering consumes application state and emits typed actions. Background work uses GPUI's executors plus cancellation tokens rather than adding an async runtime.

## Homebrew behavior

- Installed formulae are scanned under `<prefix>/Cellar`; installed casks are scanned under `<prefix>/Caskroom`.
- Formula pin state comes from `<prefix>/var/homebrew/pinned` and remains formula-specific.
- Catalog loading first attempts `~/Library/Caches/Homebrew/api/internal/packages.*.jws.json.payload`. That location and schema are treated as a private optimization: reads verify stable file identity/metadata, retry bounded replacement races, and reject malformed or empty data.
- If the private cache is unavailable or invalid, Kettle falls back to Homebrew's supported public formula and cask JSON APIs with bounded network deadlines. The slower supported fallback is not on the normal launch path.
- Outdated state comes from `brew outdated --json=v2`.
- Installs and upgrades use direct argv execution, never a shell. Formula and cask batches are separate.
- Exit status alone determines command success. Stderr output from a successful command remains informational rather than turning the operation into a failure.

## GitHub sign-in

GitHub sign-in uses the OAuth device flow with only the `read:user` scope. It identifies the signed-in account; it does **not** read repositories, inspect repository languages, or enable repository recommendations. Kettle does not request repository write access.

OAuth requests have explicit connect and request deadlines. Polling honors the server interval, `slow_down`, denial, expiry, and cancellation. Access tokens are sent in an in-process HTTP Authorization header and are never placed in process arguments, environment variables, temporary files, logs, or debug output. Tokens are stored as a generic password in the macOS Keychain under Kettle's fixed service and account identifiers. Unit tests use fakes and never access a developer's Keychain or GitHub account.

## Optional sudo / 1Password integration

The application bundle includes a small `kettle-askpass` helper. It is opt-in; without its configuration file Homebrew and sudo behave normally.

To opt in, create a file containing one 1Password secret reference, then restrict it:

```sh
install -d -m 700 "$HOME/.config/kettle"
printf '%s\n' 'op://Private/macOS/password' > "$HOME/.config/kettle/sudo-secret"
chmod 600 "$HOME/.config/kettle/sudo-secret"
```

Replace the example reference with the user's actual vault/item/field reference; do not put the password itself in that file. The helper uses only fixed Homebrew locations for the 1Password CLI, writes only password bytes to stdout after success, clears its output buffer, and fails with empty stdout on every error. See [the privilege threat model](docs/PRIVILEGE_THREAT_MODEL.md) before enabling it.

## Interaction and shortcuts

The search field is a GPUI input handler, not a painted key listener. It supports insertion, grapheme-aware deletion/navigation, selection, clipboard actions, marked text/IME composition, keyboard layouts, mouse selection, focus indication, and the standard Edit menu actions.

| Action | Shortcut |
| --- | --- |
| Refresh | Command-R |
| Upgrade all visible outdated packages | Command-U |
| Clear search | Command-K |
| Outdated / Installed / Browse | Command-1 / Command-2 / Command-3 |
| Settings | Command-, |
| Move selection | Up / Down |
| Extend or contract selection | Shift-Up / Shift-Down |
| Select visible packages | Command-A when the package list is focused |
| Run the primary action | Return when the package list is focused |
| Toggle a row | Command-click |
| Select a row range | Shift-click |

Kettle follows the system light/dark appearance and exposes normal application, Edit, View, Window, and Help menus. GPUI 0.2.2 supports keyboard focus/tab stops but does not yet expose public per-element accessibility role/label APIs; complete VoiceOver role annotation is therefore a known framework limitation rather than simulated metadata.

## Tests and quality checks

Tests use isolated temporary directories and fake external boundaries. They do not mutate the real Homebrew installation, Keychain, 1Password vault, or GitHub account.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check --warn unmaintained
cargo build --workspace --release --all-features
```

The dependency gate denies vulnerabilities, direct advisories, unknown registries/git sources, wildcard requirements, and unapproved licenses. Transitive unmaintained notices from the latest published GPUI 0.2.2 dependency graph remain visible warnings because no safe GPUI upgrade currently exists.

Search performance is measured outside unit tests:

```sh
cargo bench --bench search
cargo run --release --example perf_probe
```

When a current Homebrew API cache with at least 10,000 entries exists, the benchmark uses that real catalog. Otherwise it uses a deterministic 16,291-package corpus. Absolute latency is reported for engineering review but is not a flaky CI assertion.

See [the performance record](docs/PERFORMANCE.md) for the baseline, current
measurements, hardware, workload boundaries, and reproduction commands.

## Packaging and release

`tools/bundle.sh` has three explicit modes:

```sh
./tools/bundle.sh dev       # host-architecture .app, ad-hoc signed, not notarized
./tools/bundle.sh adhoc     # universal .app and DMG, ad-hoc signed, not notarized
./tools/bundle.sh release   # universal Developer ID signing and notarization
```

Development and ad-hoc artifacts are for local testing. They are not described as publicly distributable.

Public release mode requires credentials already provisioned outside the repository:

```sh
xcrun notarytool store-credentials kettle-notary
export KETTLE_CODESIGN_IDENTITY='Developer ID Application: …'
export KETTLE_NOTARY_PROFILE='kettle-notary'
export KETTLE_BUNDLE_ID='owner.assigned.bundle.identifier'
./tools/bundle.sh release
```

The script never invents an identity. It signs the helper and main executable individually, signs the outer bundle without `--deep`, enables the hardened runtime, verifies every signature, submits the application archive for notarization, staples and validates the app, creates and signs the DMG, notarizes and staples the DMG, and runs `spctl` assessments. Signing and notarization secrets belong in the login Keychain or CI secret store, never in source.

## Known limitations

- Homebrew prefixes outside `/opt/homebrew` and `/usr/local` are not detected automatically.
- The fast catalog source is a private Homebrew cache and can change; Kettle falls back safely but the fallback is slower.
- Process cancellation terminates the directly launched Homebrew process. Kettle does not make a portability claim that every descendant process can always be terminated atomically.
- Version ordering is deliberately described as Kettle's local installed-directory ordering, not as a reimplementation of Homebrew's complete version semantics.
- GPUI 0.2.2 does not expose accessibility labels/roles for arbitrary elements, limiting VoiceOver metadata despite keyboard-operable controls.
- The latest GPUI dependency graph contains transitive unmaintained-crate advisories with no safe published GPUI upgrade; the dependency gate reports them on every run.
- No source license is present. Selecting and adding a license remains an explicit repository-owner decision.
