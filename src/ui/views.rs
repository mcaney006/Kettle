use super::*;
use std::rc::Rc;

type ButtonAction = dyn Fn(&mut Kettle, &mut Context<Kettle>);

impl Kettle {
    #[allow(clippy::too_many_arguments)]
    fn button(
        &self,
        id: &'static str,
        label: String,
        accent: bool,
        enabled: bool,
        tab_index: isize,
        theme: Theme,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> gpui::AnyElement {
        let action: Rc<ButtonAction> = Rc::new(action);
        div()
            .id(id)
            .tab_index(tab_index)
            .px_3()
            .py_1()
            .rounded(px(5.))
            .text_sm()
            .border_1()
            .border_color(rgb(theme.line))
            .when(accent && enabled, |element| {
                element
                    .bg(rgb(theme.selection))
                    .text_color(rgb(0xffffff))
                    .border_color(rgb(theme.selection))
            })
            .when(!accent && enabled, |element| {
                element
                    .text_color(rgb(theme.text))
                    .hover(|style| style.bg(rgb(theme.line)))
            })
            .when(!enabled, |element| element.text_color(rgb(theme.faint)))
            .when(enabled, |element| element.cursor_pointer())
            .focus(|style| style.border_2().border_color(rgb(theme.selection)))
            .child(label)
            .on_click({
                let action = action.clone();
                cx.listener(move |this, _, _, cx| {
                    if enabled {
                        action(this, cx);
                    }
                })
            })
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if enabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    action(this, cx);
                }
            }))
            .into_any_element()
    }

    fn sidebar(
        &self,
        view: View,
        label: &'static str,
        count: usize,
        tab_index: isize,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active = self.controller.state.view == view;
        div()
            .id(label)
            .tab_index(tab_index)
            .flex()
            .items_center()
            .mx_2()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .text_sm()
            .cursor_pointer()
            .when(active, |element| {
                element.bg(rgb(theme.line)).text_color(rgb(theme.text))
            })
            .when(!active, |element| {
                element
                    .text_color(rgb(theme.muted))
                    .hover(|style| style.bg(rgb(theme.chrome)))
            })
            .focus(|style| style.border_2().border_color(rgb(theme.selection)))
            .child(div().flex_1().child(label))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme.faint))
                    .child(if count > 0 {
                        count.to_string()
                    } else {
                        String::new()
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_view(view, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.set_view(view, cx);
                }
            }))
            .into_any_element()
    }

    fn package_row(&self, index: usize, entity: &Entity<Self>, theme: Theme) -> gpui::AnyElement {
        let state = &self.controller.state;
        let Some(id) = state.selection.visible().get(index) else {
            return div().into_any_element();
        };
        let Some(package) = state.packages.package(id) else {
            return div().into_any_element();
        };
        let selected = state.selection.selected().contains(id);
        let id = id.clone();
        let name = SharedString::new(id.name().shared());
        let kind = id.kind().label();
        let description = SharedString::new(package.shared_description().unwrap_or_default());
        let installed = package
            .installed_version()
            .map(|version| SharedString::new(version.shared()));
        let latest = package
            .latest_version()
            .map(|version| SharedString::new(version.shared()));
        let available = package.update_state() == UpdateState::UpdateAvailable;
        let pinned = package.is_pinned();
        let entity = entity.clone();
        div()
            .id(("row", index))
            .flex()
            .items_center()
            .h(px(24.))
            .px_2()
            .when(selected, |element| element.bg(rgb(theme.selection)))
            .when(!selected, |element| {
                element.hover(|style| style.bg(rgb(theme.chrome)))
            })
            .cursor_pointer()
            .on_click(move |event, window, cx| {
                entity.update(cx, |this, cx| {
                    window.focus(&this.focus);
                    this.controller.state.selection.click(
                        index,
                        ClickModifiers {
                            command: event.modifiers().platform,
                            shift: event.modifiers().shift,
                        },
                    );
                    cx.notify();
                });
            })
            .child(cell(0).text_color(rgb(theme.text)).child(name))
            .child(cell(1).text_color(rgb(theme.faint)).child(kind))
            .child(
                cell(2)
                    .font_family(MONO_FONT)
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_color(rgb(if available { theme.muted } else { theme.text }))
                            .child(
                                installed
                                    .clone()
                                    .or_else(|| latest.clone())
                                    .unwrap_or_else(|| SharedString::new_static("unknown")),
                            ),
                    )
                    .when(
                        available && installed.is_some() && latest.is_some(),
                        |element| {
                            element
                                .child(div().text_color(rgb(theme.faint)).child("→"))
                                .child(
                                    div()
                                        .text_color(rgb(theme.stale))
                                        .child(latest.clone().unwrap_or_default()),
                                )
                        },
                    )
                    .when(pinned, |element| {
                        element.child(div().text_color(rgb(theme.faint)).child("pinned"))
                    }),
            )
            .child(cell(3).text_color(rgb(theme.muted)).child(description))
            .into_any_element()
    }

    fn settings(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let auth = &self.controller.state.auth;
        let status = match auth {
            AuthState::SignedOut => "Not signed in".to_owned(),
            AuthState::RequestingDeviceCode => "Contacting GitHub…".to_owned(),
            AuthState::AwaitingApproval(_) => "Waiting for browser approval".to_owned(),
            AuthState::SignedIn(GitHubUser(login)) => format!("Signed in as {login}"),
            AuthState::Failed(error) => auth_failure_message(error),
        };
        let mut panel = div()
            .flex_1()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .child(section("GitHub", theme))
            .child(setting("Status", status, theme))
            .child(setting(
                "Access",
                "read:user identifies the account; Kettle does not read repositories".to_owned(),
                theme,
            ));
        if let AuthState::AwaitingApproval(prompt) = auth {
            let code = prompt.user_code.clone();
            panel = panel
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_2xl()
                        .text_color(rgb(theme.stale))
                        .child(prompt.user_code.clone()),
                )
                .child(self.button(
                    "copy-device-code",
                    "Copy code".to_owned(),
                    false,
                    true,
                    20,
                    theme,
                    cx,
                    move |_, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(code.clone()));
                    },
                ));
        }
        panel = match auth {
            AuthState::SignedIn(_) => panel.child(self.button(
                "sign-out",
                "Sign out".to_owned(),
                false,
                true,
                21,
                theme,
                cx,
                |this, cx| this.sign_out(cx),
            )),
            AuthState::RequestingDeviceCode | AuthState::AwaitingApproval(_) => {
                panel.child(self.button(
                    "cancel-auth",
                    "Cancel sign-in".to_owned(),
                    false,
                    true,
                    21,
                    theme,
                    cx,
                    |this, cx| {
                        this.controller.cancel_authentication();
                        cx.notify();
                    },
                ))
            }
            _ => panel.child(self.button(
                "sign-in",
                "Sign in with GitHub".to_owned(),
                true,
                true,
                21,
                theme,
                cx,
                |this, cx| this.sign_in(cx),
            )),
        };
        panel
            .child(section("Homebrew", theme))
            .child(setting(
                "Prefix",
                self.backend.prefix().display().to_string(),
                theme,
            ))
            .child(setting(
                "Privilege helper",
                "Optional fail-closed 1Password SUDO_ASKPASS; see the threat model".to_owned(),
                theme,
            ))
            .into_any_element()
    }
}

