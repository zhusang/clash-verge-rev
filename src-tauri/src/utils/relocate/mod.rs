//! macOS 自搬移安装（"双击即安装"）入口。
//!
//! 用户在 DMG 中双击 `.app` 时，应用会运行在**易失路径**（DMG 挂载卷，或被
//! Gatekeeper 重定向的 App Translocation 沙盒）。此时把自身拷贝到固定位置
//! （`/Applications`，不可写时降级到 `~/Applications`），再从新位置重启，
//! 从而免去"手动拖拽到应用程序"的步骤。
//!
//! 除体验外，这还修掉了一类真实故障：从 DMG 直接运行时，
//! - 开机自启会把 `/Volumes/...` 记进 LaunchAgent，DMG 卸载后自启失效；
//! - 内核等伴随二进制按可执行文件相对路径查找，DMG 卸载后不可用。
//!
//! **调用位置有硬性要求**：必须在 Tauri / WebView 初始化之前（本模块由
//! `main.rs` 在 `app_lib::run()` 之前调用）。该调用发生在日志系统初始化之前，
//! 因此内部日志直接写 stderr。

mod plan;
pub use plan::*;

#[cfg(target_os = "macos")]
mod macos;

/// 执行一次自搬移尝试。
///
/// 返回 `true` 表示调用方**应立即退出当前进程**：要么已从新位置重启了新实例，
/// 要么已激活目标位置的在用副本。返回 `false` 表示无需搬移或搬移失败，
/// 调用方应按正常流程继续启动。
///
/// 在非 macOS 平台恒为 `false`（空实现），使调用点无需平台分支。
#[cfg(target_os = "macos")]
pub fn relocate_if_needed() -> bool {
    macos::relocate_if_needed().is_some()
}

#[cfg(not(target_os = "macos"))]
#[inline]
pub const fn relocate_if_needed() -> bool {
    false
}
