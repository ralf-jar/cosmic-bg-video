// cosmic-bg-core/src/main.rs

mod state;
mod video;
mod app;

use state::AppState;
use app::AppData;
use std::fs;

use gstreamer::prelude::*;

use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    registry::RegistryState,
    shell::wlr_layer::{Layer, LayerShell, Anchor, KeyboardInteractivity},
    shm::Shm,
    seat::SeatState,
};
use wayland_client::{
    Connection,
    globals::registry_queue_init,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let is_daemon = args.iter().any(|arg| arg == "--daemon");

    // 1. Lógica de inicio automático (--restore)
    if args.len() == 1 || (args.get(1).map(|s| s.as_str()) == Some("--restore") && !is_daemon) {
        let state = AppState::load();
        for (monitor, video) in state.monitors {
            spawn_core_instance(&video, &monitor);
        }
        return;
    }

    // 2. Lógica de ejecución normal (Launcher)
    if args.len() < 3 { return; }
    let video_path = args[1].clone();
    let target_monitor = args[2].clone();

    // Guardar estado
    let mut state = AppState::load();
    state.monitors.insert(target_monitor.clone(), video_path.clone());
    state.save();

    // --- MODIFICACIÓN: Si no es daemon, lanza uno y cierra el actual ---
    if !is_daemon {
        spawn_core_instance(&video_path, &target_monitor);
        return;
    }
    // ------------------------------------------------------------------

    // Manejo de PID para transición fluida
    let pid_path = AppState::get_pid_path(&target_monitor);
    let old_pid = fs::read_to_string(&pid_path).ok()
        .and_then(|s| s.trim().parse::<u32>().ok());

    let _ = fs::write(&pid_path, std::process::id().to_string());

    // --- INICIO DEL MOTOR (Solo llega aquí si es --daemon) ---

    // Inicializar GStreamer (vía nuestro módulo video)
    let uri = format!("file://{}", video_path);
    let (pipeline, appsink) = video::init_pipeline(&uri);

    // Inicializar Wayland
    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    // Estados de SCTK
    let compositor_state = CompositorState::bind(&globals, &qh).expect("wl_compositor faltante");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer_shell faltante");
    let shm_state = Shm::bind(&globals, &qh).expect("wl_shm faltante");
    let output_state = OutputState::new(&globals, &qh);
    let seat_state = SeatState::new(&globals, &qh);

    let surface = compositor_state.create_surface(&qh);

    let mut app = AppData {
        registry_state: RegistryState::new(&globals),
        seat_state,
        output_state,
        compositor_state,
        layer_shell,
        shm_state,
        surface: surface.clone(),
        layer_surface: None,
        pipeline,
        appsink,
        pool: None,
        width: 0,
        height: 0,
        configured: false,
        exit: false,
        old_pid,
        pid_path
    };

    // Detectar monitores
    event_queue.roundtrip(&mut app).unwrap();
    event_queue.roundtrip(&mut app).unwrap();

    // Buscar el monitor objetivo
    let mut target_output = None;
    for output in app.output_state.outputs() {
        if let Some(info) = app.output_state.info(&output) {
            if info.name.as_deref() == Some(&target_monitor) {
                target_output = Some(output);
                break;
            }
        }
    }

    // Crear la superficie de capa (Layer Surface)
    let layer_surface = app.layer_shell.create_layer_surface(
        &qh, surface.clone(), Layer::Background, Some("video-wallpaper"), target_output.as_ref()
    );

    layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer_surface.set_exclusive_zone(-1);
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);

    surface.commit();

    // Inicializar Pool de memoria para frames
    let pool = smithay_client_toolkit::shm::slot::SlotPool::new(1920 * 1080 * 4, &app.shm_state)
        .expect("Falló pool init");

    app.layer_surface = Some(layer_surface);
    app.pool = Some(pool);

    // Arrancar video
    app.pipeline.set_state(gstreamer::State::Playing)
        .expect("No se pudo iniciar la reproducción de GStreamer");

    let bus = app.pipeline.bus().expect("No se pudo obtener el bus de GStreamer");

    println!("Motor iniciado correctamente para el monitor: {}", target_monitor);

    // 2. BUCLE PRINCIPAL (Evento de Wayland + Mensajes de GStreamer)
    while !app.exit {
        // Procesar mensajes de GStreamer
        while let Some(msg) = bus.pop() {
            match msg.view() {
                gstreamer::MessageView::Eos(..) => {
                    let _ = app.pipeline.seek_simple(
                        gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::KEY_UNIT,
                        gstreamer::ClockTime::ZERO,
                    );
                }
                gstreamer::MessageView::Error(err) => {
                    eprintln!("Error GStreamer: {}", err.error());
                    app.exit = true;
                }
                _ => (),
            }
        }

        if let Err(e) = event_queue.blocking_dispatch(&mut app) {
            eprintln!("Wayland dispatch error: {}", e);
            break;
        }
    }
}

// MODIFICACIÓN: Se añadió .arg("--daemon") para que el hijo ejecute el motor
fn spawn_core_instance(video: &str, monitor: &str) {
    let _ = std::process::Command::new(std::env::current_exe().unwrap())
        .arg(video)
        .arg(monitor)
        .arg("--daemon")
        .spawn();
}