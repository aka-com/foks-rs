//! GPUI desktop surface for the standalone local FOKS agent.

#![forbid(unsafe_code)]

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod text_field;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod supported {
    use std::path::PathBuf;
    use std::sync::Arc;

    use clap::Parser as _;
    use foks_agent_client::AgentClient;
    use foks_agent_proto::SecretString;
    use foks_desktop::{DesktopModel, PassphraseAction, Screen};
    use gpui::{
        div, prelude::*, px, rgb, size, App, Application, Bounds, Context, Entity, SharedString,
        Window, WindowBounds, WindowOptions,
    };

    use crate::text_field::TextField;

    #[derive(clap::Parser)]
    #[command(name = "foks-desktop")]
    struct Arguments {
        /// Private Unix socket exposed by foks-agent.
        #[arg(long, conflicts_with = "state_dir")]
        agent_socket: Option<PathBuf>,
        /// Explicit standalone FOKS state root; uses its agent.sock.
        #[arg(long, conflicts_with = "agent_socket")]
        state_dir: Option<PathBuf>,
    }

    struct FoksDesktop {
        model: DesktopModel,
        loading: bool,
        request_generation: u64,
        account_alias: Entity<TextField>,
        account_username: Entity<TextField>,
        account_device: Entity<TextField>,
        account_email: Entity<TextField>,
        account_invite: Entity<TextField>,
        account_passphrase: Entity<TextField>,
        account_passphrase_confirmation: Entity<TextField>,
        security_passphrase: Entity<TextField>,
        security_passphrase_confirmation: Entity<TextField>,
    }

    impl FoksDesktop {
        fn new(socket: PathBuf, cx: &mut Context<Self>) -> Self {
            let mut desktop = Self {
                model: DesktopModel::new(Arc::new(AgentClient::new(socket))),
                loading: false,
                request_generation: 0,
                account_alias: cx.new(|cx| TextField::new("Local alias", false, 64, cx)),
                account_username: cx.new(|cx| TextField::new("FOKS username", false, 256, cx)),
                account_device: cx.new(|cx| TextField::new("Device name", false, 256, cx)),
                account_email: cx.new(|cx| TextField::new("Email (optional)", false, 320, cx)),
                account_invite: cx
                    .new(|cx| TextField::new("Signup invite (optional)", true, 4096, cx)),
                account_passphrase: cx
                    .new(|cx| TextField::new("Passphrase (optional)", true, 1024, cx)),
                account_passphrase_confirmation: cx
                    .new(|cx| TextField::new("Confirm passphrase", true, 1024, cx)),
                security_passphrase: cx.new(|cx| TextField::new("Passphrase", true, 1024, cx)),
                security_passphrase_confirmation: cx
                    .new(|cx| TextField::new("Confirm passphrase", true, 1024, cx)),
            };
            desktop.refresh(cx);
            desktop
        }

        fn refresh(&mut self, cx: &mut Context<Self>) {
            let operation = match self.model.operation() {
                Ok(operation) => operation,
                Err(error) => {
                    self.model.accept(Err(error.to_owned()));
                    cx.notify();
                    return;
                }
            };
            let transport = self.model.transport();
            self.request_generation = self.request_generation.wrapping_add(1);
            let generation = self.request_generation;
            self.loading = true;
            cx.notify();
            let task = cx
                .background_executor()
                .spawn(async move { transport.call(operation) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this.request_generation != generation {
                        return;
                    }
                    this.loading = false;
                    this.model.accept(result);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }

        fn navigate(&mut self, screen: Screen, cx: &mut Context<Self>) {
            self.model.navigate(screen);
            self.request_generation = self.request_generation.wrapping_add(1);
            self.loading = false;
            if screen != Screen::Jobs {
                self.refresh(cx);
            } else {
                cx.notify();
            }
        }

        fn sidebar_button(&self, screen: Screen, cx: &Context<Self>) -> gpui::AnyElement {
            let selected = self.model.screen() == screen;
            div()
                .id(SharedString::from(format!("nav-{}", screen.label())))
                .w_full()
                .px_3()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .text_color(if selected {
                    rgb(0xffffff)
                } else {
                    rgb(0xc9d3e1)
                })
                .bg(if selected {
                    rgb(0x2764d8)
                } else {
                    rgb(0x172235)
                })
                .hover(|style| style.bg(rgb(0x263653)))
                .child(screen.label())
                .on_click(cx.listener(move |this, _, _, cx| this.navigate(screen, cx)))
                .into_any_element()
        }

        fn context_selectors(&self, cx: &Context<Self>) -> gpui::AnyElement {
            let mut row = div().flex().gap_2().flex_wrap();
            if self.model.screen() == Screen::Profiles {
                if let Some(profiles) = self.model.value().and_then(|value| value.as_array()) {
                    for profile in profiles {
                        if let Some(name) = profile.get("name").and_then(|name| name.as_str()) {
                            let name = name.to_owned();
                            let selected = self.model.selected_profile() == Some(name.as_str());
                            row = row.child(
                                div()
                                    .id(SharedString::from(format!("profile-{name}")))
                                    .px_3()
                                    .py_1()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .bg(if selected {
                                        rgb(0x2764d8)
                                    } else {
                                        rgb(0xe8edf5)
                                    })
                                    .text_color(if selected {
                                        rgb(0xffffff)
                                    } else {
                                        rgb(0x172235)
                                    })
                                    .child(name.clone())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.model.select_profile(name.clone());
                                        cx.notify();
                                    })),
                            );
                        }
                    }
                }
            } else if self.model.screen() == Screen::Accounts {
                if let Some(accounts) = self.model.value().and_then(|value| value.as_array()) {
                    for account in accounts {
                        if let Some(alias) = account.as_str() {
                            let alias = alias.to_owned();
                            let selected = self.model.selected_account() == Some(alias.as_str());
                            row = row.child(
                                div()
                                    .id(SharedString::from(format!("account-{alias}")))
                                    .px_3()
                                    .py_1()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .bg(if selected {
                                        rgb(0x2764d8)
                                    } else {
                                        rgb(0xe8edf5)
                                    })
                                    .text_color(if selected {
                                        rgb(0xffffff)
                                    } else {
                                        rgb(0x172235)
                                    })
                                    .child(alias.clone())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.model.select_account(alias.clone());
                                        cx.notify();
                                    })),
                            );
                        }
                    }
                }
            }
            row.into_any_element()
        }

        fn account_form(&self, cx: &Context<Self>) -> gpui::AnyElement {
            let field = |label: &'static str, input: Entity<TextField>| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().text_color(rgb(0x526178)).child(label))
                    .child(input)
            };
            div()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(rgb(0xd8dfeb))
                .bg(rgb(0xf8faff))
                .child(div().text_lg().child("Create account"))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x65738a))
                        .child("The invite and optional passphrase go only to the private local agent; passphrase fields are consumed when submitted."),
                )
                .child(
                    div()
                        .grid()
                        .grid_cols(2)
                        .gap_3()
                        .child(field("Local alias", self.account_alias.clone()))
                        .child(field("Username", self.account_username.clone()))
                        .child(field("Device", self.account_device.clone()))
                        .child(field("Email", self.account_email.clone())),
                )
                .child(field("Invite", self.account_invite.clone()))
                .child(
                    div()
                        .grid()
                        .grid_cols(2)
                        .gap_3()
                        .child(field("Passphrase (optional)", self.account_passphrase.clone()))
                        .child(field(
                            "Confirm passphrase",
                            self.account_passphrase_confirmation.clone(),
                        )),
                )
                .child(
                    div()
                        .id("create-account")
                        .w(px(160.))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(rgb(0x2764d8))
                        .text_color(rgb(0xffffff))
                        .child(if self.loading {
                            "Creating…"
                        } else {
                            "Create account"
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.submit_account(cx))),
                )
                .into_any_element()
        }

        fn submit_account(&mut self, cx: &mut Context<Self>) {
            if self.loading {
                return;
            }
            let alias = self.account_alias.read(cx).value().to_owned();
            let username = self.account_username.read(cx).value().to_owned();
            let device = self.account_device.read(cx).value().to_owned();
            let email = self.account_email.read(cx).value().to_owned();
            let invite = self.account_invite.read(cx).value().to_owned();
            let passphrase = self
                .account_passphrase
                .update(cx, |input, cx| input.take_secret(cx));
            let confirmation = self
                .account_passphrase_confirmation
                .update(cx, |input, cx| input.take_secret(cx));
            let operation = match self.model.create_account_operation(
                &alias,
                &username,
                &device,
                &email,
                &invite,
                Some(SecretString::new(passphrase)),
                Some(SecretString::new(confirmation)),
            ) {
                Ok(operation) => operation,
                Err(error) => {
                    self.model.accept(Err(error.to_owned()));
                    cx.notify();
                    return;
                }
            };
            let transport = self.model.transport();
            self.request_generation = self.request_generation.wrapping_add(1);
            let generation = self.request_generation;
            self.loading = true;
            cx.notify();
            let task = cx
                .background_executor()
                .spawn(async move { transport.call(operation) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this.request_generation != generation {
                        return;
                    }
                    this.loading = false;
                    if result.is_ok() {
                        this.model.select_account(alias.clone());
                        for input in [
                            &this.account_alias,
                            &this.account_username,
                            &this.account_device,
                            &this.account_email,
                            &this.account_invite,
                        ] {
                            input.update(cx, |input, cx| input.clear(cx));
                        }
                    }
                    this.model.accept(result);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }

        fn passphrase_form(&self, cx: &Context<Self>) -> gpui::AnyElement {
            let button = |id: &'static str, label: &'static str, action: PassphraseAction| {
                div()
                    .id(id)
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(rgb(0x2764d8))
                    .text_color(rgb(0xffffff))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.submit_passphrase(action, cx)))
            };
            div()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(rgb(0xd8dfeb))
                .bg(rgb(0xf8faff))
                .child(div().text_lg().child("Account passphrase"))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x65738a))
                        .child("Select an account. Set and change require confirmation; verify performs the public login challenge."),
                )
                .child(
                    div()
                        .grid()
                        .grid_cols(2)
                        .gap_3()
                        .child(self.security_passphrase.clone())
                        .child(self.security_passphrase_confirmation.clone()),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(button("set-passphrase", "Set", PassphraseAction::Set))
                        .child(button(
                            "change-passphrase",
                            "Change",
                            PassphraseAction::Change,
                        ))
                        .child(button(
                            "verify-passphrase",
                            "Verify",
                            PassphraseAction::Verify,
                        )),
                )
                .into_any_element()
        }

        fn submit_passphrase(&mut self, action: PassphraseAction, cx: &mut Context<Self>) {
            if self.loading {
                return;
            }
            let passphrase = SecretString::new(
                self.security_passphrase
                    .update(cx, |input, cx| input.take_secret(cx)),
            );
            let confirmation = SecretString::new(
                self.security_passphrase_confirmation
                    .update(cx, |input, cx| input.take_secret(cx)),
            );
            let confirmation = (action != PassphraseAction::Verify).then_some(confirmation);
            let operation = match self
                .model
                .passphrase_operation(action, passphrase, confirmation)
            {
                Ok(operation) => operation,
                Err(error) => {
                    self.model.accept(Err(error.to_owned()));
                    cx.notify();
                    return;
                }
            };
            let transport = self.model.transport();
            self.request_generation = self.request_generation.wrapping_add(1);
            let generation = self.request_generation;
            self.loading = true;
            cx.notify();
            let task = cx
                .background_executor()
                .spawn(async move { transport.call(operation) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this.request_generation != generation {
                        return;
                    }
                    this.loading = false;
                    this.model.accept(result);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    impl Render for FoksDesktop {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let context = format!(
                "Profile: {}    Account: {}",
                self.model.selected_profile().unwrap_or("none"),
                self.model.selected_account().unwrap_or("none")
            );
            let content = if self.loading {
                "Contacting local agent…".to_owned()
            } else if let Some(error) = self.model.error() {
                format!("Unavailable\n\n{error}")
            } else if let Some(value) = self.model.value() {
                serde_json::to_string_pretty(value)
                    .unwrap_or_else(|error| format!("Could not render response: {error}"))
            } else {
                if self.model.screen() == Screen::Jobs {
                    "Scheduled work runs only when you press “Run due jobs”.".to_owned()
                } else {
                    "No data loaded.".to_owned()
                }
            };
            let action = if self.model.screen() == Screen::Jobs {
                "Run due jobs"
            } else {
                "Refresh"
            };

            let response = div()
                .id("screen-content")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(rgb(0xd8dfeb))
                .bg(rgb(0xffffff))
                .font_family("monospace")
                .text_sm()
                .child(content);
            let body = if self.model.screen() == Screen::Accounts {
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(self.account_form(cx))
                    .child(self.passphrase_form(cx))
                    .child(response)
                    .into_any_element()
            } else {
                response.into_any_element()
            };

            div()
                .flex()
                .size_full()
                .bg(rgb(0xf5f7fb))
                .text_color(rgb(0x172235))
                .child(
                    div()
                        .w(px(220.0))
                        .h_full()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_4()
                        .bg(rgb(0x172235))
                        .child(
                            div()
                                .text_xl()
                                .text_color(rgb(0xffffff))
                                .mb_4()
                                .child("FOKS"),
                        )
                        .children(
                            Screen::ALL
                                .into_iter()
                                .map(|screen| self.sidebar_button(screen, cx)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .flex()
                        .flex_col()
                        .p_6()
                        .gap_4()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .child(div().text_2xl().child(self.model.screen().label()))
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(0x65738a))
                                                .child(context),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("refresh")
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(rgb(0x2764d8))
                                        .text_color(rgb(0xffffff))
                                        .child(action)
                                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                                ),
                        )
                        .child(self.context_selectors(cx))
                        .child(body),
                )
        }
    }

    pub fn main() {
        let arguments = Arguments::parse();
        let state_dir = arguments.state_dir.or_else(default_state_directory);
        let socket = arguments
            .agent_socket
            .or_else(|| {
                state_dir
                    .as_ref()
                    .map(|directory| directory.join("agent.sock"))
            })
            .expect("clap requires an agent socket or state directory");
        if let Some(directory) = state_dir {
            foks_desktop::install_crash_reporter(directory.join("crashes"));
        }
        Application::new().run(move |cx: &mut App| {
            crate::text_field::bind_keys(cx);
            let bounds = Bounds::centered(None, size(px(1100.0), px(720.0)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| FoksDesktop::new(socket, cx)),
            )
            .expect("FOKS desktop window could not be opened");
            cx.activate(true);
        });
    }

    fn default_state_directory() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        #[cfg(target_os = "macos")]
        return Some(home.join("Library/Application Support/FOKS"));
        #[cfg(target_os = "linux")]
        return Some(
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"))
                .join("foks-rs"),
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    supported::main();
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {
    eprintln!("foks-desktop supports macOS and Linux only");
    std::process::exit(1);
}
