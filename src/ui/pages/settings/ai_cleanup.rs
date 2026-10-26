//! AI cleanup: local model (LM Studio), cloud fallback provider, time limit, formatting and learning.

use super::Section;
use crate::types::*;
use crate::ui::hub::Hub;
use crate::ui::pages::open_url;
use crate::ui::theme::{Theme, TextExt};
use crate::ui::widgets::{self, Button, Icon};
use eframe::egui::{self, RichText, Stroke, Ui};

pub fn show(hub: &mut Hub, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 0.0;
    hub.with_settings(|hub, s| {
        cloud_issue_card(hub, ui, s);

        widgets::settings_group(ui, |g| {
            g.row("Local model (LM Studio)", "Adds punctuation and fixes run-on sentences on your Mac. Tried first.", |ui| {
                widgets::toggle(ui, &mut s.use_local_llm);
            });
            if s.use_local_llm {
                g.row("Server URL", "Where LM Studio is listening", |ui| {
                    widgets::input_field(ui, &mut s.local_llm_url, "http://localhost:1234/v1", 260.0, false);
                });
                g.row("Model", "Model id to request from the server", |ui| {
                    widgets::input_field(ui, &mut s.local_llm_model, "Model id", 260.0, false);
                });
            }
        });

        widgets::group_header(ui, "Cloud fallback");
        widgets::settings_group(ui, |g| {
            g.row("Use a cloud model", "Used when the local model is unavailable, off or too slow.", |ui| {
                widgets::toggle(ui, &mut s.use_llm_cleanup);
            });
            if s.use_llm_cleanup {
                g.row("Provider", "Which service cleans up your text", |ui| {
                    let opts: Vec<(CloudProvider, &str)> = CloudProvider::ALL.iter().map(|p| (*p, p.label())).collect();
                    let mut sel = s.cloud_provider;
                    if widgets::segmented(ui, "provider", &mut sel, &opts).changed() {
                        s.cloud_provider = sel;
                        hub.prefs.key_input.clear();
                        hub.prefs.key_msg = None;
                    }
                });
                let provider = s.cloud_provider;
                let model = match provider {
                    CloudProvider::Claude => &mut s.llm_model,
                    CloudProvider::ChatGpt => &mut s.openai_model,
                    CloudProvider::Grok => &mut s.grok_model,
                };
                g.row(&format!("{} model", provider.label()), "Model id used for cleanup", |ui| {
                    widgets::input_field(ui, model, "Model id", 260.0, false);
                });
                let signed_in = provider == CloudProvider::ChatGpt && crate::chatgpt_auth::account().is_some();
                let cred = if signed_in {
                    "Signed in with ChatGPT".to_string()
                } else {
                    match crate::accounts::key_source(provider) {
                        Some("Keychain") => "API key saved in Keychain".to_string(),
                        Some(_) => "Using API key from environment variable".to_string(),
                        None => "No API key yet".to_string(),
                    }
                };
                g.row("Credentials", cred, |ui| {
                    if ui.add(Button::secondary("Manage").height(32.0).min_width(100.0)).clicked() {
                        hub.prefs.accounts_provider = Some(provider);
                        hub.settings_modal = Some(Section::Accounts);
                    }
                });
            }
            g.row("Time limit", "After this, Murmur pastes the locally formatted text instead.", |ui| {
                ui.spacing_mut().slider_width = 150.0;
                ui.add(
                    egui::Slider::new(&mut s.llm_timeout_ms, 1000..=8000)
                        .step_by(250.0)
                        .custom_formatter(|v, _| format!("{:.1} s", v / 1000.0))
                        .custom_parser(|t| t.trim().trim_end_matches('s').trim().parse::<f64>().ok().map(|v| v * 1000.0)),
                );
            });
        });

        widgets::group_header(ui, "Formatting");
        widgets::settings_group(ui, |g| {
            g.row("Smart formatting", "Remove filler words, fix punctuation and follow spoken commands like “new line”.", |ui| {
                widgets::toggle(ui, &mut s.smart_formatting);
            });
            g.row(
                "Learn from my corrections",
                "After pasting, Murmur watches your edits for up to a minute and adds corrected names and terms to your dictionary.",
                |ui| {
                    widgets::toggle(ui, &mut s.auto_learn_words);
                },
            );
        });
    });
}

