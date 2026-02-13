use eframe::egui;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gst::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};

#[derive(Deserialize, Serialize, Default, Clone)]
struct AppConfig {
    video_path: String,
    monitor_name: String,
    recent_videos: Vec<String>,
}

impl AppConfig {
    fn config_path() -> PathBuf {
        PathBuf::from("config.json")
    }

    fn load() -> Self {
        let mut config: AppConfig = fs::read_to_string(Self::config_path())
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default();

        config.recent_videos.retain(|p| Path::new(p).exists());
        config
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(Self::config_path(), json);
        }
    }
}

struct ConfigApp {
    config: AppConfig,
    status: String,
    monitors: Vec<String>,
    preview_texture: Option<egui::TextureHandle>,
    history_textures: HashMap<String, egui::TextureHandle>,
    tx: Sender<(String, egui::ColorImage)>,
    rx: Receiver<(String, egui::ColorImage)>,
}

impl ConfigApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        gst::init().expect("Error al inicializar GStreamer");
        Self::setup_custom_style(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let config = AppConfig::load();
        let (tx, rx) = channel();

        let mut app = Self {
            config,
            status: "Listo para iniciar.".to_owned(),
            monitors: Vec::new(),
            preview_texture: None,
            history_textures: HashMap::new(),
            tx,
            rx,
        };

        app.refresh_monitors();

        for path in app.config.recent_videos.clone() {
            app.spawn_preview_task(path, &cc.egui_ctx);
        }

        if !app.config.video_path.is_empty() && Path::new(&app.config.video_path).exists() {
            app.spawn_preview_task(app.config.video_path.clone(), &cc.egui_ctx);
        }

        app
    }

    fn setup_custom_style(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(15, 15, 20);
        visuals.widgets.noninteractive.rounding = 14.0.into();
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(30, 30, 35);
        visuals.widgets.inactive.rounding = 12.0.into();
        visuals.selection.bg_fill = egui::Color32::from_rgb(49, 175, 145);
        ctx.set_visuals(visuals);

        ctx.style_mut(|style| {
            style.spacing.interact_size.y = 48.0;
            style.spacing.item_spacing = egui::vec2(12.0, 15.0);
            style.spacing.button_padding = egui::vec2(8.0, 8.0);
        });
    }

    fn spawn_preview_task(&self, path: String, ctx: &egui::Context) {
        let tx = self.tx.clone();
        let ctx_clone = ctx.clone();
        let path_clone = path.clone();
        std::thread::spawn(move || {
            if let Some(img) = Self::task_generate_preview(path_clone.clone()) {
                let _ = tx.send((path_clone, img));
                ctx_clone.request_repaint();
            }
        });
    }

    fn task_generate_preview(path: String) -> Option<egui::ColorImage> {
        if !Path::new(&path).exists() { return None; }
        let pipeline_str = format!(
            "filesrc location=\"{}\" ! decodebin ! videoconvert ! videoscale ! video/x-raw,width=640,height=360,format=RGBA ! appsink name=sink sync=true max-buffers=1 drop=true",
            path
        );
        let pipeline = gst::parse::launch(&pipeline_str).ok()?.dynamic_cast::<gst::Pipeline>().ok()?;
        pipeline.set_state(gst::State::Paused).ok()?;

        let _ = pipeline.state(gst::ClockTime::from_seconds(2));
        let _ = pipeline.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, gst::ClockTime::from_seconds(1));

        let sink = pipeline.by_name("sink")?.dynamic_cast::<gst_app::AppSink>().ok()?;
        let sample = sink.pull_preroll().ok().or_else(|| sink.pull_sample().ok())?;
        let buffer = sample.buffer()?;
        let map = buffer.map_readable().ok()?;

        let color_image = egui::ColorImage::from_rgba_unmultiplied([640, 360], map.as_slice());
        let _ = pipeline.set_state(gst::State::Null);
        Some(color_image)
    }

    fn add_to_history(&mut self, path: String) {
        if path.is_empty() || !Path::new(&path).exists() { return; }
        self.config.recent_videos.retain(|v| v != &path);
        self.config.recent_videos.insert(0, path);
        self.config.recent_videos.truncate(4);
        self.config.save();
    }

    fn refresh_monitors(&mut self) {
        let mut detected = Vec::new();
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains('-') && !name.contains("render") {
                    if let Ok(status) = fs::read_to_string(entry.path().join("status")) {
                        if status.trim() == "connected" {
                            detected.push(name.splitn(2, '-').nth(1).unwrap_or(&name).to_string());
                        }
                    }
                }
            }
        }
        self.monitors = detected;
        if !self.monitors.contains(&self.config.monitor_name) {
            self.config.monitor_name = self.monitors.first().cloned().unwrap_or_default();
        }
        self.config.save();
    }
}

