//! ArtCraft Launcher updating itself.
//!
//! Delegated to [`craft_update::self_update`].
//! Not compiled for Android (self-update is handled by the Java Updater).

#[cfg(not(target_os = "android"))]
use std::ffi::OsString;

#[cfg(not(target_os = "android"))]
use artcraft_launcher_core::PackageKind;
#[cfg(not(target_os = "android"))]
use artcraft_launcher_core::platform::Platform;
#[cfg(not(target_os = "android"))]
use artcraft_launcher_core::protocol::JobPhase;
#[cfg(not(target_os = "android"))]
use artcraft_launcher_core::release::Release;
#[cfg(not(target_os = "android"))]
use artcraft_launcher_core::version::Version;

#[cfg(not(target_os = "android"))]
use crate::http::{Http, Stop};
#[cfg(not(target_os = "android"))]
use crate::paths::Paths;

#[cfg(not(target_os = "android"))]
pub use craft_update::self_update::{FINISH_FLAG, Install, PRETEND_VAR, is_self_updating};

/// The version this launcher counts as (see [`PRETEND_VAR`]).
#[cfg(not(target_os = "android"))]
pub fn current_version() -> Option<Version> {
    craft_update::self_update::current_version(&craft_update::suite::LAUNCHER, env!("CARGO_PKG_VERSION"))
}

/// How this copy is installed, if it's one the launcher knows how to replace.
#[cfg(not(target_os = "android"))]
pub fn current() -> Option<Install> {
    craft_update::self_update::current(&craft_update::suite::LAUNCHER)
}

#[cfg(not(target_os = "android"))]
fn to_update_paths(paths: &Paths) -> craft_update::Paths {
    craft_update::Paths { data: paths.data.clone(), cache: paths.cache.clone() }
}

#[cfg(not(target_os = "android"))]
fn to_update_release(release: &Release) -> craft_update::Release {
    craft_update::Release {
        tag: release.tag.clone(),
        version: release.version.clone(),
        name: release.name.clone(),
        published_at: release.published_at,
        html_url: release.html_url.clone(),
        notes: release.notes.clone(),
        prerelease: release.prerelease,
        assets: release
            .assets
            .iter()
            .map(|a| craft_update::Asset { name: a.name.clone(), size: a.size, url: a.url.clone(), sha256: a.sha256.clone() })
            .collect(),
    }
}

#[cfg(not(target_os = "android"))]
fn to_update_platform(platform: Platform) -> craft_update::Platform {
    // This function is only called on non-Android platforms, so Android is unreachable
    craft_update::Platform {
        os: match platform.os {
            artcraft_launcher_core::Os::Windows => craft_update::Os::Windows,
            artcraft_launcher_core::Os::MacOs => craft_update::Os::MacOs,
            artcraft_launcher_core::Os::Linux => craft_update::Os::Linux,
            artcraft_launcher_core::Os::FreeBsd => craft_update::Os::FreeBsd,
            // Android is not compiled on this target, so this arm is never reached
            _ => craft_update::Os::Linux,
        },
        arch: match platform.arch {
            artcraft_launcher_core::Arch::X64 => craft_update::Arch::X64,
            artcraft_launcher_core::Arch::Arm64 => craft_update::Arch::Arm64,
            artcraft_launcher_core::Arch::X86 => craft_update::Arch::X86,
            artcraft_launcher_core::Arch::Riscv64 => craft_update::Arch::Riscv64,
        },
    }
}

/// Whether `release` is the version this launcher just updated itself to.
#[cfg(not(target_os = "android"))]
pub fn just_updated_to(paths: &Paths, release: &Release) -> bool {
    let update_paths = to_update_paths(paths);
    let update_release = to_update_release(release);
    craft_update::self_update::just_updated_to(&update_paths, &update_release)
}

/// The error of the last update, if it failed, once.
#[cfg(not(target_os = "android"))]
pub fn take_failure(paths: &Paths) -> Option<String> {
    let update_paths = to_update_paths(paths);
    craft_update::self_update::take_failure(&update_paths)
}

