//! Kettle -- a GPU-rendered Homebrew front-end.
//!
//! The UI is GPUI: every pixel is drawn by Metal through GPUI's AppKit/Objective-C
//! platform layer (NSWindow + CAMetalLayer), not by AppKit controls.
//!
//! Blocking brew work runs on GPUI's background executor and lands back on the
//! foreground executor via `Entity::update`, so the render thread never blocks.

mod brew;
mod rank;

use brew::Pkg;
use gpui::{
    actions, div, prelude::*, px, rgb, size, uniform_list, App, Application, Bounds, Context,
    FocusHandle, Focusable, KeyDownEvent, Menu, MenuItem, ScrollStrategy, SharedString,
    UniformListScrollHandle, Window, WindowBounds, WindowOptions,
};
use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

actions!(
    kettle,
    [
        Quit,
        Refresh,
        UpgradeAll,
        Primary,
        ClearSearch,
        ViewOutdated,
        ViewInstalled,
        ViewBrowse
    ]
);

// --- palette: neutral graphite, in the register of a macOS system utility
// (Activity Monitor, Font Book) rather than a branded app. Deliberately NOT
// themed: hue is information here, not decoration. Only two colors carry
// meaning -- STALE marks a version you should act on, and SEL is the system
// selection blue. Everything else is grey, so the two that aren't stand out.
const BG: u32 = 0x1E1E1E;
const CHROME: u32 = 0x2A2A2A;
const LINE: u32 = 0x383838;
const INK: u32 = 0xE4E4E4;
const MUTE: u32 = 0x8E8E8E;
const FAINT: u32 = 0x5E5E5E;
const SEL: u32 = 0x2C5FA8;
const STALE: u32 = 0xD9903A;
const FAIL: u32 = 0xD4726A;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Outdated,
    Installed,
    Browse,
}

/// Column layout, shared by the header and every row so they cannot drift.
/// Widths are fixed rather than flexed on purpose: a flexed last column drifts
/// out of alignment with the header the moment the list grows a scrollbar.
///
/// "Installed" and "Latest" used to be two equal columns. They are one now --
/// this app exists to show a *change*, so the change is a single cell that
/// reads `14.1.0 -> 14.1.1`, not two numbers you have to diff by eye.
/// Version is the widest fixed column because it holds two versions and an
/// arrow, and truncating it drops digits off the one number this app exists to
/// show: real cask versions run to `26.803.81509 -> 26.825.41651` (28 chars).
///
/// Description's width is unused -- it flexes (see `cell`). Under a tiling
/// window manager the window is routinely half-screen, and a fourth fixed
/// column simply pushes Description off the right edge where it can never be
/// read. The first three stay fixed so the header cannot drift from the rows.
const COLS: [(&str, f32); 4] = [
    ("Package", 220.0),
    ("Kind", 64.0),
    ("Version", 260.0),
    ("Description", 0.0),
];

/// Body/UI stack. Named fallbacks matter: `font_family` takes one name, and if
/// SF Pro is ever unavailable GPUI silently falls back to something arbitrary.
const UI_FONT: &str = "SF Pro Text";
const MONO_FONT: &str = "SF Mono";

/// Rank `pkgs` into row indices. No Pkg is cloned -- this runs on every
/// keystroke over the full 16k catalog.
///
/// An empty needle lists everything. Browse used to return an empty Vec here on
/// the theory that 16k unfiltered rows were noise, which made the tab
/// impossible to actually browse and indistinguishable from a broken catalog.
/// The list is virtualized, so showing all of it costs nothing.
fn rank_rows(needle: &[u8], pkgs: &[Pkg]) -> Vec<u32> {
    if needle.is_empty() {
        return (0..pkgs.len() as u32).collect();
    }
    let mut scored: Vec<(i32, u32)> = pkgs
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let n = rank::score(needle, &p.name_lc, p.name.as_bytes());
            let d = rank::score(needle, &p.desc_lc, p.desc.as_bytes()).map(|s| s / 3 - 40);
            let best = match (n, d) {
                (Some(a), Some(b)) => a.max(b),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => return None,
            };
            Some((best, i as u32))
        })
        .collect();
    scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// A cell at column `c`, used by both the header and every row so the two
