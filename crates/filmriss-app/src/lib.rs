//! Filmriss – Kamera-App mit Live-Filmsimulationen.
//!
//! UI: egui (Maus + Touch), Rendering: wgpu (Metal / Vulkan / DX12 / GL).
//! Die App ist als Bibliothek aufgebaut, damit dieselbe Codebasis später auch als
//! Android-`cdylib` (über `android_main`) gestartet werden kann.

mod app;
mod capture;
mod editor;
mod preview;
mod style;
mod thumbnails;

pub use app::FilmrissApp;

pub fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Filmriss")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([480.0, 400.0]),
        renderer: eframe::Renderer::Wgpu,
        // Kein MSAA nötig: das Preview-Dreieck füllt den Viewport komplett.
        multisampling: 0,
        ..Default::default()
    }
}

pub fn run(options: eframe::NativeOptions) -> eframe::Result {
    eframe::run_native("Filmriss", options, Box::new(|cc| Ok(Box::new(FilmrissApp::new(cc)?))))
}

/// Einstiegspunkt für Android (noch nicht gebaut/getestet, siehe docs/ARCHITECTURE.md).
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..native_options()
    };
    if let Err(e) = run(options) {
        log::error!("Filmriss beendet mit Fehler: {e}");
    }
}
