//! 以管理员身份重启应用（仅 Windows）。
//!
//! 流程刻意分为"先启动提权实例，成功后再退出"，原因见 design.md 决策 3：
//! `prepare_exit()` 是幂等且会在清理失败时"取消退出"的，若先标记退出再提权，
//! 用户取消 UAC 或提权失败时应用会卡在"正在退出"的中间态；而若先清理再提权，
//! 失败时又会留下"代理已关、TUN 已关、应用还在"的坏状态。

/// 以管理员身份重启的结果，供命令层决定是否退出进程。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminRestartOutcome {
    /// 已发起提权重启：新实例已启动且退出清理已完成，调用方应立即退出当前进程
    Relaunched,
    /// 当前已经是管理员，无需重启，应用继续运行
    AlreadyElevated,
}

/// 以管理员身份重启应用。
///
/// - 仅 Windows 支持，其他平台返回错误（由命令层决定是否暴露入口）。
/// - 当前已是管理员时直接返回 [`AdminRestartOutcome::AlreadyElevated`]，不重启。
/// - 提权启动成功后才会走 `prepare_exit()` 完成退出清理；
///   用户取消 UAC 或启动失败时**不做任何清理**，应用保持运行（返回错误）。
/// - 本函数不退出进程：返回 [`AdminRestartOutcome::Relaunched`] 后由调用方负责退出，
///   这样单例端口在退出前已释放，提权实例可以顺利用退避重试接管。
pub async fn restart_as_admin() -> anyhow::Result<AdminRestartOutcome> {
    #[cfg(not(target_os = "windows"))]
    {
        anyhow::bail!("run as administrator is only supported on Windows")
    }

    #[cfg(target_os = "windows")]
    {
        use crate::utils::elevate::{self, ElevateError};
        use anyhow::Context as _;
        use clash_verge_logging::{Type, logging};

        if elevate::is_process_elevated() {
            logging!(info, Type::System, "当前已是管理员模式，跳过提权重启");
            return Ok(AdminRestartOutcome::AlreadyElevated);
        }

        let exe_path = std::env::current_exe().context("获取当前可执行文件路径失败")?;

        logging!(info, Type::System, "以管理员身份启动新实例: {exe_path:?}");

        match elevate::launch_elevated(&exe_path, &[elevate::ELEVATED_FLAG]) {
            Ok(()) => {}
            Err(ElevateError::Cancelled) => {
                logging!(info, Type::System, "用户取消了 UAC 提权提示，保持当前实例运行");
                // 新实例没有启动，绝对不能执行退出清理，否则会留下"代理已关但应用还在"的坏状态。
                anyhow::bail!("用户取消了管理员权限请求");
            }
            Err(err) => {
                logging!(error, Type::System, "提权启动失败: {err}");
                anyhow::bail!("无法以管理员身份重启：{err}");
            }
        }

        // 到这里提权实例已经拉起来了，现在才执行退出清理。若清理失败
        // （prepare_exit 返回 false 并已 cancel_exit），不要强制退出，直接返回错误，
        // 由前端提示用户重试，避免"新实例已起、旧实例未退"长期双开。
        if !super::window::prepare_exit().await {
            logging!(
                error,
                Type::System,
                "提权实例已启动，但退出清理未完成，保持当前实例运行"
            );
            anyhow::bail!("提权已发起，但退出清理未完成，请重试");
        }

        logging!(info, Type::System, "退出清理完成，等待调用方退出当前实例");
        Ok(AdminRestartOutcome::Relaunched)
    }
}