/// cannot drift. The shrink/min-width guards are load-bearing: without them a
/// long value grows past its column and shoves the rest of the row out of
/// alignment with the header.
fn cell(c: usize) -> gpui::Div {
    let base = div()
        .min_w_0()
        .px_1()
        .text_sm()
        .whitespace_nowrap()
        .overflow_hidden()
        .truncate();
    if c == COLS.len() - 1 {
        // Last column takes whatever is left, so it survives a narrow window.
        base.flex_1()
    } else {
        base.w(px(COLS[c].1)).flex_shrink_0()
    }
}

/// "upgrade" -> "Upgrading", "install" -> "Installing". The verb arrives
/// lowercase because it doubles as the literal brew subcommand.
///
/// English drops a silent trailing `e` before `-ing`; naive concatenation gives
/// you "Upgradeing". Only ever fed the two verbs this app runs.
fn progressive(verb: &str) -> String {
    let stem = verb.strip_suffix('e').unwrap_or(verb);
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str() + "ing",
        None => String::new(),
    }
}

/// Next cursor position for an arrow keypress, clamped to the list.
///
/// Split out from `Kettle` so it is testable: building a `Kettle` needs a
/// `FocusHandle`, which needs a running GPUI app.
fn next_cursor(cur: Option<usize>, delta: isize, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    Some(match cur {
        // Nothing selected yet: Down starts at the top, Up starts at the bottom,
        // rather than silently landing on row 0 for both.
        None => {
            if delta > 0 {
                0
            } else {
                last
            }
        }
        Some(c) => c.saturating_add_signed(delta).min(last),
    })
}

struct Kettle {
    prefix: PathBuf,
    tab: Tab,
    installed: Vec<Pkg>,
    catalog: Vec<Pkg>,
    outdated: Vec<Pkg>,
    /// Active tab's packages. Keystrokes filter indices into this, never clone it.
    snapshot: Vec<Pkg>,
    rows: Vec<u32>,
    query: String,
    selected: HashSet<u32>,
    /// Keyboard cursor as a position in `rows`, not a snapshot index -- arrows
    /// move in visible order, and `rows` is what's visible.
    cursor: Option<usize>,
    log: Arc<Mutex<Vec<String>>>,
    /// Raw brew output is diagnostic, not ambient. Closed until you ask for it,
    /// or until a run starts and it becomes the thing you're watching.
    log_open: bool,
    busy: bool,
    status: SharedString,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl Kettle {
    fn new(prefix: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut k = Self {
            prefix,
            tab: Tab::Outdated,
            installed: Vec::new(),
            catalog: Vec::new(),
            outdated: Vec::new(),
            snapshot: Vec::new(),
            rows: Vec::new(),
            query: String::new(),
            selected: HashSet::new(),
            cursor: None,
            log: Arc::new(Mutex::new(Vec::new())),
            log_open: false,
            busy: false,
            status: "Loading".into(),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        };
        k.refresh(cx);
        k
    }

    fn push_log(&self, line: impl Into<String>) {
        let mut g = self.log.lock().unwrap_or_else(|e| e.into_inner());
        g.push(line.into());
        // A long `brew upgrade` emits thousands of lines; keep the tail bounded.
        if g.len() > 4000 {
            let n = g.len() - 2000;
            g.drain(..n);
        }
    }

    /// Rebuild the active-tab snapshot, then re-filter.
    fn resnapshot(&mut self) {
        let mut snap = match self.tab {
            Tab::Outdated => self.outdated.clone(),
            Tab::Installed => self.installed.clone(),
            Tab::Browse => self.catalog.clone(),
        };
        if self.tab == Tab::Installed {
            let stale: HashSet<&str> = self.outdated.iter().map(|p| p.name.as_str()).collect();
            for p in snap.iter_mut() {
                p.outdated = stale.contains(p.name.as_str());
            }
        } else {
            // Every row on Outdated is outdated by definition; the marker is noise.
            for p in snap.iter_mut() {
                p.outdated = false;
            }
        }
        // `brew outdated` carries no descriptions; graft them from the catalog.
        if self.tab != Tab::Browse && !self.catalog.is_empty() {
            let by_name: std::collections::HashMap<&str, &Pkg> =
                self.catalog.iter().map(|p| (p.name.as_str(), p)).collect();
            for p in snap.iter_mut() {
                if p.desc.is_empty() {
                    let short = p.name.rsplit('/').next().unwrap_or(&p.name);
                    if let Some(c) = by_name.get(short) {
                        p.desc = c.desc.clone();
                        p.desc_lc = c.desc_lc.clone();
                    }
                }
            }
        }
        self.snapshot = snap;
        self.selected.clear();
        self.refilter();
    }

    fn refilter(&mut self) {
        let needle = rank::fold(&self.query);
        self.rows = rank_rows(&needle, &self.snapshot);
        // Rows were just rebuilt, so any old cursor position points at a
        // different package now. Every caller that reshuffles rows goes through
        // here, so this is the one place that needs to forget it.
        self.cursor = None;
    }

    /// Move the keyboard cursor and select what it lands on. `extend` (shift)
    /// adds to the selection instead of replacing it.
    ///
    /// ponytail: shift+arrow grows the selection one row at a time but never
    /// shrinks it -- reversing direction re-adds rows that are already in the
    /// set instead of dropping them. Matches the additive shift+click already in
    /// `row()`, and the ceiling is that there is no anchor. Upgrade path: keep an
    /// `anchor: Option<usize>` set on the first extend, and on each move replace
    /// the selection with the anchor..=cursor span.
    fn move_cursor(&mut self, delta: isize, extend: bool) {
        let next = next_cursor(self.cursor, delta, self.rows.len());
        self.cursor = next;
        let Some(n) = next else { return };
        if !extend {
            self.selected.clear();
        }
        self.selected.insert(self.rows[n]);
        // Non-strict: a row already on screen doesn't yank the viewport.
        self.scroll.scroll_to_item(n, ScrollStrategy::Top);
    }

    /// Switch views. Shared by the sidebar click and the Cmd-1/2/3 bindings.
    fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.resnapshot();
        cx.notify();
    }

