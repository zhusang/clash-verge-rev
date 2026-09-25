use crate::config::Config;
use crate::core::{CoreManager, handle, sysopt};
use crate::module::lightweight;
use crate::utils;
use crate::utils::window_manager::WindowManager;
use clash_verge_logging::{Type, logging};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogKind};
use tokio::time::{Duration, timeout};

pub async fn open_or_close_dashboard() {
    if lightweight::is_in_lightweight_mode() {
        let _ = lightweight::exit_lightweight_mode().await;
        return;
    }

    let result = WindowManager::toggle_main_window().await;
    logging!(info, Type::Window, "Window toggle result: {result:?}");
}

pub async fn quit() {
    quit_with_code(0).await;
}

pub async fn quit_with_code(code: i32) {
    if prepare_exit().await {
        handle::Handle::app_handle().exit(code);
    }
}

pub async fn prepare_exit() -> bool {
    let handle = handle::Handle::global();
    if !handle.try_begin_exit() {
        logging!(debug, Type::System, "退出流程已在进行，忽略重复请求");
        return false;
    }

    logging!(info, Type::System, "退出前关闭系统代理和虚拟网卡");
    if !clean_async().await {
        handle.cancel_exit();
        logging!(error, Type::System, "网络清理未完成，已取消退出，可重试");
        handle::Handle::app_handle()
            .dialog()
            .message("未能完成系统代理或虚拟网卡的关闭，已取消退出。请检查日志后重试退出。")
            .title("DinoVPN")
            .kind(MessageDialogKind::Error)
            .show(|_| {});
        return false;
    }

    // 只清理运行状态，保留下次启动使用的开关偏好。
    Config::apply_all_and_save_file().await;
    utils::server::shutdown_embedded_server();
    handle.complete_exit_cleanup();
    true
}

pub async fn clean_async() -> bool {
    logging!(info, Type::System, "开始执行异步清理操作...");

    let network_task = tokio::task::spawn(clean_network(
        sysopt::Sysopt::global().reset_sysproxy(),
        CoreManager::global().shutdown_for_exit(),
    ));

    // DNS恢复（仅macOS）
    let dns_task = tokio::task::spawn(async {
        #[cfg(target_os = "macos")]
        match timeout(
            Duration::from_millis(1000),
            crate::utils::resolve::dns::restore_public_dns(),
        )
        .await
        {
            Ok(_) => {
                logging!(info, Type::Window, "DNS设置已恢复");
                true
            }
            Err(_) => {
                logging!(warn, Type::Window, "Warning: 恢复DNS设置超时");
                false
            }
        }
        #[cfg(not(target_os = "macos"))]
        true
    });

    // 并行执行清理任务
    let (network_result, dns_result) = tokio::join!(network_task, dns_task);

    let network_success = network_result.unwrap_or_default();
    let dns_success = dns_result.unwrap_or_default();

    let all_success = network_success && dns_success;

    logging!(
        info,
        Type::System,
        "异步关闭操作完成 - 网络: {}, DNS: {}, 总体: {}",
        network_success,
        dns_success,
        all_success
    );

    all_success
}

async fn clean_network(
    proxy: impl Future<Output = anyhow::Result<()>> + Send,
    core: impl Future<Output = anyhow::Result<()>> + Send,
) -> bool {
    // 先关闭系统代理，避免核心停止后系统仍将流量发往已关闭的端口。
    if !cleanup_step("系统代理", Duration::from_secs(5), proxy).await {
        return false;
    }
    cleanup_step("TUN和核心", Duration::from_secs(10), core).await
}

async fn cleanup_step(name: &str, limit: Duration, operation: impl Future<Output = anyhow::Result<()>> + Send) -> bool {
    match timeout(limit, operation).await {
        Ok(Ok(())) => {
            logging!(info, Type::System, "{name}已关闭");
            true
        }
        Ok(Err(e)) => {
            logging!(error, Type::System, "关闭{name}失败: {e}");
            false
        }
        Err(_) => {
            logging!(error, Type::System, "关闭{name}超时");
            false
        }
    }
}

#[cfg(target_os = "macos")]
pub async fn hide() {
    use crate::module::lightweight::add_light_weight_timer;

    let enable_auto_light_weight_mode = Config::verge()
        .await
        .data_arc()
        .enable_auto_light_weight_mode
        .unwrap_or(false);

    if enable_auto_light_weight_mode {
        add_light_weight_timer().await;
    }

    if let Some(window) = WindowManager::get_main_window()
        && window.is_visible().unwrap_or(false)
    {
        let _ = window.hide();
    }
    handle::Handle::global().set_activation_policy_accessory();
}

#[cfg(test)]
mod tests {
    use super::{clean_network, cleanup_step};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::Duration;

    #[tokio::test]
    async fn cleanup_disables_proxy_before_stopping_core() {
        let step = AtomicUsize::new(0);
        let success = clean_network(
            async {
                assert_eq!(step.fetch_add(1, Ordering::SeqCst), 0);
                Ok(())
            },
            async {
                assert_eq!(step.fetch_add(1, Ordering::SeqCst), 1);
                Ok(())
            },
        )
        .await;
        assert!(success);
        assert_eq!(step.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn proxy_failure_keeps_core_running() {
        let calls = AtomicUsize::new(0);
        let success = clean_network(async { Err(anyhow::anyhow!("代理关闭失败")) }, async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert!(!success);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn core_failure_is_not_reported_as_success() {
        assert!(!clean_network(async { Ok(()) }, async { Err(anyhow::anyhow!("停止核心失败")) }).await);
    }

    #[tokio::test]
    async fn cleanup_timeout_is_not_reported_as_success() {
        assert!(!cleanup_step("测试", Duration::from_millis(1), std::future::pending()).await);
    }
}