/// Downloads `release` for `platform`, puts it in place and starts the helper that relaunches
/// it once this process has exited. On success the caller must quit promptly. `last` is called
/// once the download is ready and returns when the launcher may replace itself and restart.
#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
pub fn update(
    install: &Install,
    release: &Release,
    platform: Platform,
    _http: &Http,
    paths: &Paths,
    stop: &Stop,
    phase: &mut dyn FnMut(JobPhase),
    last: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    let update_release = to_update_release(release);
    let update_platform = to_update_platform(platform);
    let update_paths = to_update_paths(paths);
    let update_http = craft_update::Http::new("ArtCraft-Launcher/SelfUpdate")?;
    let mut update_phase = |p: craft_update::JobPhase| {
        phase(match p {
            craft_update::JobPhase::Downloading { done, total } => JobPhase::Downloading { done, total },
            craft_update::JobPhase::Reconnecting { done, total } => JobPhase::Reconnecting { done, total },
            craft_update::JobPhase::Waiting => JobPhase::Waiting,
            craft_update::JobPhase::Verifying => JobPhase::Verifying,
            craft_update::JobPhase::Installing => JobPhase::Installing,
            craft_update::JobPhase::Removing => JobPhase::Removing,
        });
    };
    craft_update::self_update::update(
        &craft_update::suite::LAUNCHER,
        install,
        &update_release,
        update_platform,
        &update_http,
        &update_paths,
        stop,
        &mut update_phase,
        last,
        Some(env!("CARGO_PKG_VERSION")),
    )
}

/// The package format updating `install` downloads: the one it was installed from. Mirrors the
/// private `Install::kind` in `craft_update::self_update`, which picks the update's asset by it.
#[cfg(not(target_os = "android"))]
pub fn kind(install: &Install) -> PackageKind {
    match install {
        Install::MacApp(_) => PackageKind::Dmg,
        Install::Msi(_) => PackageKind::Msi,
        Install::Portable(_) => PackageKind::PortableZip,
        Install::AppImage(_) => PackageKind::AppImage,
        Install::Package(kind, _) => match kind {
            craft_update::PackageKind::Msi => PackageKind::Msi,
            craft_update::PackageKind::PortableZip => PackageKind::PortableZip,
            craft_update::PackageKind::Dmg => PackageKind::Dmg,
            craft_update::PackageKind::AppImage => PackageKind::AppImage,
            craft_update::PackageKind::Deb => PackageKind::Deb,
            craft_update::PackageKind::Rpm => PackageKind::Rpm,
            craft_update::PackageKind::ArchPkg => PackageKind::ArchPkg,
            craft_update::PackageKind::Flatpak => PackageKind::Flatpak,
            craft_update::PackageKind::TarGz => PackageKind::TarGz,
        },
        Install::Flatpak => PackageKind::Flatpak,
        Install::Standalone(_) if cfg!(windows) => PackageKind::PortableZip,
        Install::Standalone(_) => PackageKind::TarGz,
    }
}

/// Finishes an update when this program was started as the update helper.
#[cfg(not(target_os = "android"))]
pub fn finish(args: &[OsString]) -> Option<Result<(), String>> {
    craft_update::self_update::finish(&craft_update::suite::LAUNCHER, args)
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;

    #[test]
    fn a_development_build_does_not_update_itself() {
        assert_eq!(current(), None);
    }

    #[test]
    fn an_update_downloads_the_format_the_copy_was_installed_from() {
        use std::path::PathBuf;
        assert_eq!(kind(&Install::Msi(PathBuf::new())), PackageKind::Msi);
        assert_eq!(kind(&Install::Portable(PathBuf::new())), PackageKind::PortableZip);
        assert_eq!(kind(&Install::MacApp(PathBuf::new())), PackageKind::Dmg);
        assert_eq!(kind(&Install::AppImage(PathBuf::new())), PackageKind::AppImage);
        assert_eq!(kind(&Install::Package(craft_update::PackageKind::Rpm, PathBuf::new())), PackageKind::Rpm);
        assert_eq!(kind(&Install::Flatpak), PackageKind::Flatpak);
        let standalone = if cfg!(windows) { PackageKind::PortableZip } else { PackageKind::TarGz };
        assert_eq!(kind(&Install::Standalone(PathBuf::new())), standalone);
    }
}
