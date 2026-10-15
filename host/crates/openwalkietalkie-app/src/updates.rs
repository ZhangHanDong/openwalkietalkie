//! Update controls and polling; USB and downloads remain in the daemon.
use crate::{
    shell::Shell,
    style::palette,
    ui::{self, Btn},
};
use deck_gui_core::UiAction;
use gpui::{div, prelude::*, Context, Div, Window};
use gpui_component::{h_flex, v_flex};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct FirmwarePane {
    status: Value,
    pending: bool,
    polling: bool,
    checked: Option<Instant>,
    selected_board: Option<String>,
    confirm: bool,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn busy(value: &Value) -> bool {
    matches!(
        text(value, "phase"),
        "checking" | "downloading" | "installing" | "reconnecting"
    )
}

impl Shell {
    pub(crate) fn refresh_firmware(
        &mut self,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.firmware.pending || !self.state.connected() {
            return;
        }
        let check = force
            || self
                .firmware
                .checked
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(21600));
        self.firmware.pending = true;
        self.ask(if check { UiAction::FirmwareCheck { force } } else { UiAction::FirmwareStatus }, window, cx, move |this, result, window, cx| {
            this.firmware.pending = false;
            match result {
                Ok(status) => {
                    if check { this.firmware.checked = Some(Instant::now()); }
                    let announce = status["available"].as_bool() == Some(true) && this.firmware.status["available"].as_bool() != Some(true);
                    let previous = this.firmware.status["device"].clone();
                    this.firmware.status = status;
                    if announce { this.toast("A firmware update is available in Service → Updates.", window, cx); }
                    if previous != this.firmware.status["device"] { this.firmware.confirm = false; this.firmware.selected_board = None; }
                }
                Err(error) => { this.firmware.status = serde_json::json!({"phase":"error","error":failure_text(&error),"message":"Firmware updates are unavailable from this service."}); }
            }
            this.schedule_update_poll(window, cx);
            cx.notify();
        });
    }

    fn schedule_update_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.firmware.polling {
            return;
        }
        self.firmware.polling = true;
        let delay = if busy(&self.firmware.status) { 1 } else { 30 };
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(delay))
                .await;
            this.update_in(cx, |this, window, cx| {
                this.firmware.polling = false;
                this.refresh_firmware(false, window, cx);
                this.restart_updated_service(window, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn restart_updated_service(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_restart_attempted || !self.state.connected() {
            return;
        }
        let Some(expected) = bundled_daemon() else {
            return;
        };
        self.update_restart_attempted = true;
        self.ask(
            UiAction::ServiceInfo,
            window,
            cx,
            move |this, result, window, cx| {
                let Ok(value) = result else {
                    this.update_restart_attempted = false;
                    return;
                };
                if !needs_restart(&value, &expected, env!("CARGO_PKG_VERSION")) {
                    return;
                }
                this.ask(
                    UiAction::ServiceRestartWhenIdle,
                    window,
                    cx,
                    |this, result, _, _| {
                        // Busy recognition/firmware work is retried by the poll; a
                        // failed call never forces an exit or kills the daemon.
                        if result.is_err() {
                            this.update_restart_attempted = false;
                        }
                    },
                );
            },
        );
    }

    pub(crate) fn render_updates(&self, dim: bool, cx: &Context<Self>) -> Div {
        let p = *palette(cx);
        let status = &self.firmware.status;
        let device = &status["device"];
        let board = self
            .firmware
            .selected_board
            .as_deref()
            .unwrap_or_else(|| text(device, "board"));
        let version = text(&status["release"]["firmware"], "version");
        let serial = text(device, "serial");
        let active = busy(status) || self.firmware.pending;
        let can_install = !active
            && !dim
            && !serial.is_empty()
            && !version.is_empty()
            && status["available"].as_bool() == Some(true)
            && status["release"]["firmware"]["variants"]
                .get(board)
                .is_some();
        let label = if active {
            format!(
                "{} {}%",
                text(status, "message"),
                status["progress"].as_u64().unwrap_or(0)
            )
        } else if !text(status, "message").is_empty() {
            text(status, "message").to_string()
        } else {
            "Check for firmware updates with your deck connected by USB.".into()
        };
        let mut body =
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child("App updates"))
                        .child(
                            Btn::new("app-check-updates", "Check for app updates").on_click(
                                cx.listener(|this, _, window, cx| this.check_updates(window, cx)),
                            ),
                        ),
                )
                .child(div().text_color(p.muted).child(
                    "Release builds automatically check for new app versions every six hours.",
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(ui::brand_mark(28.))
                        .child(div().flex_1().child("OpenWalkieTalkie firmware"))
                        .child(
                            Btn::new("firmware-check", "Check firmware")
                                .disabled(dim || active)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.refresh_firmware(true, window, cx)
                                })),
                        ),
                )
                .child(div().child(label));
        if !serial.is_empty() {
            body = body.child(div().child(format!(
                "{} · USB {} · Installed {} · Available {}",
                if board.is_empty() {
                    "Select your board below"
                } else if board == "hw-a" {
                    "Rev A"
                } else {
                    "Rev B"
                },
                serial,
                if device["recovery"].as_bool() == Some(true) {
                    "Recovery mode"
                } else {
                    text(device, "version")
                },
                version
            )));
            if text(device, "board").is_empty() {
                body = body.child(div().child("The device cannot report its board. Choose the revision printed on your board."))
                    .child(h_flex().gap_2()
                        .child(Btn::new("firmware-rev-a", "Rev A").disabled(active).on_click(cx.listener(|this, _, _, cx| { this.firmware.selected_board = Some("hw-a".into()); this.firmware.confirm = false; cx.notify(); })))
                        .child(Btn::new("firmware-rev-b", "Rev B").disabled(active).on_click(cx.listener(|this, _, _, cx| { this.firmware.selected_board = Some("hw-b".into()); this.firmware.confirm = false; cx.notify(); }))));
            }
        }
        if let Some(error) = status["error"].as_str() {
            body = body.child(div().text_color(p.muted).child(error.to_string()));
        }
        if self.firmware.confirm {
            body = body
                .child(div().child(format!(
                    "Install firmware {version}? Keep USB connected until your deck restarts."
                )))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Btn::new("firmware-confirm", "Install firmware")
                                .solid()
                                .disabled(!can_install)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.install_firmware(window, cx)
                                })),
                        )
                        .child(Btn::new("firmware-cancel", "Cancel").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.firmware.confirm = false;
                                cx.notify();
                            },
                        ))),
                );
        } else {
            body = body.child(
                Btn::new("firmware-review", "Update firmware…")
                    .disabled(!can_install)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.firmware.confirm = true;
                        cx.notify();
                    })),
            );
        }
        ui::section("Updates", body, &p)
    }

    fn install_firmware(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.firmware.pending || !self.firmware.confirm {
            return;
        }
        let status = &self.firmware.status;
        let action = UiAction::FirmwareInstall {
            board: self
                .firmware
                .selected_board
                .clone()
                .unwrap_or_else(|| text(&status["device"], "board").into()),
            serial: text(&status["device"], "serial").into(),
            version: text(&status["release"]["firmware"], "version").into(),
        };
        self.firmware.confirm = false;
        self.firmware.pending = true;
        self.ask(action, window, cx, |this, result, window, cx| {
            this.firmware.pending = false;
            match result {
                Ok(status) => this.firmware.status = status,
                Err(error) => this.toast(failure_text(&error), window, cx),
            }
            this.refresh_firmware(false, window, cx);
        });
    }
}