impl Render for Kettle {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_window(window);
        let view = self.controller.state.view;
        let busy = self.controller.state.operation != OperationState::Idle;
        let selected = self.controller.state.selection.selected().len();
        let visible = self.controller.state.selection.visible().len();
        let counts = [
            self.controller.state.packages.ids(View::Outdated).len(),
            self.controller.state.packages.ids(View::Installed).len(),
            self.controller.state.packages.ids(View::Browse).len(),
        ];
        let primary_action = if view == View::Browse {
            BrewAction::Install
        } else {
            BrewAction::Upgrade
        };
        let status: SharedString = match self.controller.state.operation {
            OperationState::Idle if selected > 0 => format!("{selected} selected").into(),
            OperationState::Idle if !self.controller.state.query.is_empty() => {
                format!("{visible} matches").into()
            }
            OperationState::Idle => "".into(),
            OperationState::Refreshing(stage) => format!("Refreshing: {stage:?}").into(),
            OperationState::Mutating { action, targets } => {
                format!("{} {targets} package(s)…", action.progressive()).into()
            }
        };
        let logs: Vec<_> = self
            .controller
            .state
            .logs()
            .iter()
            .rev()
            .take(200)
            .rev()
            .cloned()
            .collect();
        let log_count = self.controller.state.logs().len();
        let log_open = self.log_open;

