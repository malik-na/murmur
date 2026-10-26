//! Accounts & keys: ChatGPT sign-in and per-provider API keys (Keychain), with a connection test.

use super::{AuthEvent, TestState};
use crate::types::*;
use crate::ui::hub::Hub;
use crate::ui::pages::open_url;
use crate::ui::theme::{Theme, TextExt};
use crate::ui::widgets::{self, Button, Icon};
use eframe::egui::{self, RichText, Stroke, Ui};
use std::time::Duration;

pub fn show(hub: &mut Hub, ui: &mut Ui) {
    let t = Theme::of(ui);
    ui.spacing_mut().item_spacing.y = 0.0;
    let settings = hub.settings_snapshot();
    if let Some(issue) = hub.shared.cloud_issue.lock().unwrap().as_ref() {
        hub.prefs.accounts_provider.get_or_insert(issue.provider);
    }

    let mut provider = hub.prefs.accounts_provider.unwrap_or(settings.cloud_provider);
    let opts: Vec<(CloudProvider, &str)> = CloudProvider::ALL.iter().map(|p| (*p, p.label())).collect();
    if widgets::segmented(ui, "accounts_provider", &mut provider, &opts).changed() {
        hub.prefs.accounts_provider = Some(provider);
        hub.prefs.key_input.clear();
        hub.prefs.key_msg = None;
    }
    ui.add_space(12.0);
    if provider == settings.cloud_provider && settings.use_llm_cleanup {
        ui.label(RichText::new(format!("{} is your cloud cleanup provider.", provider.label())).size(13.0).color(t.text_muted));
    }
    ui.add_space(24.0);

    if provider == CloudProvider::ChatGpt {
        widgets::group_header(ui, "ChatGPT account");
        widgets::settings_group(ui, |g| {
            g.custom(|ui| chatgpt_sign_in_ui(hub, ui));
        });
        widgets::group_header(ui, "Or use an API key");
    } else {
        widgets::group_header(ui, "API key");
    }

    let model = match provider {
        CloudProvider::Claude => settings.llm_model.clone(),
        CloudProvider::ChatGpt => settings.openai_model.clone(),
        CloudProvider::Grok => settings.grok_model.clone(),
    };
    key_group(hub, ui, provider, model);
}

fn key_group(hub: &mut Hub, ui: &mut Ui, provider: CloudProvider, model: String) {
    use crate::accounts;
    let t = Theme::of(ui);
    let status = match accounts::key_source(provider) {
        Some("Keychain") => "Saved in Keychain",
        Some(_) => "Using environment variable",
        None => "No key saved",
    };
    widgets::settings_group(ui, |g| {
        g.row(&format!("{} API key", provider.label()), status, |ui| {
            if ui.add(Button::secondary("Get API key").icon(Icon::ExternalLink).height(32.0)).clicked() {
                open_url(accounts::console_url(provider));
            }
        });
        g.custom(|ui| {
            let st = &mut hub.prefs;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let w = (ui.available_width() - 250.0).max(160.0);
                widgets::input_field(ui, &mut st.key_input, "Paste API key", w, true);
                if ui.add_enabled(!st.key_input.trim().is_empty(), Button::primary("Save").height(32.0)).clicked() {
                    st.key_msg = Some(match accounts::set_api_key(provider, &st.key_input) {
                        Ok(()) => (provider, true, "Saved".to_string()),
                        Err(e) => (provider, false, format!("{e:#}")),
                    });
                    st.key_input.clear();
                }
                if ui.add_enabled(accounts::key_source(provider) == Some("Keychain"), Button::secondary("Remove").height(32.0)).clicked() {
                    st.key_msg = Some(match accounts::delete_api_key(provider) {
                        Ok(()) => (provider, true, "Removed".to_string()),
                        Err(e) => (provider, false, format!("{e:#}")),
                    });
                }
                let running = matches!(*st.test_result.lock().unwrap(), TestState::Running(_));
                // Signed in with ChatGPT: Test checks the plan path, the one cleanup will actually use.
                let plan = provider == CloudProvider::ChatGpt && crate::chatgpt_auth::account().is_some_and(|a| a.plan_usage);
                let can_test = !running && (plan || accounts::key_source(provider).is_some());
                if ui.add_enabled(can_test, Button::secondary("Test").height(32.0)).clicked() {
                    let slot = st.test_result.clone();
                    let ctx = ui.ctx().clone();
                    let model = model.clone();
                    if plan {
                        *st.test_result.lock().unwrap() = TestState::Running(provider);
                        st.key_msg = None;
                        std::thread::spawn(move || {
                            let r = match crate::chatgpt_auth::access_token() {
                                Some(token) => crate::format::test_chatgpt_plan(&model, &token).map_err(|e| format!("{e:#}")),
                                None => Err("ChatGPT session expired; sign in again".to_string()),
                            };
                            *slot.lock().unwrap() = TestState::Done(provider, r);
                            ctx.request_repaint();
                        });
                    } else if let Some(key) = accounts::api_key(provider) {
                        *st.test_result.lock().unwrap() = TestState::Running(provider);
                        st.key_msg = None;
                        std::thread::spawn(move || {
                            let r = crate::format::test_provider(provider, &model, &key).map_err(|e| format!("{e:#}"));
                            *slot.lock().unwrap() = TestState::Done(provider, r);
                            ctx.request_repaint();
                        });
                    }
                }
            });
            match &*st.test_result.lock().unwrap() {
                TestState::Running(p) if *p == provider => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Testing…").size(13.0).color(t.text_muted));
                    });
                }
                TestState::Done(p, r) if *p == provider => match r {
                    Ok(()) => {
                        ui.horizontal(|ui| {
                            widgets::icon(ui, Icon::Check, 15.0, t.text);
                            ui.label(RichText::new("Works").size(13.0).medium().color(t.text));
                        });
                    }
                    Err(e) => {
                        ui.add(egui::Label::new(RichText::new(e).size(13.0).color(t.danger)).wrap());
                    }
                },
                _ => {}
            }
            if let Some((p, ok, m)) = &st.key_msg {
                if *p == provider {
                    let c = if *ok { t.text_muted } else { t.danger };
                    ui.label(RichText::new(m).size(13.0).color(c));
                }
            }
        });
    });
}