    /// The primary action: upgrade the selection, or install it on Browse.
    /// Shared by the toolbar button and the Enter key so they cannot diverge.
    fn primary(&mut self, cx: &mut Context<Self>) {
        let items = self.selected_items();
        let verb = if self.tab == Tab::Browse { "install" } else { "upgrade" };
        // run_brew no-ops on an empty set, so Enter with nothing selected is safe.
        self.run_brew(verb, items, cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Scanning…".into();
        let prefix = self.prefix.clone();

        cx.spawn(async move |this, cx| {
            // Instant: pure filesystem + cached catalog, no Ruby.
            let p2 = prefix.clone();
            let (inst, cat) = cx
                .background_spawn(async move { (brew::scan_installed(&p2), brew::load_catalog()) })
                .await;
            this.update(cx, |this, cx| {
                this.installed = inst;
                match cat {
                    Ok(c) => this.catalog = c,
                    // An empty catalog is indistinguishable from a broken one on
                    // screen, so name the reason instead of showing nothing.
                    Err(e) => this.push_log(format!("Couldn't load the catalog. {e}")),
                }
                this.status = "Checking for updates".into();
                this.resnapshot();
                cx.notify();
            })
            .ok();

            // Slow: brew's own outdated logic (portable-ruby boot, ~2s).
            let p3 = prefix.clone();
            let res = cx.background_spawn(async move { brew::outdated(&p3) }).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(o) => this.outdated = o,
                    Err(e) => this.push_log(format!("Couldn't read outdated packages. {e}")),
                }
                this.busy = false;
                this.status = "".into();
                this.resnapshot();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// `items` is (name, is_cask). Formulae and casks MUST be separate commands:
    /// a token can exist as both (e.g. `ant`), and a bare name resolves to the
    /// formula, so `brew upgrade ant` fails with "ant not installed" and aborts
    /// the whole batch even though the cask is what's installed.
    fn run_brew(&mut self, verb: &'static str, items: Vec<(String, bool)>, cx: &mut Context<Self>) {
        if self.busy || items.is_empty() {
            return;
        }
        self.busy = true;
        let n = items.len();
        self.status =
            format!("{} {n} package{}", progressive(verb), if n == 1 { "" } else { "s" }).into();
        // A run is the one time raw output is what you actually want to look at.
        self.log_open = true;
        let prefix = self.prefix.clone();
        let log = self.log.clone();

        cx.spawn(async move |this, cx| {
            let p2 = prefix.clone();
            let log2 = log.clone();
            let res = cx
                .background_spawn(async move {
                    let append = |line: String| {
                        let mut g = log2.lock().unwrap_or_else(|e| e.into_inner());
                        g.push(line);
                        if g.len() > 4000 {
                            let n = g.len() - 2000;
                            g.drain(..n);
                        }
                    };

                    let mut failures = Vec::new();
                    for args in brew::plan(verb, items) {
                        append(format!("$ brew {}", args.join(" ")));
                        if let Err(e) = brew::run_stream(&p2, &args, append) {
                            failures.push(e);
                        }
                    }
                    if failures.is_empty() {
                        Ok(())
                    } else {
                        Err(failures.join("; "))
                    }
                })
                .await;

            // Success is exit status, never stderr -- see brew.rs.
            this.update(cx, |this, _| match res {
                Ok(()) => this.push_log(format!("Finished. {n} package{} {verb}d.",
                    if n == 1 { "" } else { "s" })),
                Err(e) => this.push_log(format!("Failed. {e}")),
            })
            .ok();

            let p3 = prefix.clone();
            let (inst, out) = cx
                .background_spawn(async move { (brew::scan_installed(&p3), brew::outdated(&p3)) })
                .await;
            this.update(cx, |this, cx| {
                this.installed = inst;
                if let Ok(o) = out {
                    this.outdated = o;
                }
                this.busy = false;
                this.status = "".into();
                this.resnapshot();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn selected_items(&self) -> Vec<(String, bool)> {
        let mut v: Vec<(String, bool)> = self
            .selected
            .iter()
            .filter_map(|&i| self.snapshot.get(i as usize))
            .filter(|p| !p.pinned)
            .map(|p| (p.name.clone(), p.cask))
            .collect();
        v.sort();
        v
    }

    fn all_outdated_items(&self) -> Vec<(String, bool)> {
        self.outdated
            .iter()
            .filter(|p| !p.pinned)
            .map(|p| (p.name.clone(), p.cask))
            .collect()
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        // `platform` is Cmd on macOS. Let shortcuts through to the action system.
        if m.platform || m.control || m.function {
            return;
        }
        match ks.key.as_str() {
            // Navigation only moves the cursor -- it must not touch the query or
            // wipe the selection the way the editing keys below do.
            "up" | "down" => {
                self.move_cursor(if ks.key == "down" { 1 } else { -1 }, m.shift);
                cx.notify();
                return;
            }
            "backspace" => {
                self.query.pop();
            }
            "escape" => self.query.clear(),
            "enter" => return,
            _ => {
                // key_char carries the actually-typed character (layout/IME aware);
                // ks.key is the physical key label.
                let Some(c) = ks.key_char.as_ref() else { return };
                if c.chars().any(|ch| ch.is_control()) {
                    return;
                }
                self.query.push_str(c);
            }
        }
        self.refilter();
        self.selected.clear();
        cx.notify();
    }

    // ---- view helpers

    /// One row of the source list. A sidebar uses an *inset rounded* selection,
    /// unlike the full-width selection of the data table beside it -- that
    /// contrast is how macOS distinguishes navigation from content.
    fn sidebar_item(
        &self,
        tab: Tab,
        label: &'static str,
        count: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on = self.tab == tab;
        div()
            .id(label)
            .flex()
            .flex_row()
            .items_center()
            .mx_2()
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .text_sm()
            .cursor_pointer()
            .when(on, |d| d.bg(rgb(0x3C3C3C)).text_color(rgb(INK)))
            .when(!on, |d| {
                d.text_color(rgb(MUTE)).hover(|h| h.bg(rgb(0x303030)))
            })
            .child(div().flex_1().child(label))
            // The count is the whole reason a sidebar beats tabs here: you can
            // see how much work is waiting without switching views.
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(if on { MUTE } else { FAINT }))
                    .child(if count > 0 { count.to_string() } else { String::new() }),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_tab(tab, cx)))
    }

    fn button(
        &self,
        id: &'static str,
        label: String,
        accent: bool,
        enabled: bool,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px_3()
            .py_1()
            .rounded(px(5.0))
            .text_sm()
            .border_1()
            .border_color(rgb(LINE))
            .when(accent && enabled, |d| {
                // The default button is the system blue one, as on any macOS
                // sheet -- it is not a brand color, it is a role.
                d.bg(rgb(SEL)).text_color(rgb(0xFFFFFF)).border_color(rgb(SEL))
            })
            .when(!accent && enabled, |d| {
                d.bg(rgb(0x333333)).text_color(rgb(INK)).hover(|h| h.bg(rgb(0x3E3E3E)))
            })
            .when(!enabled, |d| d.text_color(rgb(FAINT)))
            .when(enabled, |d| d.cursor_pointer())
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if enabled {
                    f(this, cx)
                }
            }))
    }

    /// One row. `me` is the entity handle so the click closure -- which only gets
    /// `&mut App`, not `&mut Context<Self>` -- can still mutate our state.
    fn row(&self, i: usize, me: &gpui::Entity<Self>) -> gpui::AnyElement {
        let Some(&ri) = self.rows.get(i) else {
            return div().into_any_element();
        };
        let Some(p) = self.snapshot.get(ri as usize) else {
            return div().into_any_element();
        };
        let sel = self.selected.contains(&ri);
        // Show a transition only when we actually know both ends of it. This must
        // NOT key off `p.outdated`: resnapshot clears that flag on the Outdated
        // tab (every row there is outdated by definition), which would suppress
        // the arrow on the one tab that exists to show it.
        let has_delta = !p.installed.is_empty() && !p.latest.is_empty() && p.latest != p.installed;
        // The Installed tab marks stale rows but `scan_installed` has no target
        // version to point at, so tint the version rather than draw an arrow to
        // nowhere -- otherwise staleness is invisible on that tab.
        let stale_only = p.outdated && !has_delta;
        let version = if p.installed.is_empty() {
            p.latest.clone()
        } else {
            p.installed.clone()
        };
        let latest = p.latest.clone();
        let name = p.name.clone();
        let kind = if p.cask { "cask" } else { "formula" };
        let desc = p.desc.clone();
        let pinned = p.pinned;
        let me = me.clone();
        div()
            .id(("row", i))
            .flex()
            .flex_row()
            .items_center()
            .h(px(24.0))
            .px_2()
            // One background, not three. Zebra striping plus a hover tint plus a
            // selection tint means a row can be in four visual states for two
            // bits of information.
            .when(sel, |d| d.bg(rgb(SEL)))
            .when(!sel, |d| d.hover(|h| h.bg(rgb(0x2E2E2E))))
            .cursor_pointer()
            .on_click(move |ev, _window, cx| {
                let additive = ev.modifiers().platform || ev.modifiers().shift;
                me.update(cx, |this, cx| {
                    // Arrows should carry on from the row you just clicked.
                    this.cursor = Some(i);
                    if !additive {
                        let only = this.selected.len() == 1 && this.selected.contains(&ri);
                        this.selected.clear();
                        if only {
                            cx.notify();
                            return;
                        }
                        this.selected.insert(ri);
                    } else if !this.selected.remove(&ri) {
                        this.selected.insert(ri);
                    }
                    cx.notify();
                });
            })
            // Package
            .child(cell(0).text_color(rgb(INK)).child(name))
            // Kind -- structural, so it stays quiet
            .child(cell(1).text_color(rgb(FAINT)).child(kind))
            // Version. The signature: an upgrade is a transition, so it reads as
            // one. Old version recedes, arrow is furniture, new version is the
            // only warm pixel on screen because it is the only thing to act on.
            .child(
                cell(2)
                    .font_family(MONO_FONT)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_color(rgb(if has_delta {
                                MUTE
                            } else if stale_only {
                                STALE
                            } else {
                                INK
                            }))
                            .child(version),
                    )
                    .when(has_delta, |d| {
                        d.child(div().text_color(rgb(FAINT)).child("\u{2192}"))
                            .child(div().text_color(rgb(STALE)).child(latest))
                    })
                    .when(pinned, |d| {
                        // Pinned packages are skipped by every action; say so
                        // rather than letting the user wonder why nothing ran.
                        d.child(div().text_color(rgb(FAINT)).child("pinned"))
                    }),
            )
            // Description
            .child(cell(3).text_color(rgb(MUTE)).child(desc))
            .into_any_element()
    }
}

impl Focusable for Kettle {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Kettle {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.rows.len();
        let n_out = self.outdated.len();
        let n_inst = self.installed.len();
        let n_cat = self.catalog.len();
        let busy = self.busy;
        let sel_n = self.selected.len();

