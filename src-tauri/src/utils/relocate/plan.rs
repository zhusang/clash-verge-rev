//! macOS 自搬移的**纯逻辑**部分：不含任何平台 IO，可在任意平台编译与测试。
//!
//! 把路径推导与决策从平台实现中抽出来，是为了让核心规则可以在 CI（含非 macOS）
//! 上被真正编译并执行单元测试，而不是只躺在 `#[cfg(target_os = "macos")]` 后面
//! 从未被验证过。

use std::{path::Path, path::PathBuf};

/// macOS 挂载卷根目录前缀。
pub const VOLUMES_PREFIX: &str = "/Volumes/";
/// Gatekeeper 随机化运行路径（App Translocation）的标记。
pub const TRANSLOCATION_MARKER: &str = "/AppTranslocation/";
/// 系统级应用安装目录。
pub const SYSTEM_APPLICATIONS_DIR: &str = "/Applications";
/// 应用包内可执行文件所在子目录。
const BUNDLE_EXECUTABLE_SUBDIR: &str = "Contents/MacOS";
/// 应用包扩展名（含点）。
const BUNDLE_EXTENSION: &str = ".app";

/// 从可执行文件路径反推 `.app` 包路径。
///
/// 例：`/Applications/DinoVPN.app/Contents/MacOS/DinoVPN`
/// → `/Applications/DinoVPN.app`。
///
/// 取**最后一次**出现的 `.app/`，以免应用名本身含 `.app` 时切错；
/// 路径中不存在 `.app/`（如开发构建的 `target/debug/...`）时返回 `None`。
pub fn app_bundle_from_exe(exe: &Path) -> Option<PathBuf> {
    let exe = exe.to_str()?;
    let marker = format!("{BUNDLE_EXTENSION}/");
    let idx = exe.rfind(&marker)?;
    Some(PathBuf::from(&exe[..idx + BUNDLE_EXTENSION.len()]))
}

/// 当前运行位置是否为**易失**位置（DMG 挂载卷，或 Gatekeeper 的
/// App Translocation 沙盒）。只有这两种情况才需要自搬移。
pub fn is_volatile_bundle_path(bundle: &Path) -> bool {
    match bundle.to_str() {
        Some(path) => path.starts_with(VOLUMES_PREFIX) || path.contains(TRANSLOCATION_MARKER),
        None => false,
    }
}

/// 从 `/Volumes/<卷名>/...` 反推挂载卷根 `/Volumes/<卷名>`。
///
/// 用于搬移完成后卸载源 DMG。若不是从 `/Volumes/` 启动（例如 Translocation
/// 场景），返回 `None`，由平台实现改用枚举已挂载镜像的方式定位。
pub fn mounted_volume_for_bundle(bundle: &Path) -> Option<PathBuf> {
    let rest = bundle.to_str()?.strip_prefix(VOLUMES_PREFIX)?;
    let name = rest.split('/').next()?;
    if name.is_empty() {
        return None;
    }
    Some(PathBuf::from(format!("{VOLUMES_PREFIX}{name}")))
}

/// 目标副本在系统级应用目录下的路径。
pub fn system_target(bundle: &Path) -> Option<PathBuf> {
    let name = bundle.file_name()?;
    Some(Path::new(SYSTEM_APPLICATIONS_DIR).join(name))
}

/// 目标副本在用户级应用目录（`<home>/Applications`）下的路径。
///
/// `home` 由调用方注入，便于测试。
pub fn user_target(bundle: &Path, home: &Path) -> Option<PathBuf> {
    let name = bundle.file_name()?;
    Some(home.join("Applications").join(name))
}