/// Applies finished background sign-in/out results and refreshes the cached account.
fn poll_chatgpt_auth(hub: &mut Hub) {
    let st = &mut hub.prefs;
    let events: Vec<AuthEvent> = std::mem::take(&mut *st.auth_events.lock().unwrap());
    for ev in events {
        let (id, r, out) = match ev {
            AuthEvent::SignedIn(id, r) => (id, r, false),
            AuthEvent::SignedOut(id, r) => (id, r, true),
        };
        if st.auth_pending != Some((id, out)) {
            continue; // cancelled in the UI; the account cache below still catches up
        }
        st.auth_pending = None;
        st.chatgpt_checked = None;
        match r {
            Ok(()) => {
                st.auth_error = None;
                st.plan_notice_due = !out;
            }
            Err(e) => st.auth_error = Some(e),
        }
    }
    let stale = st.chatgpt_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(5));
    if stale {
        st.chatgpt_account = crate::chatgpt_auth::account();
        st.chatgpt_checked = Some(std::time::Instant::now());
    }
}

fn chatgpt_sign_in_ui(hub: &mut Hub, ui: &mut Ui) {
    use crate::chatgpt_auth;
    poll_chatgpt_auth(hub);
    let t = Theme::of(ui);
    let st = &mut hub.prefs;
    let ctx = ui.ctx().clone();
    ctx.request_repaint_after(if st.auth_pending.is_some() { Duration::from_millis(200) } else { Duration::from_secs(5) });

    match st.chatgpt_account.clone() {
        None => {
            match st.auth_pending {
                Some((_, false)) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Waiting for you to finish in the browser…").color(t.text));
                        if ui.link("Cancel").clicked() {
                            st.auth_pending = None;
                            st.auth_attempt += 1;
                        }
                    });
                }
                _ => {
                    ui.label(RichText::new("Use your ChatGPT Plus or Pro plan for cleanup. No API key needed.").size(13.0).color(t.text_muted));
                    ui.add_space(4.0);
                    if ui.add(Button::primary("Continue with ChatGPT").height(40.0).radius(10.0).min_width(ui.available_width())).clicked() {
                        st.auth_attempt += 1;
                        let id = st.auth_attempt;
                        st.auth_pending = Some((id, false));
                        st.auth_error = None;
                        let events = st.auth_events.clone();
                        std::thread::spawn(move || {
                            let r = chatgpt_auth::sign_in().map(|_| ()).map_err(|e| format!("{e:#}"));
                            events.lock().unwrap().push(AuthEvent::SignedIn(id, r));
                            ctx.request_repaint();
                        });
                    }
                }
            }
            if let Some(e) = &st.auth_error {
                ui.label(RichText::new(e).size(13.0).color(t.danger));
            }
        }
        Some(acct) => {
            let who = acct.email.clone().unwrap_or_else(|| "your ChatGPT account".to_string());
            let signing_out = matches!(st.auth_pending, Some((_, true)));
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("Signed in as {who}")).size(14.0).medium().color(t.text));
                if acct.plan_usage {
                    widgets::pill_badge(ui, "Using ChatGPT plan", t.accent, t.accent.gamma_multiply(0.16));
                }
            });
            if !acct.plan_usage {
                ui.label(
                    RichText::new("This account can't share plan usage; Murmur will use your API key instead.")
                        .size(13.0)
                        .color(t.warning),
                );
            }
            ui.horizontal(|ui| {
                if ui.add(Button::secondary("Manage usage").icon(Icon::ExternalLink).height(32.0)).clicked() {
                    open_url(chatgpt_auth::MANAGE_USAGE_URL);
                }
                if ui.add_enabled(!signing_out, Button::secondary("Sign out").height(32.0)).clicked() {
                    st.auth_attempt += 1;
                    let id = st.auth_attempt;
                    st.auth_pending = Some((id, true));
                    st.auth_error = None;
                    let events = st.auth_events.clone();
                    std::thread::spawn(move || {
                        let r = chatgpt_auth::sign_out().map_err(|e| format!("{e:#}"));
                        events.lock().unwrap().push(AuthEvent::SignedOut(id, r));
                        ctx.request_repaint();
                    });
                }
                if signing_out {
                    ui.spinner();
                }
            });
            if let Some(e) = &st.auth_error {
                ui.label(RichText::new(e).size(13.0).color(t.danger));
            }
            if st.plan_notice_due && !st.plan_notice_dismissed {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(t.accent.gamma_multiply(if t.dark { 0.15 } else { 0.08 }))
                    .stroke(Stroke::new(1.0_f32, t.accent.gamma_multiply(0.4)))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("You're using your ChatGPT plan — Murmur's cleanup requests count toward your ChatGPT usage.")
                                .color(t.text),
                        );
                        if ui.add(Button::secondary("Got it").height(32.0)).clicked() {
                            st.plan_notice_dismissed = true;
                        }
                    });
            }
        }
    }
}
