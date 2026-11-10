#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use avcwt::app::SyncPlayApp;
use avcwt::server::SyncServer;

fn print_usage() {
    println!(
        "AVCWT - Synchronized Media Playback (Rust)\n\n\
         Usage:\n  \
           avcwt                      Launch the desktop GUI client (with built-in server option)\n  \
           avcwt --server [OPTIONS]   Run the standalone headless WebSocket sync server\n\n\
         Server Options:\n  \
           --host <HOST>              Bind address (default: 0.0.0.0)\n  \
           --port <PORT>              Bind port (default: 8765)\n  \
           -h, --help                 Show this help message"
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut server_mode = false;
    let mut host = "0.0.0.0".to_string();
    let mut port: u16 = 8765;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--server" | "-s" => {
                server_mode = true;
            }
            "--host" => {
                if let Some(h) = args.get(i + 1) {
                    host = h.clone();
                    i += 1;
                }
            }
            "--port" | "-p" => {
                if let Some(p) = args.get(i + 1).and_then(|s| s.parse::<u16>().ok()) {
                    port = p;
                    i += 1;
                }
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    if server_mode {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        rt.block_on(async move {
            let server = SyncServer::new(&host, port);
            server.start().await
        })?;
        return Ok(());
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AVCWT")
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([480.0, 360.0]),
        ..Default::default()
    };

    eframe::run_native(
        "AVCWT",
        native_options,
        Box::new(|cc| {
            SyncPlayApp::apply_dark_fusion_theme(&cc.egui_ctx);
            Ok(Box::new(SyncPlayApp::new()))
        }),
    )
    .map_err(|e| format!("Failed to start GUI: {e}").into())
}