        // The sidebar owns the counts now, so the status bar must not repeat
        // them -- and it must not restate the empty-state message sitting in
        // the middle of the same window. What is left is genuinely transient:
        // progress, selection size, and how much a search narrowed things.
        let status: SharedString = if !self.status.is_empty() {
            self.status.clone()
        } else if sel_n > 0 {
            format!("{sel_n} selected").into()
        } else if !self.query.is_empty() && !self.rows.is_empty() {
            format!("{shown} match{}", if shown == 1 { "" } else { "es" }).into()
        } else {
            "".into()
        };

        let verb = if self.tab == Tab::Browse { "Install" } else { "Upgrade" };
        // An empty table should say why it is empty. Staying silent while busy
        // avoids flashing "nothing here" during the initial scan.
        let empty: Option<String> = if !self.rows.is_empty() || busy {
            None
        } else if !self.query.is_empty() {
            Some(format!("No packages match \u{201C}{}\u{201D}", self.query))
        } else {
            Some(match self.tab {
                Tab::Outdated => "Everything is up to date".into(),
                Tab::Installed => "No packages installed".into(),
                // Browse is only empty if the catalog failed to load; the
                // Activity log carries the reason.
                Tab::Browse => "Catalog unavailable \u{2014} run brew update, then Refresh".into(),
            })
        };
        // One lock, one poison policy. Taking it twice let a poisoned mutex show
        // log lines while hiding the control that toggles them.
        let (log_tail, log_n): (Vec<String>, usize) = {
            let g = self.log.lock().unwrap_or_else(|e| e.into_inner());
            (g.iter().rev().take(200).rev().cloned().collect(), g.len())
        };
        let log_open = self.log_open;

