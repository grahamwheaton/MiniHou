//! The window needs a wgpu backend compiled in. With eframe's default features off none is, and the app panics at start-up
//! ("No wgpu backend feature that is implemented for the target platform was enabled"): `--selftest` never opens a window
//! and does not see it, so this checks the build instead.

use eframe::egui_wgpu::wgpu::Instance;

#[test]
fn a_gpu_backend_is_compiled_in() {
    let enabled = Instance::enabled_backend_features();
    assert!(!enabled.is_empty(), "no wgpu backend is enabled in this build: the window would panic at start-up");
}