fn failure_text(error: &deck_client::ClientError) -> String {
    let details = match error {
        deck_client::ClientError::Rpc { data, .. } => data.as_ref(),
        deck_client::ClientError::Refused(error) => Some(&error.data),
        _ => None,
    };
    details
        .and_then(|data| data.get("why"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

fn bundled_daemon() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let directory = exe.parent()?;
    if directory.file_name()? != "MacOS" || directory.parent()?.file_name()? != "Contents" {
        return None;
    }
    Some(directory.join("deckd").to_string_lossy().into_owned())
}

fn needs_restart(info: &Value, expected: &str, version: &str) -> bool {
    text(info, "executable") == expected
        && !text(info, "daemon").is_empty()
        && text(info, "daemon") != version
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_older_copy_of_this_bundled_daemon_is_restarted() {
        let exe = "/Applications/OpenWalkieTalkie.app/Contents/MacOS/deckd";
        assert!(needs_restart(
            &serde_json::json!({"daemon":"0.1.0","executable":exe}),
            exe,
            "0.2.0"
        ));
        assert!(!needs_restart(
            &serde_json::json!({"daemon":"0.2.0","executable":exe}),
            exe,
            "0.2.0"
        ));
        assert!(!needs_restart(
            &serde_json::json!({"daemon":"0.1.0","executable":"/tmp/deckd"}),
            exe,
            "0.2.0"
        ));
        assert!(!needs_restart(&serde_json::json!({}), exe, "0.2.0"));
    }
}