        div()
            .key_context("Kettle")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &Primary, _, cx| this.primary(cx)))
            .on_action(cx.listener(|t, _: &ViewOutdated, _, cx| t.select_tab(Tab::Outdated, cx)))
            .on_action(cx.listener(|t, _: &ViewInstalled, _, cx| t.select_tab(Tab::Installed, cx)))
            .on_action(cx.listener(|t, _: &ViewBrowse, _, cx| t.select_tab(Tab::Browse, cx)))
            .on_action(cx.listener(|this, _: &ClearSearch, _, cx| {
                this.query.clear();
                this.refilter();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &UpgradeAll, _, cx| {
                let items = this.all_outdated_items();
                this.run_brew("upgrade", items, cx);
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(INK))
            .font_family(UI_FONT)
            // ---- toolbar. No app name: the titlebar already says Kettle, and
            // repeating it in-window is branding a utility does not need.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(CHROME))
                    // Search. GPUI has no native text field; this is a minimal
                    // one -- no selection, no IME. The caret is drawn only while
                    // there is text, so an empty field reads as a placeholder
                    // rather than a fake focused input.
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(120.0))
                            .max_w(px(320.0))
                            .px_2()
                            .py_1()
                            .rounded(px(5.0))
                            .bg(rgb(0x1A1A1A))
                            .border_1()
                            .border_color(rgb(LINE))
                            .text_sm()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .when(self.query.is_empty(), |d| {
                                d.text_color(rgb(FAINT)).child("Search")
                            })
                            .when(!self.query.is_empty(), |d| {
                                d.text_color(rgb(INK))
                                    .child(format!("{}\u{258F}", self.query))
                            }),
                    )
                    .flex_shrink_0()
                    .child(self.button("refresh", "Refresh".into(), false, !busy, cx, |t, cx| {
                        t.refresh(cx)
                    }))
                    .child(self.button(
                        "primary",
                        if sel_n > 0 {
                            format!("{verb} {sel_n}")
                        } else {
                            format!("{verb} Selected")
                        },
                        true,
                        !busy && sel_n > 0,
                        cx,
                        move |t, cx| t.primary(cx),
                    ))
                    .child(self.button(
                        "upall",
                        "Upgrade All".into(),
                        false,
                        !busy && n_out > 0,
                        cx,
                        |t, cx| {
                            let items = t.all_outdated_items();
                            t.run_brew("upgrade", items, cx);
                        },
                    )),
            )
            // ---- body: source list on the left, table on the right. This is
            // the macOS idiom for this shape of app (Font Book, Mail), and it
            // buys somewhere honest to put the per-view counts.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .overflow_hidden()
                    // ---- sidebar
                    .child(
                        div()
                            .w(px(180.0))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .pt_2()
                            .bg(rgb(0x252525))
                            .border_r_1()
                            .border_color(rgb(LINE))
                            .child(
                                div()
                                    .px_4()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(rgb(FAINT))
                                    .child("Library"),
                            )
                            .child(self.sidebar_item(Tab::Outdated, "Outdated", n_out, cx))
                            .child(self.sidebar_item(Tab::Installed, "Installed", n_inst, cx))
                            .child(self.sidebar_item(Tab::Browse, "Browse", n_cat, cx)),
                    )
                    // ---- content
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_hidden()
                            // column header, sentence case rather than SHOUTED CAPS
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .px_2()
                                    .py_1()
                                    .bg(rgb(CHROME))
                                    .border_b_1()
                                    .border_color(rgb(LINE))
                                    .children(COLS.iter().enumerate().map(|(i, (name, _))| {
                                        cell(i).text_color(rgb(MUTE)).child(*name)
                                    })),
                            )
                            .child(match empty {
                                Some(msg) => div()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(div().text_sm().text_color(rgb(FAINT)).child(msg))
                                    .into_any_element(),
                                None => div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .child(
                                        uniform_list(
                                            "rows",
                                            self.rows.len(),
                                            cx.processor(
                                                move |this, range: Range<usize>, _, cx| {
                                                    let me = cx.entity();
                                                    range
                                                        .map(|i| this.row(i, &me))
                                                        .collect::<Vec<_>>()
                                                },
                                            ),
                                        )
                                        .size_full()
                                        .track_scroll(self.scroll.clone()),
                                    )
                                    .into_any_element(),
                            })
                            // ---- activity log, disclosed rather than always-on.
                            // Raw brew output is diagnostic; keeping it
                            // permanently on screen is what makes a tool look
                            // like a demo of itself.
                            .when(log_open, |d| {
                                d.child(
                                    div()
                                        .h(px(180.0))
                                        .border_t_1()
                                        .border_color(rgb(LINE))
                                        .bg(rgb(0x181818))
                                        .p_2()
                                        .overflow_hidden()
                                        .flex()
                                        .flex_col()
                                        .justify_end()
                                        .children(log_tail.into_iter().map(|l| {
                                            let c = if l.starts_with("Failed")
                                                || l.starts_with("Couldn't")
                                            {
                                                rgb(FAIL)
                                            } else if l.starts_with("Finished") {
                                                rgb(INK)
                                            } else if l.starts_with('$') {
                                                rgb(MUTE)
                                            } else {
                                                rgb(FAINT)
                                            };
                                            div()
                                                .text_xs()
                                                .font_family(MONO_FONT)
                                                .text_color(c)
                                                .whitespace_nowrap()
                                                .overflow_hidden()
                                                .child(l)
                                        })),
                                )
                            }),
                    ),
            )
            // ---- status bar, doubling as the log's disclosure control
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(CHROME))
                    .text_xs()
                    .child(div().flex_1().text_color(rgb(MUTE)).child(status))
                    .when(log_n > 0, |d| {
                        d.child(
                            div()
                                .id("log-toggle")
                                .cursor_pointer()
                                .text_color(rgb(MUTE))
                                .hover(|h| h.text_color(rgb(INK)))
                                .child(if log_open {
                                    "Activity \u{25BE}"
                                } else {
                                    "Activity \u{25B8}"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.log_open = !this.log_open;
                                    cx.notify();
                                })),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{brew::Pkg, next_cursor, progressive, rank, rank_rows};

    fn pkg(name: &str, desc: &str) -> Pkg {
        let mut p = Pkg {
            name: name.into(),
            desc: desc.into(),
            ..Default::default()
        };
        super::brew::fold_all(std::slice::from_mut(&mut p));
        p
    }

    /// Browse is the whole catalog with no query typed. It once returned an
    /// empty list here, which made the tab impossible to browse and looked
    /// exactly like a catalog that had failed to load.
    #[test]
    fn empty_query_lists_every_package() {
        let pkgs = [pkg("ripgrep", "search tool"), pkg("bat", "cat clone")];
        assert_eq!(rank_rows(&[], &pkgs), vec![0, 1]);
        assert_eq!(rank_rows(&[], &[]), Vec::<u32>::new());
    }

    #[test]
    fn query_narrows_to_matches() {
        let pkgs = [
            pkg("ripgrep", "search tool"),
            pkg("bat", "cat clone"),
            pkg("fd", "find alternative"),
        ];
        assert_eq!(rank_rows(&rank::fold("ripg"), &pkgs), vec![0]);
        // A description-only hit still matches, so search is not name-only.
        assert_eq!(rank_rows(&rank::fold("clone"), &pkgs), vec![1]);
        assert!(rank_rows(&rank::fold("zzzznope"), &pkgs).is_empty());
    }

    /// Both verbs go through the status bar, and one of them has a silent e.
    #[test]
    fn progressive_drops_the_silent_e() {
        assert_eq!(progressive("upgrade"), "Upgrading");
        assert_eq!(progressive("install"), "Installing");
    }

    #[test]
    fn arrows_clamp_and_never_wrap() {
        // Down from the last row stays put; Up from the first stays put. Wrapping
        // would silently jump the viewport across a 16k-row catalog.
        assert_eq!(next_cursor(Some(4), 1, 5), Some(4));
        assert_eq!(next_cursor(Some(0), -1, 5), Some(0));
        assert_eq!(next_cursor(Some(2), 1, 5), Some(3));
        assert_eq!(next_cursor(Some(2), -1, 5), Some(1));
    }

    #[test]
    fn first_press_enters_from_the_matching_end() {
        assert_eq!(next_cursor(None, 1, 5), Some(0), "Down starts at the top");
        assert_eq!(next_cursor(None, -1, 5), Some(4), "Up starts at the bottom");
    }

    /// Browse opens with zero rows, so arrows there must not index into `rows`.
    #[test]
    fn empty_list_has_no_cursor() {
        assert_eq!(next_cursor(None, 1, 0), None);
        assert_eq!(next_cursor(Some(3), -1, 0), None);
    }

    /// A stale cursor past the end of a freshly-filtered list must be pulled
    /// back in bounds, not used as-is.
    #[test]
    fn stale_cursor_is_clamped_into_the_new_list() {
        assert_eq!(next_cursor(Some(900), 1, 3), Some(2));
    }
}

fn main() {
    let Some(prefix) = brew::detect_prefix() else {
        eprintln!("Homebrew not found in /opt/homebrew or /usr/local");
        std::process::exit(1);
    };

    Application::new().run(move |cx: &mut App| {
        cx.activate(true);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-q", Quit, None),
            gpui::KeyBinding::new("cmd-r", Refresh, Some("Kettle")),
            gpui::KeyBinding::new("cmd-u", UpgradeAll, Some("Kettle")),
            gpui::KeyBinding::new("cmd-k", ClearSearch, Some("Kettle")),
            // on_key deliberately passes "enter" through so it lands here.
            gpui::KeyBinding::new("enter", Primary, Some("Kettle")),
            // Cmd-1/2/3 for views, as in most macOS apps with a source list.
            gpui::KeyBinding::new("cmd-1", ViewOutdated, Some("Kettle")),
            gpui::KeyBinding::new("cmd-2", ViewInstalled, Some("Kettle")),
            gpui::KeyBinding::new("cmd-3", ViewBrowse, Some("Kettle")),
        ]);
        cx.set_menus(vec![Menu {
            name: "Kettle".into(),
            items: vec![
                MenuItem::action("Outdated", ViewOutdated),
                MenuItem::action("Installed", ViewInstalled),
                MenuItem::action("Browse", ViewBrowse),
                MenuItem::separator(),
                MenuItem::action("Refresh", Refresh),
                MenuItem::action("Upgrade All", UpgradeAll),
                MenuItem::separator(),
                MenuItem::action("Quit", Quit),
            ],
        }]);

        // Wide enough that the sidebar (180) plus every column (1086) fits
        // without clipping Description off the right edge.
        let bounds = Bounds::centered(None, size(px(1340.0), px(820.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Kettle".into()),
                        appears_transparent: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| Kettle::new(prefix.clone(), cx)),
            )
            .unwrap();

        // Focus the root so typing goes to the search box immediately.
        window
            .update(cx, |view, window, cx| {
                window.focus(&view.focus_handle(cx));
            })
            .ok();
    });
}
