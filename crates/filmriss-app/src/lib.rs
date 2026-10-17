//! Filmriss – camera app with live film simulations.
//!
//! UI: egui (mouse + touch), rendering: wgpu (Metal / Vulkan / DX12 / GL).
//! The app is structured as a library so the same codebase can later also be launched
//! as an Android `cdylib` (via `android_main`).

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
        // No MSAA needed: the preview triangle covers the whole viewport.
        multisampling: 0,
        ..Default::default()
    }
}

pub fn run(options: eframe::NativeOptions) -> eframe::Result {
    eframe::run_native("Filmriss", options, Box::new(|cc| Ok(Box::new(FilmrissApp::new(cc)?))))
}

/// Entry point for Android (not built/tested yet, see docs/ARCHITECTURE.md).
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..native_options()
    };
    if let Err(e) = run(options) {
        log::error!("Filmriss exited with error: {e}");
    }
}