/// 包内主可执行文件路径（`<bundle>/Contents/MacOS/<stem>`）。
///
/// 用于按精确路径判断"目标副本是否正在运行"。
pub fn executable_in_bundle(bundle: &Path) -> Option<PathBuf> {
    Some(bundle.join(BUNDLE_EXECUTABLE_SUBDIR).join(bundle.file_stem()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_bundle_from_executable_path() {
        let exe = Path::new("/Applications/DinoVPN.app/Contents/MacOS/DinoVPN");
        assert_eq!(
            app_bundle_from_exe(exe),
            Some(PathBuf::from("/Applications/DinoVPN.app"))
        );
    }

    #[test]
    fn derives_bundle_from_volumes_path() {
        let exe = Path::new("/Volumes/DinoVPN/DinoVPN.app/Contents/MacOS/DinoVPN");
        assert_eq!(
            app_bundle_from_exe(exe),
            Some(PathBuf::from("/Volumes/DinoVPN/DinoVPN.app"))
        );
    }

    #[test]
    fn derives_bundle_from_translocation_path() {
        let exe =
            Path::new("/private/var/folders/ab/cdef/T/AppTranslocation/1234-5678/d/DinoVPN.app/Contents/MacOS/DinoVPN");
        assert_eq!(
            app_bundle_from_exe(exe),
            Some(PathBuf::from(
                "/private/var/folders/ab/cdef/T/AppTranslocation/1234-5678/d/DinoVPN.app"
            ))
        );
    }

    #[test]
    fn uses_last_app_marker_when_name_contains_app() {
        // 应用名本身含 ".app" 时，必须切在最后一个 ".app/" 上。
        let exe = Path::new("/Volumes/V/My.app.app/Contents/MacOS/My.app");
        assert_eq!(app_bundle_from_exe(exe), Some(PathBuf::from("/Volumes/V/My.app.app")));
    }

    #[test]
    fn development_build_has_no_bundle() {
        let exe = Path::new("/Users/me/project/target/debug/clash-verge");
        assert_eq!(app_bundle_from_exe(exe), None);
    }

    #[test]
    fn volumes_and_translocation_are_volatile() {
        assert!(is_volatile_bundle_path(Path::new("/Volumes/DinoVPN/DinoVPN.app")));
        assert!(is_volatile_bundle_path(Path::new(
            "/private/var/folders/x/T/AppTranslocation/1/d/DinoVPN.app"
        )));
    }

    #[test]
    fn installed_locations_are_stable() {
        assert!(!is_volatile_bundle_path(Path::new("/Applications/DinoVPN.app")));
        assert!(!is_volatile_bundle_path(Path::new(
            "/Users/me/Applications/DinoVPN.app"
        )));
        // 边界：仅前缀相似但不是挂载卷根。
        assert!(!is_volatile_bundle_path(Path::new("/VolumesExtra/DinoVPN.app")));
    }

    #[test]
    fn mounted_volume_is_derived_only_for_volumes_paths() {
        assert_eq!(
            mounted_volume_for_bundle(Path::new("/Volumes/DinoVPN/DinoVPN.app")),
            Some(PathBuf::from("/Volumes/DinoVPN"))
        );
        assert_eq!(
            mounted_volume_for_bundle(Path::new("/private/var/folders/x/T/AppTranslocation/1/d/DinoVPN.app")),
            None
        );
        assert_eq!(mounted_volume_for_bundle(Path::new("/Applications/DinoVPN.app")), None);
    }

    #[test]
    fn derives_target_paths() {
        let bundle = Path::new("/Volumes/DinoVPN/DinoVPN.app");
        assert_eq!(system_target(bundle), Some(PathBuf::from("/Applications/DinoVPN.app")));
        assert_eq!(
            user_target(bundle, Path::new("/Users/me")),
            Some(PathBuf::from("/Users/me/Applications/DinoVPN.app"))
        );
    }

    #[test]
    fn derives_executable_inside_bundle() {
        assert_eq!(
            executable_in_bundle(Path::new("/Applications/DinoVPN.app")),
            Some(PathBuf::from("/Applications/DinoVPN.app/Contents/MacOS/DinoVPN"))
        );
    }
}