        div()
            .key_context("Kettle")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &Primary, _, cx| this.primary(cx)))
            .on_action(cx.listener(|this, _: &UpgradeAll, _, cx| {
                let targets = this
                    .controller
                    .state
                    .packages
                    .ids(View::Outdated)
                    .to_vec();
                this.mutate(BrewAction::Upgrade, targets, cx);
            }))
            .on_action(cx.listener(|this, _: &ClearSearch, _, cx| {
                this.search.update(cx, SearchInput::clear);
            }))
            .on_action(cx.listener(|this, _: &SelectAllPackages, _, cx| {
                this.controller.state.selection.select_all();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.move_selection(-1, false, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.move_selection(1, false, cx)
            }))
            .on_action(cx.listener(|this, _: &ExtendUp, _, cx| {
                this.move_selection(-1, true, cx)
            }))
            .on_action(cx.listener(|this, _: &ExtendDown, _, cx| {
                this.move_selection(1, true, cx)
            }))
            .on_action(cx.listener(|_, _: &FocusNext, window, _| window.focus_next()))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, _| window.focus_prev()))
            .on_action(cx.listener(|this, _: &ViewOutdated, _, cx| {
                this.set_view(View::Outdated, cx)
            }))
            .on_action(cx.listener(|this, _: &ViewInstalled, _, cx| {
                this.set_view(View::Installed, cx)
            }))
            .on_action(cx.listener(|this, _: &ViewBrowse, _, cx| {
                this.set_view(View::Browse, cx)
            }))
            .on_action(cx.listener(|this, _: &ViewSettings, _, cx| {
                this.set_view(View::Settings, cx)
            }))
            .on_action(cx.listener(|_, _: &About, window, cx| {
                std::mem::drop(window.prompt(
                    PromptLevel::Info,
                    "Kettle",
                    Some(
                        "A native GPUI frontend for Homebrew. Homebrew remains authoritative for every mutation.",
                    ),
                    &["OK"],
                    cx,
                ));
            }))
            .on_action(cx.listener(|_, _: &Help, _, _| {
                let _ = Command::new("/usr/bin/open")
                    .arg("https://github.com/mcaney006/Kettle#readme")
                    .spawn();
            }))
            .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme.background))
            .text_color(rgb(theme.text))
            .font_family(UI_FONT)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(theme.line))
                    .bg(rgb(theme.chrome))
                    .child(div().w(px(320.)).min_w(px(160.)).child(self.search.clone()))
                    .child(self.button(
                        "refresh",
                        "Refresh".to_owned(),
                        false,
                        !busy,
                        2,
                        theme,
                        cx,
                        |this, cx| this.refresh(cx),
                    ))
                    .child(self.button(
                        "primary",
                        if selected == 0 {
                            format!("{} Selected", primary_action.label())
                        } else {
                            format!("{} {selected}", primary_action.label())
                        },
                        true,
                        !busy && selected > 0,
                        3,
                        theme,
                        cx,
                        |this, cx| this.primary(cx),
                    ))
                    .child(self.button(
                        "upgrade-all",
                        "Upgrade All".to_owned(),
                        false,
                        !busy && counts[0] > 0,
                        4,
                        theme,
                        cx,
                        |this, cx| {
                            let targets = this
                                .controller
                                .state
                                .packages
                                .ids(View::Outdated)
                                .to_vec();
                            this.mutate(BrewAction::Upgrade, targets, cx);
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .w(px(180.))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .pt_2()
                            .bg(rgb(theme.sidebar))
                            .border_r_1()
                            .border_color(rgb(theme.line))
                            .child(self.sidebar(
                                View::Outdated,
                                "Outdated",
                                counts[0],
                                10,
                                theme,
                                cx,
                            ))
                            .child(self.sidebar(
                                View::Installed,
                                "Installed",
                                counts[1],
                                11,
                                theme,
                                cx,
                            ))
                            .child(self.sidebar(
                                View::Browse,
                                "Browse",
                                counts[2],
                                12,
                                theme,
                                cx,
                            ))
                            .child(div().flex_1())
                            .child(self.sidebar(
                                View::Settings,
                                "Settings",
                                0,
                                13,
                                theme,
                                cx,
                            ))
                            .child(div().h_2()),
                    )
                    .child(self.content(view, visible, busy, theme, logs, log_open, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(rgb(theme.line))
                    .bg(rgb(theme.chrome))
                    .text_xs()
                    .child(div().flex_1().text_color(rgb(theme.muted)).child(status))
                    .when(log_count > 0, |element| {
                        element.child(
                            div()
                                .id("activity-toggle")
                                .tab_index(30)
                                .cursor_pointer()
                                .text_color(rgb(theme.muted))
                                .focus(|style| style.text_color(rgb(theme.text)))
                                .child(if log_open {
                                    "Activity ▾"
                                } else {
                                    "Activity ▸"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.log_open = !this.log_open;
                                    cx.notify();
                                }))
                                .on_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, _, cx| {
                                        if matches!(
                                            event.keystroke.key.as_str(),
                                            "enter" | "space"
                                        ) {
                                            this.log_open = !this.log_open;
                                            cx.notify();
                                        }
                                    },
                                )),
                        )
                    }),
            )
    }
}

