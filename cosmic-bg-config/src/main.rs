use eframe::egui;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gst::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use crossbeam_channel::{unbounded, Receiver, Sender};
use tray_item::{IconSource, TrayItem};

// Usar la definición de estado de cosmic-bg-core para consistencia
use cosmic_bg_core::state::AppState as CoreAppState;

enum TrayMessage {
    OpenWindow,
    RemoveAll,
    Quit,
}

#[derive(Deserialize, Serialize, Default, Clone)]
struct AppConfig {
    video_path: String,
    monitor_name: String,
    recent_videos: Vec<String>,
}

impl AppConfig {
    fn config_path() -> PathBuf {
        dirs::config_dir()
            .expect("No se encontró carpeta config")
            .join("cosmic-bg/config.json")
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
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}

struct ConfigApp {
    config: AppConfig,
    status: String,
    monitors: Vec<String>,
    preview_texture: Option<egui::TextureHandle>,
    history_textures: HashMap<String, egui::TextureHandle>,
    // Canal para generar vistas previas
    preview_tx: Sender<(String, egui::ColorImage)>,
    preview_rx: Receiver<(String, egui::ColorImage)>,
    // Canal para recibir mensajes de la bandeja del sistema
    tray_rx: Receiver<TrayMessage>,
}

impl ConfigApp {
    fn new(cc: &eframe::CreationContext<'_>, tray_rx: Receiver<TrayMessage>) -> Self {
        gst::init().expect("Error al inicializar GStreamer");
        Self::setup_custom_style(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let config = AppConfig::load();
        let (preview_tx, preview_rx) = unbounded();

        let mut app = Self {
            config,
            status: "Listo para iniciar.".to_owned(),
            monitors: Vec::new(),
            preview_texture: None,
            history_textures: HashMap::new(),
            preview_tx,
            preview_rx,
            tray_rx,
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
        let tx = self.preview_tx.clone();
        let ctx_clone = ctx.clone();
        std::thread::spawn(move || {
            if let Some(img) = Self::task_generate_preview(path.clone()) {
                if tx.send((path, img)).is_ok() {
                    ctx_clone.request_repaint();
                }
            }
        });
    }

    fn task_generate_preview(path: String) -> Option<egui::ColorImage> {
        if !Path::new(&path).exists() { return None; }
        let pipeline_str = format!(
            "filesrc location=\"{}\" ! decodebin ! videoconvert ! videoscale ! video/x-raw,width=320,height=180,format=RGBA ! appsink name=sink sync=false max-buffers=1 drop=true",
            path
        );
        let pipeline = gst::parse::launch(&pipeline_str).ok()?.dynamic_cast::<gst::Pipeline>().ok()?;
        pipeline.set_state(gst::State::Paused).ok()?;

        let _ = pipeline.state(gst::ClockTime::from_seconds(2));
        
        // Buscar un frame válido, probando algunos segundos
        for i in 1..=5 {
            if pipeline.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, gst::ClockTime::from_seconds(i)).is_err() {
                break;
            }
            let sink = pipeline.by_name("sink")?.dynamic_cast::<gst_app::AppSink>().ok()?;
            if let Some(sample) = sink.try_pull_preroll(gst::ClockTime::from_seconds(1)).or_else(|| sink.try_pull_sample(gst::ClockTime::from_seconds(1))) {
                let buffer = sample.buffer()?;
                let map = buffer.map_readable().ok()?;
                let color_image = egui::ColorImage::from_rgba_unmultiplied([320, 180], map.as_slice());
                let _ = pipeline.set_state(gst::State::Null);
                return Some(color_image);
            }
        }
        
        let _ = pipeline.set_state(gst::State::Null);
        None
    }

    fn add_to_history(&mut self, path: String) {
        if path.is_empty() || !Path::new(&path).exists() { return; }
        self.config.recent_videos.retain(|v| v != &path);
        self.config.recent_videos.insert(0, path);
        self.config.recent_videos.truncate(4);
        self.config.save();
    }

    fn refresh_monitors(&mut self) {
        self.monitors = cosmic_bg_core::utils::get_connected_monitors();
        if !self.monitors.contains(&self.config.monitor_name) {
            self.config.monitor_name = self.monitors.first().cloned().unwrap_or_default();
        }
        self.config.save();
    }

    fn remove_all_videos(&mut self) {
        self.status = "Deteniendo todos los fondos...".to_string();
        
        // 1. Detener procesos
        let temp_dir = std::env::temp_dir();
        if let Ok(entries) = fs::read_dir(temp_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with("cosmic-bg-") && name.ends_with(".pid") {
                        if let Ok(pid_str) = fs::read_to_string(&path) {
                            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                                unsafe { libc::kill(pid, libc::SIGTERM); }
                            }
                        }
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }

        // 2. Limpiar el estado del core
        let mut core_state = CoreAppState::load();
        core_state.monitors.clear();
        core_state.save();
        
        self.status = "Todos los fondos han sido detenidos.".to_string();
    }
}

impl eframe::App for ConfigApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Manejar mensajes de la bandeja del sistema
        if let Ok(msg) = self.tray_rx.try_recv() {
            match msg {
                TrayMessage::OpenWindow => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                },
                TrayMessage::RemoveAll => self.remove_all_videos(),
                TrayMessage::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }



