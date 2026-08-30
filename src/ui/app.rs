use super::{
    text_input::{self, SearchChanged, SearchInput},
    theme::Theme,
};
use crate::{
    application::{
        AppController, AuthFailure, AuthState, ClickModifiers, DevicePrompt, GitHubUser, LogEvent,
        LogLevel, LogSource, OperationState, RefreshStage, View,
    },
    domain::{BrewAction, PackageId, UpdateState},
    infrastructure::{
        InfrastructureError,
        github::{
            GitHubTransport, MacKeychain, OAuthTransport, PollResult, TokenStore,
            ensure_poll_allowed, validate_verification_uri,
        },
        homebrew::{
            HomebrewBackend, ProcessEvent, ProcessStream, SystemHomebrew, detect_prefix,
            execute_plans, plan_commands,
        },
    },
};
use gpui::{
    App, Application, Bounds, Context, Entity, FocusHandle, Focusable, KeyDownEvent, Menu,
    MenuItem, PromptLevel, ScrollStrategy, SharedString, UniformListScrollHandle, Window,
    WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size, uniform_list,
};
#[path = "views.rs"]
mod views;

use std::{
    borrow::Cow,
    ops::Range,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

actions!(
    kettle,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        About,
        Help,
        Refresh,
        UpgradeAll,
        CancelOperation,
        Primary,
        ClearSearch,
        SelectAllPackages,
        MoveUp,
        MoveDown,
        ExtendUp,
        ExtendDown,
        FocusNext,
        FocusPrevious,
        ViewOutdated,
        ViewInstalled,
        ViewBrowse,
        ViewSettings,
        Minimize,
        Zoom
    ]
);

const COLUMNS: [(&str, f32); 4] = [
    ("Package", 220.),
    ("Kind", 64.),
    ("Version", 260.),
    ("Description", 0.),
];
const APP_FONT: &str = "IBM Plex Mono";
const APP_FONT_DATA: &[u8] = include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf");

pub fn run() -> Result<(), InfrastructureError> {
    let launched = Instant::now();
    let prefix = detect_prefix().ok_or(InfrastructureError::HomebrewUnavailable)?;
    let backend: Arc<dyn HomebrewBackend> = Arc::new(SystemHomebrew::new(prefix)?);
    let transport: Arc<dyn OAuthTransport> = Arc::new(GitHubTransport::new()?);
    let keychain: Arc<dyn TokenStore> = Arc::new(MacKeychain);

    Application::new().run(move |cx| {
        configure(cx);
        let bounds = Bounds::centered(None, size(px(1340.), px(820.)), cx);
        let (backend, transport, keychain) = (backend.clone(), transport.clone(), keychain.clone());
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Kettle".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    cx.new(|cx| Kettle::new(backend, transport, keychain, window, cx))
                },
            )
            .expect("Kettle window must open");
        window
            .update(cx, |view, window, cx| {
                window.focus(&view.search.read(cx).focus_handle(cx));
            })
            .ok();
        cx.activate(true);
        if std::env::var_os("KETTLE_PERF_LOG").is_some() {
            eprintln!(
                "kettle_perf window_ready_ms={}",
                launched.elapsed().as_millis()
            );
        }
    });
    Ok(())
}

