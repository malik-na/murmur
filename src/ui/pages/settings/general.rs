//! General: shortcut, microphone, language and speech model.

use crate::types::*;
use crate::ui::hub::Hub;
use crate::ui::pages::{capitalize, clickable};
use crate::ui::theme::{Theme, TextExt};
use crate::ui::widgets::{self, Button};
use eframe::egui::{self, Align, CornerRadius, Layout, Margin, RichText, Stroke, Ui};

pub const MODELS: [(&str, &str, &str); 3] = [
    ("ggml-large-v3-turbo-q5_0.bin", "Large v3 Turbo", "Recommended · multilingual · ~550 MB"),
    ("ggml-base.en.bin", "Base (English)", "Fastest · English only · ~140 MB"),
    ("ggml-small.en.bin", "Small (English)", "Balanced · English only · ~460 MB"),
];

pub const LANGUAGES: [(&str, &str); 14] = [
    ("auto", "Auto-detect"),
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("ja", "Japanese"),
    ("zh", "Chinese"),
    ("ko", "Korean"),
    ("hi", "Hindi"),
    ("ar", "Arabic"),
    ("ru", "Russian"),
];

const CONTROL_W: f32 = 200.0;

pub fn show(hub: &mut Hub, ui: &mut Ui) {
    let devices = hub.prefs.devices.get_or_insert_with(crate::audio::list_input_devices).clone();
    ui.spacing_mut().item_spacing.y = 0.0;
    hub.with_settings(|hub, s| {
        widgets::settings_group(ui, |g| {
            let mut job = egui::text::LayoutJob::default();
            let t = Theme::of(g.ui);
            let f = |font| egui::TextFormat::simple(font, t.text_muted);
            job.append("Hold ", 0.0, f(crate::ui::theme::regular(13.0)));
            job.append("fn", 0.0, egui::TextFormat::simple(crate::ui::theme::semibold(13.0), t.text));
            job.append(" and speak. ", 0.0, f(crate::ui::theme::regular(13.0)));
            job.append("fn + space", 0.0, egui::TextFormat::simple(crate::ui::theme::semibold(13.0), t.text));
            job.append(" for hands-free.", 0.0, f(crate::ui::theme::regular(13.0)));
            g.row("Shortcuts", job, |ui| {
                ui.add_enabled(false, Button::secondary("Change").height(32.0).min_width(100.0))
                    .on_disabled_hover_text("Custom shortcuts are coming soon");
            });
            g.row("Microphone", "The input device Murmur listens to", |ui| {
                let mut opts: Vec<(Option<String>, String)> = vec![(None, "System default".to_string())];
                opts.extend(devices.iter().map(|d| (Some(d.clone()), d.clone())));
                if let Some(cur) = &s.input_device {
                    if !devices.contains(cur) {
                        opts.push((Some(cur.clone()), format!("{cur} (unavailable)")));
                    }
                }
                widgets::dropdown(ui, "mic", &mut s.input_device, &opts, CONTROL_W);
            });
            g.row("Language", "The language you speak", |ui| {
                let opts: Vec<(String, &str)> = LANGUAGES.iter().map(|(c, n)| (c.to_string(), *n)).collect();
                widgets::dropdown(ui, "lang", &mut s.language, &opts, CONTROL_W);
            });
        });

        widgets::group_header(ui, "Speech model");
        models(hub, ui, s);
    });
}

fn models(hub: &mut Hub, ui: &mut Ui, s: &mut Settings) {
    let t = Theme::of(ui);
    let status = hub.shared.model_status.lock().unwrap().clone();
    let downloading = status.starts_with("downloading");
    for (file, name, desc) in MODELS {
        let selected = s.model_file == file;
        let present = hub.prefs.models_dir.join(file).exists();
        let stroke = if selected { Stroke::new(1.5_f32, t.accent) } else { Stroke::new(1.0_f32, t.border) };
        let mut download = false;
        let resp = clickable(ui, |ui| {
            egui::Frame::new()
                .fill(t.surface_sunken)
                .stroke(stroke)
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(name).size(14.0).medium().color(t.text));
                            ui.label(RichText::new(desc).size(13.0).color(t.text_muted));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let label = if selected {
                                match status.as_str() {
                                    "ready" => "Ready".to_string(),
                                    "loading" => "Loading…".to_string(),
                                    "missing" | "not_loaded" if !present => "Not downloaded".to_string(),
                                    "missing" | "not_loaded" => "Downloaded".to_string(),
                                    other => capitalize(other),
                                }
                            } else if present {
                                "Downloaded".to_string()
                            } else {
                                "Not downloaded".to_string()
                            };
                            if !present
                                && !(selected && downloading)
                                && ui.add_enabled(!downloading, Button::primary("Download").height(32.0)).clicked()
                            {
                                download = true;
                            }
                            ui.label(RichText::new(label).size(13.0).color(t.text_muted));
                        });
                    });
                });
        });
        if download {
            s.model_file = file.to_string();
            start_download(hub, file);
        } else if resp.clicked() && !selected && !downloading {
            s.model_file = file.to_string();
        }
        ui.add_space(8.0);
    }
}

fn start_download(hub: &mut Hub, file: &str) {
    let shared = hub.shared.clone();
    let tx = hub.tx.clone();
    let file = file.to_string();
    let dest = hub.prefs.models_dir.join(&file);
    *shared.model_status.lock().unwrap() = "downloading 0%".into();
    std::thread::spawn(move || {
        let progress = |done: u64, total: Option<u64>| {
            let text = match total {
                Some(t) if t > 0 => format!("downloading {}%", done * 100 / t),
                _ => format!("downloading {} MB", done / 1_000_000),
            };
            *shared.model_status.lock().unwrap() = text;
        };
        match crate::transcribe::download_model(&file, &dest, progress) {
            Ok(()) => {
                *shared.model_status.lock().unwrap() = "not_loaded".into();
                let _ = tx.send(AppEvent::ReloadModel);
            }
            Err(e) => {
                log::error!("model download: {e:#}");
                *shared.model_status.lock().unwrap() = format!("error: download failed: {e}");
            }
        }
    });
}