impl Kettle {
    #[allow(clippy::too_many_arguments)]
    fn content(
        &self,
        view: View,
        visible: usize,
        busy: bool,
        theme: Theme,
        logs: Vec<LogEvent>,
        log_open: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if view == View::Settings {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .overflow_hidden()
                .child(self.settings(theme, cx))
                .into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .px_2()
                    .py_1()
                    .bg(rgb(theme.chrome))
                    .border_b_1()
                    .border_color(rgb(theme.line))
                    .children(COLUMNS.iter().enumerate().map(|(index, (name, _))| {
                        cell(index).text_color(rgb(theme.muted)).child(*name)
                    })),
            )
            .child(if visible == 0 && !busy {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(theme.faint))
                    .child(empty_message(view, &self.controller.state.query))
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        uniform_list(
                            "package-rows",
                            visible,
                            cx.processor(move |this, range: Range<usize>, _, cx| {
                                let entity = cx.entity();
                                range
                                    .map(|index| this.package_row(index, &entity, theme))
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .size_full()
                        .track_scroll(self.scroll.clone()),
                    )
                    .into_any_element()
            })
            .when(log_open, |element| {
                element.child(
                    div()
                        .h(px(180.))
                        .border_t_1()
                        .border_color(rgb(theme.line))
                        .p_2()
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .justify_end()
                        .children(logs.into_iter().map(|event| {
                            div()
                                .text_xs()
                                .font_family(MONO_FONT)
                                .text_color(rgb(log_color(event.level, theme)))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(event.message)
                        })),
                )
            })
            .into_any_element()
    }
}

fn cell(index: usize) -> gpui::Div {
    let cell = div()
        .min_w_0()
        .px_1()
        .text_sm()
        .whitespace_nowrap()
        .overflow_hidden()
        .truncate();
    if index == COLUMNS.len() - 1 {
        cell.flex_1()
    } else {
        cell.w(px(COLUMNS[index].1)).flex_shrink_0()
    }
}

fn section(title: &'static str, theme: Theme) -> gpui::AnyElement {
    div()
        .mt_4()
        .pb_1()
        .border_b_1()
        .border_color(rgb(theme.line))
        .text_sm()
        .child(title)
        .into_any_element()
}

fn setting(label: &'static str, value: String, theme: Theme) -> gpui::AnyElement {
    div()
        .flex()
        .gap_3()
        .text_sm()
        .child(
            div()
                .w(px(120.))
                .flex_shrink_0()
                .text_color(rgb(theme.muted))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(rgb(theme.text))
                .child(value),
        )
        .into_any_element()
}

fn empty_message(view: View, query: &str) -> String {
    if !query.is_empty() {
        return format!("No packages match “{query}”");
    }
    match view {
        View::Outdated => "Everything is up to date".to_owned(),
        View::Installed => "No packages installed".to_owned(),
        View::Browse => "Catalog unavailable; see Activity for the typed failure".to_owned(),
        View::Settings => String::new(),
    }
}