        // Recibir vistas previas generadas en segundo plano
        while let Ok((path, img)) = self.preview_rx.try_recv() {
            let tex = ctx.load_texture(&path, img, Default::default());
            if path == self.config.video_path {
                self.preview_texture = Some(tex.clone());
            }
            self.history_textures.insert(path, tex);
        }

        // --- DIBUJAR LA UI (código original adaptado) ---
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
                                self.spawn_preview_task(self.config.video_path.clone(), ctx);
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
                                let recent = self.config.recent_videos.clone();
                                let fixed_size = egui::vec2(200.0, 112.0);

                                for (i, path) in recent.iter().enumerate() {
                                    let full_name = Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();

                                    let btn_resp = if let Some(tex) = self.history_textures.get(path) {
                                        let img = egui::Image::from_texture(tex).fit_to_exact_size(fixed_size).rounding(8.0);
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
                            });
                        }

                        ui.add_space(15.0);

                        ui.label(egui::RichText::new("PANTALLA").size(14.0).strong().color(egui::Color32::GRAY));
                        ui.horizontal(|ui| {
                            let combo = egui::ComboBox::from_id_source("m_sel")
                                .selected_text(egui::RichText::new(&self.config.monitor_name).size(14.0))
                                .width(ui.available_width() - 60.0);

                            if combo.show_ui(ui, |ui| {
                                self.monitors.iter().any(|m| ui.selectable_value(&mut self.config.monitor_name, m.clone(), m.clone()).clicked())
                            }).inner.unwrap_or(false) {
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
                    if btn.clicked() && !self.config.video_path.is_empty() && Path::new(&self.config.video_path).exists() {
                        self.status = format!("Activo en {}", self.config.monitor_name);
                        self.config.save();

                        // --- FIX: Buscar el ejecutable de core relativo al actual ---
                        let mut core_path = std::env::current_exe().unwrap_or_default();
                        core_path.pop(); // Quita el nombre del ejecutable actual
                        core_path.push("cosmic-bg-core");

                        if core_path.exists() {
                            let _ = Command::new(&core_path)
                                .args([&self.config.video_path, &self.config.monitor_name])
                                .spawn();
                        } else {
                            // Fallback por si no lo encuentra (ej. en desarrollo con `cargo run`)
                            self.status = "Iniciando via Cargo...".to_string();
                            let _ = Command::new("cargo")
                                .args(["run", "--release", "--bin", "cosmic-bg-core", "--", &self.config.video_path, &self.config.monitor_name])
                                .spawn();
                        }
                    }
                });
            });
    }
}

fn setup_tray_icon(tx: Sender<TrayMessage>) {
    // Usar un ícono de GTK estándar. Para un ícono personalizado, se debe proveer un archivo.
    // La API de v0.9.0 usa un string para el nombre del ícono.
    let mut tray = TrayItem::new("Cosmic BG", IconSource::Resource("video-display")).unwrap();

    tray.add_label("Cosmic BG Control").unwrap();

    let open_tx = tx.clone();
    tray.add_menu_item("Abrir Configuración", move || {
        let _ = open_tx.send(TrayMessage::OpenWindow);
    }).unwrap();

    let remove_all_tx = tx.clone();
    tray.add_menu_item("Quitar Todos", move || {
        let _ = remove_all_tx.send(TrayMessage::RemoveAll);
    }).unwrap();

    // add_separator() no está disponible en v0.9.0

    let quit_tx = tx.clone();
    tray.add_menu_item("Cerrar Aplicación", move || {
        let _ = quit_tx.send(TrayMessage::Quit);
    }).unwrap();
}

fn main() -> eframe::Result<()> {
    let (tray_tx, tray_rx) = unbounded();
    
    // Iniciar el icono de la bandeja en un hilo separado
    std::thread::spawn(move || {
        setup_tray_icon(tray_tx);
    });
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 780.0])
            .with_resizable(false)
            .with_visible(false), // Iniciar invisible
        ..Default::default()
    };
    
    eframe::run_native(
        "Cosmic Wallpaper",
        options,
        Box::new(|cc| Box::new(ConfigApp::new(cc, tray_rx))),
    )
}