/// Warning card for the latest actionable cloud cleanup failure.
pub(super) fn cloud_issue_card(hub: &mut Hub, ui: &mut Ui, s: &mut Settings) {
    use crate::accounts;
    let Some(issue) = hub.shared.cloud_issue.lock().unwrap().clone() else { return };
    let t = Theme::of(ui);
    let p = issue.provider;
    let color = match issue.kind {
        CloudIssueKind::Billing | CloudIssueKind::PlanLimit => t.warning,
        CloudIssueKind::BadKey | CloudIssueKind::UnknownModel => t.danger,
    };
    let signed_in = p == CloudProvider::ChatGpt && crate::chatgpt_auth::account().is_some();
    let current_model = match p {
        CloudProvider::Claude => s.llm_model.clone(),
        CloudProvider::ChatGpt => s.openai_model.clone(),
        CloudProvider::Grok => s.grok_model.clone(),
    };
    let default_model = {
        let d = Settings::default();
        match p {
            CloudProvider::Claude => d.llm_model,
            CloudProvider::ChatGpt => d.openai_model,
            CloudProvider::Grok => d.grok_model,
        }
    };
    let title = match issue.kind {
        CloudIssueKind::Billing => format!("{} account is out of credit", p.label()),
        CloudIssueKind::BadKey if signed_in => "ChatGPT session expired — sign in again".to_string(),
        CloudIssueKind::BadKey => format!("{} rejected the API key", p.label()),
        CloudIssueKind::UnknownModel => format!("{} doesn't recognise model “{}”", p.label(), current_model),
        CloudIssueKind::PlanLimit => "ChatGPT plan limit reached for Murmur".to_string(),
    };
    let title = if p != s.cloud_provider && !title.starts_with(p.label()) { format!("{}: {}", p.label(), title) } else { title };
    let mut dismiss = false;
    egui::Frame::new()
        .fill(color.gamma_multiply(if t.dark { 0.15 } else { 0.08 }))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.5)))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                widgets::icon(ui, Icon::Warning, 17.0, color);
                ui.label(RichText::new(title).size(14.0).semibold().color(t.text));
            });
            if !issue.detail.trim().is_empty() {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(issue.detail.trim()).size(13.0).color(t.text_muted)).wrap());
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                match issue.kind {
                    CloudIssueKind::Billing => {
                        let url = match p {
                            CloudProvider::Claude => "https://platform.claude.com/settings/billing",
                            CloudProvider::ChatGpt => "https://platform.openai.com/settings/organization/billing",
                            CloudProvider::Grok => "https://console.x.ai",
                        };
                        if ui.add(Button::primary("Open billing").icon(Icon::ExternalLink).height(32.0)).clicked() {
                            open_url(url);
                        }
                    }
                    CloudIssueKind::BadKey => {
                        if ui.add(Button::primary("Get API key").icon(Icon::ExternalLink).height(32.0)).clicked() {
                            open_url(accounts::console_url(p));
                        }
                    }
                    CloudIssueKind::UnknownModel => {
                        if ui.add(Button::primary(format!("Use {default_model}")).height(32.0)).clicked() {
                            match p {
                                CloudProvider::Claude => s.llm_model = default_model.clone(),
                                CloudProvider::ChatGpt => s.openai_model = default_model.clone(),
                                CloudProvider::Grok => s.grok_model = default_model.clone(),
                            }
                            dismiss = true;
                        }
                    }
                    CloudIssueKind::PlanLimit => {
                        if ui.add(Button::primary("Manage usage").icon(Icon::ExternalLink).height(32.0)).clicked() {
                            open_url(crate::chatgpt_auth::MANAGE_USAGE_URL);
                        }
                    }
                }
                if ui.add(Button::secondary("Dismiss").height(32.0)).clicked() {
                    dismiss = true;
                }
            });
        });
    if dismiss {
        *hub.shared.cloud_issue.lock().unwrap() = None;
    }
    ui.add_space(24.0);
}
