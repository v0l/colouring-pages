pub mod app;
pub mod backend;
pub mod clean;
pub mod export;
pub mod jobs;
#[cfg(test)]
mod fake;
pub mod llm;
pub mod net;
pub mod outputs;
pub mod platform;
pub mod pipeline;
pub mod plan;
pub mod settings;
pub mod store;
pub mod style;
pub mod system;
pub mod voice;

#[cfg(not(target_os = "android"))]
pub fn run_native() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([480.0, 480.0])
        .with_title("Colouring Pages")
        .with_app_id("colouring-pages");
    eframe::run_native(
        "colouring-pages",
        eframe::NativeOptions { viewport, wgpu_options: wgpu_options(), ..Default::default() },
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

#[cfg(target_os = "ios")]
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew, wgpu};
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.device_descriptor = std::sync::Arc::new(|adapter| wgpu::DeviceDescriptor {
        label: Some("colouring pages"),
        required_limits: adapter.limits(),
        ..Default::default()
    });
    WgpuConfiguration { wgpu_setup: WgpuSetup::CreateNew(setup), ..Default::default() }
}

#[cfg(all(not(target_os = "ios"), not(target_os = "android")))]
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    Default::default()
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    platform::remember_activity(app.activity_as_ptr());
    if let Some(dir) = app.internal_data_path() {
        settings::set_data_dir(dir);
    }
    let opts = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    let _ = eframe::run_native(
        "colouring-pages",
        opts,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    );
}