fn configure(cx: &mut App) {
    cx.text_system()
        .add_fonts(vec![Cow::Borrowed(APP_FONT_DATA)])
        .expect("bundled IBM Plex Mono font must load");
    text_input::bind_keys(cx);
    cx.bind_keys([
        gpui::KeyBinding::new("cmd-q", Quit, None),
        gpui::KeyBinding::new("cmd-r", Refresh, Some("Kettle")),
        gpui::KeyBinding::new("cmd-u", UpgradeAll, Some("Kettle")),
        gpui::KeyBinding::new("cmd-.", CancelOperation, Some("Kettle")),
        gpui::KeyBinding::new("cmd-k", ClearSearch, Some("Kettle")),
        gpui::KeyBinding::new("cmd-a", SelectAllPackages, Some("Kettle")),
        gpui::KeyBinding::new("enter", Primary, Some("Kettle")),
        gpui::KeyBinding::new("up", MoveUp, Some("Kettle")),
        gpui::KeyBinding::new("down", MoveDown, Some("Kettle")),
        gpui::KeyBinding::new("shift-up", ExtendUp, Some("Kettle")),
        gpui::KeyBinding::new("shift-down", ExtendDown, Some("Kettle")),
        gpui::KeyBinding::new("tab", FocusNext, None),
        gpui::KeyBinding::new("shift-tab", FocusPrevious, None),
        gpui::KeyBinding::new("cmd-1", ViewOutdated, Some("Kettle")),
        gpui::KeyBinding::new("cmd-2", ViewInstalled, Some("Kettle")),
        gpui::KeyBinding::new("cmd-3", ViewBrowse, Some("Kettle")),
        gpui::KeyBinding::new("cmd-,", ViewSettings, Some("Kettle")),
        gpui::KeyBinding::new("cmd-m", Minimize, Some("Kettle")),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.set_menus(vec![
        Menu {
            name: "Kettle".into(),
            items: vec![
                MenuItem::action("About Kettle", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", ViewSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Kettle", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Kettle", Quit),
            ],
        },
        text_input::edit_menu(),
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Outdated", ViewOutdated),
                MenuItem::action("Installed", ViewInstalled),
                MenuItem::action("Browse", ViewBrowse),
                MenuItem::separator(),
                MenuItem::action("Refresh", Refresh),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
            ],
        },
        Menu {
            name: "Help".into(),
            items: vec![MenuItem::action("Kettle Help", Help)],
        },
    ]);
}

struct Kettle {
    backend: Arc<dyn HomebrewBackend>,
    transport: Arc<dyn OAuthTransport>,
    keychain: Arc<dyn TokenStore>,
    controller: AppController,
    search: Entity<SearchInput>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    pending_logs: Arc<Mutex<Vec<LogEvent>>>,
    log_open: bool,
}

impl Kettle {
    fn new(
        backend: Arc<dyn HomebrewBackend>,
        transport: Arc<dyn OAuthTransport>,
        keychain: Arc<dyn TokenStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(SearchInput::new);
        cx.subscribe(&search, |this, _, changed: &SearchChanged, cx| {
            this.controller.state.query.clone_from(&changed.0);
            this.controller.state.refilter();
            cx.notify();
        })
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        let mut kettle = Self {
            backend,
            transport,
            keychain,
            controller: AppController::default(),
            search,
            focus: cx.focus_handle().tab_index(1).tab_stop(true),
            scroll: UniformListScrollHandle::new(),
            pending_logs: Arc::new(Mutex::new(Vec::new())),
            log_open: false,
        };
        kettle.refresh(cx);
        kettle.restore_session(cx);
        kettle
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.controller.state.operation,
            OperationState::Mutating { .. }
        ) {
            return;
        }
        let (generation, cancel) = self.controller.begin_refresh();
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            let installed = cx
                .background_spawn({
                    let backend = backend.clone();
                    async move { backend.installed() }
                })
                .await;
            perf_stage("installed", started);
            if cancel.is_cancelled() {
                return;
            }
            this.update(cx, |this, cx| {
                if this.controller.refresh_is_current(generation) {
                    match &installed {
                        Ok(packages) => this.controller.state.packages.preview_installed(packages),
                        Err(error) => this.log_error(LogSource::Homebrew, present_error(error)),
                    }
                    this.controller.state.operation =
                        OperationState::Refreshing(RefreshStage::Catalog);
                    this.controller.state.refilter();
                    cx.notify();
                }
            })
            .ok();

            let catalog = cx
                .background_spawn({
                    let backend = backend.clone();
                    let cancel = cancel.clone();
                    async move { backend.catalog(&|| cancel.is_cancelled()) }
                })
                .await;
            perf_stage("catalog", started);
            if cancel.is_cancelled() {
                return;
            }
            this.update(cx, |this, cx| {
                if this.controller.refresh_is_current(generation) {
                    match &catalog {
                        Ok(packages) => this.controller.state.packages.preview_catalog(packages),
                        Err(error) => this.log_error(LogSource::Catalog, present_error(error)),
                    }
                    this.controller.state.operation =
                        OperationState::Refreshing(RefreshStage::Outdated);
                    this.controller.state.refilter();
                    cx.notify();
                }
            })
            .ok();

            let outdated = cx
                .background_spawn({
                    let backend = backend.clone();
                    let cancel = cancel.clone();
                    async move { backend.outdated(&|| cancel.is_cancelled()) }
                })
                .await;
            perf_stage("outdated", started);
            if cancel.is_cancelled() {
                return;
            }
            this.update(cx, |this, cx| {
                if !this.controller.refresh_is_current(generation) {
                    return;
                }
                match (installed, catalog, outdated) {
                    (Ok(installed), Ok(catalog), Ok(outdated)) => this
                        .controller
                        .state
                        .packages
                        .replace(installed, catalog, outdated),
                    (_, _, Ok(outdated)) => {
                        this.controller.state.packages.preview_outdated(&outdated)
                    }
                    (_, _, Err(error)) => {
                        this.log_error(LogSource::Homebrew, present_error(&error))
                    }
                }
                this.controller.finish_refresh(generation);
                this.controller.state.refilter();
                cx.notify();
                perf_stage("refresh_complete", started);
            })
            .ok();
        })
        .detach();
    }

    fn restore_session(&mut self, cx: &mut Context<Self>) {
        let keychain = self.keychain.clone();
        let transport = self.transport.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let Some(token) = keychain.load()? else {
                        return Ok(None);
                    };
                    transport.whoami(&token).map(Some)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.controller.state.auth != AuthState::SignedOut {
                    return;
                }
                match result {
                    Ok(Some(login)) => {
                        this.controller.state.auth = AuthState::SignedIn(GitHubUser(login));
                    }
                    Ok(None) => {}
                    Err(error) => {
                        this.controller.state.auth = AuthState::Failed(auth_failure(&error));
                        this.log_error(LogSource::GitHub, present_error(&error));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        let cancel = self.controller.begin_authentication();
        let transport = self.transport.clone();
        let keychain = self.keychain.clone();
        cx.spawn(async move |this, cx| {
            let authorization = cx
                .background_spawn({
                    let transport = transport.clone();
                    async move { transport.request_device_code() }
                })
                .await;
            if cancel.is_cancelled() {
                return;
            }
            let authorization = match authorization {
                Ok(value) => Arc::new(value),
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.controller.state.auth = AuthState::Failed(auth_failure(&error));
                        this.controller.finish_authentication();
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            if let Err(error) = validate_verification_uri(&authorization.verification_uri) {
                this.update(cx, |this, cx| {
                    this.controller.state.auth = AuthState::Failed(auth_failure(&error));
                    this.controller.finish_authentication();
                    this.log_error(LogSource::GitHub, present_error(&error));
                    cx.notify();
                })
                .ok();
                return;
            }
            let prompt = DevicePrompt {
                user_code: authorization.user_code.clone(),
                verification_uri: authorization.verification_uri.clone(),
                expires_in_seconds: authorization.expires_in.as_secs(),
            };
            this.update(cx, |this, cx| {
                if !cancel.is_cancelled() {
                    this.controller.state.auth = AuthState::AwaitingApproval(prompt);
                    cx.notify();
                }
            })
            .ok();
            if cancel.is_cancelled() {
                return;
            }
            let verification_uri = authorization.verification_uri.clone();
            let open_result = cx
                .background_spawn(async move {
                    Command::new("/usr/bin/open").arg(verification_uri).status()
                })
                .await;
            if !matches!(open_result, Ok(status) if status.success()) {
                this.update(cx, |this, cx| {
                    this.log_error(
                        LogSource::GitHub,
                        "Could not open the browser; use the verification URL shown in Settings."
                            .to_owned(),
                    );
                    cx.notify();
                })
                .ok();
            }

            let started = Instant::now();
            let mut interval = authorization.interval;
            loop {
                if let Err(error) = ensure_poll_allowed(
                    started.elapsed(),
                    authorization.expires_in,
                    cancel.is_cancelled(),
                ) {
                    if !cancel.is_cancelled() {
                        this.update(cx, |this, cx| {
                            this.controller.state.auth = AuthState::Failed(auth_failure(&error));
                            this.controller.finish_authentication();
                            cx.notify();
                        })
                        .ok();
                    }
                    return;
                }
                cx.background_executor().timer(interval).await;
                let poll = cx
                    .background_spawn({
                        let transport = transport.clone();
                        let authorization = authorization.clone();
                        async move { transport.poll(&authorization) }
                    })
                    .await;
                if cancel.is_cancelled() {
                    return;
                }
                match poll {
                    Ok(PollResult::Pending) => {}
                    Ok(PollResult::SlowDown(extra)) => interval += extra,
                    Ok(PollResult::Token(token)) => {
                        let result = cx
                            .background_spawn({
                                let transport = transport.clone();
                                let keychain = keychain.clone();
                                let poll_cancel = cancel.clone();
                                async move {
                                    if poll_cancel.is_cancelled() {
                                        return Err(InfrastructureError::Cancelled);
                                    }
                                    keychain.store(&token)?;
                                    if poll_cancel.is_cancelled() {
                                        keychain.delete()?;
                                        return Err(InfrastructureError::Cancelled);
                                    }
                                    let result = transport.whoami(&token);
                                    if poll_cancel.is_cancelled() {
                                        keychain.delete()?;
                                        return Err(InfrastructureError::Cancelled);
                                    }
                                    result
                                }
                            })
                            .await;
                        this.update(cx, |this, cx| {
                            if !cancel.is_cancelled() {
                                this.controller.state.auth = match result {
                                    Ok(login) => AuthState::SignedIn(GitHubUser(login)),
                                    Err(error) => AuthState::Failed(auth_failure(&error)),
                                };
                                this.controller.finish_authentication();
                                cx.notify();
                            }
                        })
                        .ok();
                        return;
                    }
                    Err(error) => {
                        this.update(cx, |this, cx| {
                            if !cancel.is_cancelled() {
                                this.controller.state.auth =
                                    AuthState::Failed(auth_failure(&error));
                                this.controller.finish_authentication();
                                cx.notify();
                            }
                        })
                        .ok();
                        return;
                    }
                }
            }
        })
        .detach();
    }

    fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.controller.cancel_authentication();
        let keychain = self.keychain.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { keychain.delete() }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.controller.state.auth = AuthState::SignedOut,
                    Err(error) => {
                        this.log_open = true;
                        this.log_error(LogSource::GitHub, present_error(&error));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn primary(&mut self, cx: &mut Context<Self>) {
        let action = if self.controller.state.view == View::Browse {
            BrewAction::Install
        } else {
            BrewAction::Upgrade
        };
        self.mutate(action, self.controller.state.selected_targets(), cx);
    }

    fn mutate(&mut self, action: BrewAction, targets: Vec<PackageId>, cx: &mut Context<Self>) {
        let Some(cancel) = self.controller.begin_mutation(action, targets.len()) else {
            let message = if self.controller.state.operation == OperationState::Idle {
                "No eligible packages selected."
            } else {
                "Another Homebrew operation is already running."
            };
            self.controller.state.push_log(LogEvent {
                level: LogLevel::Info,
                source: LogSource::Application,
                message: message.to_owned(),
            });
            self.log_open = true;
            cx.notify();
            return;
        };
        let plans = plan_commands(action, targets);
        let backend = self.backend.clone();
        let pending = self.pending_logs.clone();
        let done = Arc::new(AtomicBool::new(false));
        self.log_open = true;
        self.flush_logs_until(done.clone(), cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn({
                    let done = done.clone();
                    async move {
                        let mut on_plan = |plan: &crate::infrastructure::homebrew::CommandPlan| {
                            push_pending(
                                &pending,
                                LogEvent {
                                    level: LogLevel::Trace,
                                    source: LogSource::Homebrew,
                                    message: format!("brew {}", display_args(&plan.arguments())),
                                },
                            );
                        };
                        let mut on_event = |event: ProcessEvent| {
                            push_pending(
                                &pending,
                                LogEvent {
                                    level: match event.stream {
                                        ProcessStream::Stdout => LogLevel::Trace,
                                        ProcessStream::Stderr => LogLevel::Info,
                                    },
                                    source: LogSource::Homebrew,
                                    message: event.message,
                                },
                            );
                        };
                        let result = execute_plans(
                            backend.as_ref(),
                            &plans,
                            &|| cancel.is_cancelled(),
                            &mut on_plan,
                            &mut on_event,
                        );
                        done.store(true, Ordering::Release);
                        result
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.controller.state.push_log(LogEvent {
                        level: LogLevel::Success,
                        source: LogSource::Application,
                        message: format!("Packages {}.", action.completed().to_ascii_lowercase()),
                    }),
                    Err(InfrastructureError::Cancelled) => {}
                    Err(error) => this.log_error(LogSource::Homebrew, present_error(&error)),
                }
                this.controller.finish_mutation();
                cx.notify();
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    fn flush_logs_until(&self, done: Arc<AtomicBool>, cx: &mut Context<Self>) {
        let pending = self.pending_logs.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let batch = drain_pending(&pending);
                let finished = done.load(Ordering::Acquire);
                if this
                    .update(cx, |this, cx| {
                        for event in batch {
                            this.controller.state.push_log(event);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                if finished
                    && pending
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .is_empty()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn log_error(&mut self, source: LogSource, message: String) {
        self.controller.state.push_log(LogEvent {
            level: LogLevel::Error,
            source,
            message,
        });
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.controller.state.set_view(view);
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        if let Some(index) = self.controller.state.selection.move_cursor(delta, extend) {
            self.scroll.scroll_to_item(index, ScrollStrategy::Top);
        }
        cx.notify();
    }
}

fn perf_stage(stage: &str, started: Instant) {
    if std::env::var_os("KETTLE_PERF_LOG").is_some() {
        eprintln!(
            "kettle_perf {stage}_elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }
}

impl Focusable for Kettle {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn display_args(arguments: &[std::ffi::OsString]) -> String {
    arguments
        .iter()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

fn push_pending(pending: &Mutex<Vec<LogEvent>>, event: LogEvent) {
    pending
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .push(event);
}

fn drain_pending(pending: &Mutex<Vec<LogEvent>>) -> Vec<LogEvent> {
    std::mem::take(&mut *pending.lock().unwrap_or_else(|poison| poison.into_inner()))
}

fn log_color(level: LogLevel, theme: Theme) -> u32 {
    match level {
        LogLevel::Trace => theme.faint,
        LogLevel::Info => theme.muted,
        LogLevel::Success => theme.text,
        LogLevel::Error => theme.failure,
    }
}

fn present_error(error: &InfrastructureError) -> String {
    match error {
        InfrastructureError::NetworkTimeout => {
            "GitHub did not respond before the request deadline.".to_owned()
        }
        InfrastructureError::OAuthDenied(description) => {
            format!("GitHub sign-in was denied: {}", safe_message(description))
        }
        InfrastructureError::NetworkTransport(_) => {
            "GitHub could not be reached because the network request failed.".to_owned()
        }
        InfrastructureError::OAuthProtocol(description) => {
            format!("GitHub sign-in failed: {}", safe_message(description))
        }
        InfrastructureError::Keychain(_) => {
            "The OAuth token could not be updated in the macOS Keychain.".to_owned()
        }
        InfrastructureError::OAuthExpired => "The GitHub device code expired.".to_owned(),
        InfrastructureError::Cancelled => "The operation was cancelled.".to_owned(),
        _ => error.to_string(),
    }
}

fn safe_message(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control())
        .take(240)
        .collect()
}

fn auth_failure(error: &InfrastructureError) -> AuthFailure {
    match error {
        InfrastructureError::OAuthDenied(description) => {
            AuthFailure::Denied(safe_message(description))
        }
        InfrastructureError::OAuthExpired => AuthFailure::Expired,
        InfrastructureError::NetworkTimeout | InfrastructureError::NetworkTransport(_) => {
            AuthFailure::Network(present_error(error))
        }
        InfrastructureError::Keychain(_) => AuthFailure::Keychain(present_error(error)),
        _ => AuthFailure::Protocol(present_error(error)),
    }
}

fn auth_failure_message(error: &AuthFailure) -> String {
    match error {
        AuthFailure::Denied(message)
        | AuthFailure::Network(message)
        | AuthFailure::Keychain(message)
        | AuthFailure::Protocol(message) => message.clone(),
        AuthFailure::Expired => "The device code expired".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_font_is_a_nonempty_truetype_font() {
        assert!(APP_FONT_DATA.len() > 100_000);
        assert_eq!(&APP_FONT_DATA[..4], &[0, 1, 0, 0]);
    }
}
