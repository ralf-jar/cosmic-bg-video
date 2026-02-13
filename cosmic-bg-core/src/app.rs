pub use std::num::NonZeroU32;
pub use gstreamer;
pub use gstreamer_video;
pub use gstreamer_app::AppSink;

use gstreamer::prelude::*;
use gstreamer_video::prelude::*;

pub use smithay_client_toolkit::{
    compositor::{CompositorState, CompositorHandler},
    output::{OutputState, OutputHandler},
    registry::{RegistryState, ProvidesRegistryState},
    shell::wlr_layer::{LayerShell, LayerSurface, LayerShellHandler, LayerSurfaceConfigure},
    shm::{Shm, ShmHandler, slot::SlotPool},
    seat::{SeatState, SeatHandler, Capability},
};

pub use wayland_client::{
    protocol::{wl_surface, wl_shm, wl_output, wl_seat},
    Connection, QueueHandle,
};

pub struct AppData {
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    pub compositor_state: CompositorState,
    pub layer_shell: LayerShell,
    pub shm_state: Shm,
    pub surface: wl_surface::WlSurface,
    pub layer_surface: Option<LayerSurface>, // Cambiado a Option para seguridad
    pub pipeline: gstreamer::Pipeline,
    pub appsink: AppSink,
    pub pool: Option<SlotPool>,
    pub width: u32,
    pub height: u32,
    pub configured: bool,
    pub exit: bool,
    pub old_pid: Option<u32>,
    pub pid_path: std::path::PathBuf,
}

impl LayerShellHandler for AppData {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, _ls: &LayerSurface, configure: LayerSurfaceConfigure, _serial: u32) {
        let width = NonZeroU32::new(configure.new_size.0).map_or(1920, |v| v.get());
        let height = NonZeroU32::new(configure.new_size.1).map_or(1080, |v| v.get());

        self.width = width;
        self.height = height;

        let video_aspect = 16.0 / 9.0;
        let screen_aspect = width as f64 / height as f64;

        let (target_w, target_h) = if screen_aspect > video_aspect {
            (width as i32, (width as f64 / video_aspect) as i32)
        } else {
            ((height as f64 * video_aspect) as i32, height as i32)
        };

        let caps = gstreamer::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .field("width", target_w)
            .field("height", target_h)
            .build();

        self.appsink.set_caps(Some(&caps));

        if let Some(pool) = &mut self.pool {
            let _ = pool.resize((self.width * self.height * 4) as usize);
        }

        if !self.configured {
            self.configured = true;
            draw_frame(self, qh);

            if let Some(pid) = self.old_pid.take() {
                std::thread::spawn(move || {
                    // Espera 2 segundos para asegurar que el buffer de video esté lleno y visible
                    std::thread::sleep(std::time::Duration::from_secs(2));

                    let _ = std::process::Command::new("kill")
                        .arg("-15")
                        .arg(pid.to_string())
                        .status();
                });
            }
        }
    }
}

impl CompositorHandler for AppData {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _time: u32) {
        if surface == &self.surface {
            draw_frame(self, qh);
        }
    }

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for AppData {
    fn output_state(&mut self) -> &mut OutputState { &mut self.output_state }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for AppData { fn shm_state(&mut self) -> &mut Shm { &mut self.shm_state } }

impl SeatHandler for AppData {
    fn seat_state(&mut self) -> &mut SeatState { &mut self.seat_state }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, _: Capability) {}
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, _: Capability) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState { &mut self.registry_state }
    smithay_client_toolkit::registry_handlers!(OutputState, SeatState);
}

fn draw_frame(app: &mut AppData, qh: &QueueHandle<AppData>) {
    if let Some(sample) = app.appsink.try_pull_sample(gstreamer::ClockTime::from_mseconds(10)) {
        if app.width == 0 || app.height == 0 { return; }

        let buffer = sample.buffer().expect("No buffer");
        let buffer_owned = buffer.to_owned();
        let caps = sample.caps().expect("No caps");
        let info = gstreamer_video::VideoInfo::from_caps(caps).expect("No video info");

        if let Ok(video_frame) = gstreamer_video::VideoFrame::from_buffer_readable(buffer_owned, &info) {
            let pool = app.pool.as_mut().unwrap();
            let dst_stride = (app.width * 4) as i32;

            if let Ok((buffer_wl, canvas)) = pool.create_buffer(
                app.width as i32,
                app.height as i32,
                dst_stride,
                wl_shm::Format::Argb8888
            ) {
                let src_data = video_frame.plane_data(0).unwrap();
                let src_stride = info.stride()[0] as usize;

                let vid_w = info.width() as usize;
                let vid_h = info.height() as usize;
                let app_w = app.width as usize;
                let app_h = app.height as usize;

                let offset_x = if vid_w > app_w { (vid_w - app_w) / 2 } else { 0 };
                let offset_y = if vid_h > app_h { (vid_h - app_h) / 2 } else { 0 };

                for i in 0..app_h {
                    let src_row = i + offset_y;
                    if src_row >= vid_h { break; }
                    let src_idx = (src_row * src_stride) + (offset_x * 4);
                    let dst_idx = i * (app_w * 4);
                    let copy_width_bytes = std::cmp::min(app_w * 4, src_stride - (offset_x * 4));

                    if src_idx + copy_width_bytes <= src_data.len() && dst_idx + copy_width_bytes <= canvas.len() {
                        canvas[dst_idx..dst_idx + copy_width_bytes]
                            .copy_from_slice(&src_data[src_idx..src_idx + copy_width_bytes]);
                    }
                }

                buffer_wl.attach_to(&app.surface).expect("Buffer attach failed");
                app.surface.damage_buffer(0, 0, app.width as i32, app.height as i32);
                app.surface.frame(qh, app.surface.clone());
                app.surface.commit();
            }
        }
    } else {
        app.surface.frame(qh, app.surface.clone());
        app.surface.commit();
    }
}

smithay_client_toolkit::delegate_compositor!(AppData);
smithay_client_toolkit::delegate_output!(AppData);
smithay_client_toolkit::delegate_shm!(AppData);
smithay_client_toolkit::delegate_seat!(AppData);
smithay_client_toolkit::delegate_layer!(AppData);
smithay_client_toolkit::delegate_registry!(AppData);