impl eframe::App for ConfigApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok((path, img)) = self.rx.try_recv() {
            let tex = ctx.load_texture(&path, img, Default::default());
            if path == self.config.video_path {
                self.preview_texture = Some(tex.clone());
            }
            self.history_textures.insert(path, tex);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(ctx.style().visuals.widgets.noninteractive.bg_fill).inner_margin(25.0))
            .show(ctx, |ui| {
                let scroll_height = ui.available_height() - 110.0;

                egui::ScrollArea::vertical()
                    .max_height(scroll_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            let main_rect = egui::vec2(ui.available_width(), 180.0);
                            if let Some(texture) = &self.preview_texture {
                                ui.add(egui::Image::from_texture(texture).max_height(180.0).rounding(12.0));
                            } else {
                                let (rect, _) = ui.allocate_at_least(main_rect, egui::Sense::hover());
                                ui.painter().rect_filled(rect, 12.0, egui::Color32::from_rgb(25, 25, 30));
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "⏳", egui::FontId::proportional(32.0), egui::Color32::GRAY);
                            }
                        });

                        ui.add_space(15.0);

                        ui.label(egui::RichText::new("VIDEO SELECCIONADO").size(14.0).strong().color(egui::Color32::GRAY));
                        ui.horizontal(|ui| {
                            let edit = egui::TextEdit::singleline(&mut self.config.video_path)
                                .hint_text("Selecciona un video...")
                                .margin(egui::Margin { left: 10.0, right: 10.0, top: 13.0, bottom: 13.0 });

                            if ui.add_sized([ui.available_width() - 60.0, 48.0], edit).changed() {
                                self.config.save();
                            }

                            if ui.add_sized([48.0, 48.0], egui::Button::new("📂")).clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_file() {
                                    let p = path.display().to_string();
                                    self.config.video_path = p.clone();
                                    self.add_to_history(p.clone());
                                    self.spawn_preview_task(p, ctx);
                                }
                            }
                        });

                        if !self.config.recent_videos.is_empty() {
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("HISTORIAL").size(14.0).strong().color(egui::Color32::GRAY));
                            ui.add_space(5.0);

                            egui::Grid::new("hist_grid").spacing([10.0, 10.0]).show(ui, |ui| {
                                let mut to_remove = None;
                                let recent = self.config.recent_videos.clone();
                                let fixed_size = egui::vec2(200.0, 112.0);

                                for (i, path) in recent.iter().enumerate() {
                                    if !Path::new(path).exists() {
                                        to_remove = Some(path.clone());
                                        continue;
                                    }

                                    let full_name = Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();

                                    let btn_resp = if let Some(tex) = self.history_textures.get(path) {
                                        let img = egui::Image::from_texture(tex)
                                            .fit_to_exact_size(fixed_size)
                                            .rounding(8.0);
                                        ui.add(egui::ImageButton::new(img))
                                    } else {
                                        ui.add_sized(fixed_size, egui::Button::new(egui::RichText::new("⏳").size(24.0)).rounding(8.0))
                                    };

                                    if btn_resp.on_hover_text(full_name).clicked() {
                                        self.config.video_path = path.clone();
                                        if let Some(tex) = self.history_textures.get(path) {
                                            self.preview_texture = Some(tex.clone());
                                        }
                                        self.add_to_history(path.clone());
                                    }

                                    if (i + 1) % 2 == 0 { ui.end_row(); }
                                }

                                if let Some(path) = to_remove {
                                    self.config.recent_videos.retain(|v| v != &path);
                                    self.config.save();
                                }
                            });
                        }

                        ui.add_space(15.0);

                        ui.label(egui::RichText::new("PANTALLA").size(14.0).strong().color(egui::Color32::GRAY));
                        ui.horizontal(|ui| {
                            let combo = egui::ComboBox::from_id_source("m_sel")
                                .selected_text(egui::RichText::new(&self.config.monitor_name).size(14.0))
                                .width(ui.available_width() - 60.0);

                            let res = combo.show_ui(ui, |ui| {
                                let mut changed = false;
                                for m in &self.monitors {
                                    if ui.selectable_value(&mut self.config.monitor_name, m.clone(), m.clone()).clicked() {
                                        changed = true;
                                    }
                                }
                                changed
                            });

                            if res.inner.unwrap_or(false) {
                                self.config.save();
                            }

                            if ui.add_sized([48.0, 48.0], egui::Button::new("🔄")).clicked() { self.refresh_monitors(); }
                        });
                    });

                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(&self.status).italics().color(egui::Color32::GRAY).size(12.0));
                });
                ui.add_space(10.0);

                ui.scope(|ui| {
                    ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::from_rgb(49, 175, 145);
                    ui.visuals_mut().widgets.hovered.bg_fill = egui::Color32::from_rgb(60, 200, 170);
                    let btn = ui.add_sized([ui.available_width(), 55.0], egui::Button::new(egui::RichText::new("🚀 INICIAR FONDO").strong().size(18.0).color(egui::Color32::WHITE)));
                    if btn.clicked() && !self.config.video_path.is_empty() {
                        if Path::new(&self.config.video_path).exists() {
                            self.status = format!("Activo en {}", self.config.monitor_name);
                            self.config.save();
                            let _ = Command::new("cargo").args(["run", "--release", "--bin", "cosmic-bg-core", "--", &self.config.video_path, &self.config.monitor_name]).spawn();
                        }
                    }
                });
            });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([480.0, 780.0]).with_resizable(false),
        ..Default::default()
    };
    eframe::run_native("Cosmic Wallpaper", options, Box::new(|cc| Box::new(ConfigApp::new(cc))))
}