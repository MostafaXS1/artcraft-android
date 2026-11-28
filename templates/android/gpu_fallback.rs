//! Android GPU backend selection and crash-safe GLES fallback for PowerVR and
//! unstable Vulkan drivers.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

static STARTUP_MARKER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Tracks a pending startup so a native crash or `eframe::run_native` error
/// before the first rendered frame falls back to OpenGL ES on the next launch.
pub struct StartupGuard {
    state_dir: PathBuf,
    vulkan_enabled: bool,
}

impl StartupGuard {
    /// Clears the startup marker and persists GLES fallback if `eframe::run_native`
    /// returned an error while Vulkan was enabled.
    pub fn record_error(&self) {
        mark_ok();
        if self.vulkan_enabled {
            let _ = fs::write(self.state_dir.join("gpu-backend.txt"), b"gl\n");
        }
    }
}

/// Clears the startup marker once the first UI frame renders.
pub fn mark_ok() {
    if let Ok(mut guard) = STARTUP_MARKER.lock() {
        if let Some(marker) = guard.take() {
            let _ = fs::remove_file(marker);
        }
    }
}

fn is_powervr_device() -> bool {
    Path::new("/sys/module/pvrsrvkm").exists() || Path::new("/dev/pvrsrvkm").exists()
}

pub fn is_powervr_adapter(info: &eframe::wgpu::AdapterInfo) -> bool {
    if info.vendor == 0x1010 {
        return true;
    }
    let lower = info.name.to_ascii_lowercase();
    lower.contains("powervr") || lower.contains("rogue") || lower.contains("ge8322") || lower.contains("bxm")
}

/// Return-type adapter so `select_adapter` works with both `egui-wgpu` 0.34/0.35
/// (`Result<Option<Adapter>, String>`) and 0.36+ (`Result<Adapter, String>`).
pub trait AdapterSelectionResult: Sized {
    fn from_adapter(adapter: Result<eframe::wgpu::Adapter, String>) -> Self;
}

impl AdapterSelectionResult for Result<eframe::wgpu::Adapter, String> {
    fn from_adapter(adapter: Result<eframe::wgpu::Adapter, String>) -> Self {
        adapter
    }
}

impl AdapterSelectionResult for Result<Option<eframe::wgpu::Adapter>, String> {
    fn from_adapter(adapter: Result<eframe::wgpu::Adapter, String>) -> Self {
        adapter.map(Some)
    }
}

/// Selects a compatible Android adapter, preferring GLES over PowerVR Vulkan
/// while keeping Vulkan first on other hardware GPUs.
pub fn select_adapter<R: AdapterSelectionResult>(
    adapters: &[eframe::wgpu::Adapter],
    surface: Option<&eframe::wgpu::Surface<'_>>,
) -> R {
    let mut best: Option<(u8, &eframe::wgpu::Adapter)> = None;
    for adapter in adapters {
        if let Some(s) = surface {
            if !adapter.is_surface_supported(s) {
                continue;
            }
        }
        let info = adapter.get_info();
        let rank = match (info.device_type, info.backend) {
            (eframe::wgpu::DeviceType::Cpu, _) => 4,
            (_, eframe::wgpu::Backend::Vulkan) if is_powervr_adapter(&info) => 3,
            (_, eframe::wgpu::Backend::Vulkan) => 0,
            (_, eframe::wgpu::Backend::Gl) => 1,
            _ => 2,
        };
        if best.map_or(true, |(best_rank, _)| rank < best_rank) {
            best = Some((rank, adapter));
        }
    }
    R::from_adapter(
        best.map(|(_, a)| a.clone())
            .ok_or_else(|| "No compatible Android graphics adapter found".to_string()),
    )
}

/// Applies PowerVR GLES fallback, `select_adapter`, and the crash-safe startup
/// marker to `options.wgpu_options`.
pub fn configure_wgpu(options: &mut eframe::NativeOptions, state_dir: &Path) -> StartupGuard {
    let _ = fs::create_dir_all(state_dir);
    let marker = state_dir.join("gpu-startup.lock");
    let backend_pref = state_dir.join("gpu-backend.txt");
    let backends = if is_powervr_device() {
        log::info!("PowerVR GPU detected; using OpenGL ES backend for stability");
        eframe::wgpu::Backends::GL
    } else if fs::read_to_string(&backend_pref)
        .map(|s| s.trim().eq_ignore_ascii_case("gl"))
        .unwrap_or(false)
    {
        log::info!("Using persisted OpenGL ES backend");
        eframe::wgpu::Backends::GL
    } else if marker.exists() {
        log::warn!("Previous startup did not finish; falling back to OpenGL ES");
        let _ = fs::write(&backend_pref, b"gl\n");
        eframe::wgpu::Backends::GL
    } else {
        eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL
    };
    let vulkan_enabled = backends.contains(eframe::wgpu::Backends::VULKAN);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = backends;
        create.native_adapter_selector = Some(Arc::new(select_adapter));
    }
    let _ = fs::write(&marker, b"starting\n");
    if let Ok(mut guard) = STARTUP_MARKER.lock() {
        *guard = Some(marker);
    }
    StartupGuard {
        state_dir: state_dir.to_path_buf(),
        vulkan_enabled,
    }
}
