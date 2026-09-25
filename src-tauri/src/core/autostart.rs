#[cfg(target_os = "windows")]
use crate::utils::schtasks;
use crate::{
    config::{Config, IVerge},
    core::handle::Handle,
};
use anyhow::Result;
use clash_verge_logging::{Type, logging};
#[cfg(not(target_os = "windows"))]
use tauri_plugin_autostart::ManagerExt as _;
#[cfg(target_os = "windows")]
use tauri_plugin_clash_verge_sysinfo::is_current_app_handle_admin;

static AUTO_LAUNCH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 仅处理新配置的一次性默认值，不重新注册已有用户的自启任务。
pub async fn init_auto_launch() -> Result<()> {
    let _guard = AUTO_LAUNCH_LOCK.lock().await;
    if Handle::global().is_exiting() {
        return Ok(());
    }
    let verge = Config::verge().await;
    let mut config = (*verge.latest_arc()).clone();
    if config.auto_launch_pending != Some(true) {
        return Ok(());
    }

    let result = apply_first_run_launch(&mut config, set_launch_enabled(true)).await;
    let mut changed = false;
    verge.edit_draft(|draft| {
        // 用户已操作开关时，由等待锁的设置流程应用其选择。
        if draft.auto_launch_pending == Some(true) {
            draft.auto_launch_pending = config.auto_launch_pending;
            draft.enable_auto_launch = config.enable_auto_launch;
            changed = true;
        }
    });
    if changed {
        verge.apply();
        let saved = verge.data_arc();
        saved.save_file().await?;
    }
    result
}

async fn apply_first_run_launch(config: &mut IVerge, enable: impl Future<Output = Result<()>> + Send) -> Result<()> {
    if config.auto_launch_pending != Some(true) {
        return Ok(());
    }
    config.auto_launch_pending = None;
    if config.enable_auto_launch != Some(true) {
        return Ok(());
    }
    if let Err(err) = enable.await {
        config.enable_auto_launch = Some(false);
        logging!(
            warn,
            Type::System,
            "首次开启自启失败，已恢复为关闭，可在设置中手动重试: {err}"
        );
        return Err(err);
    }
    logging!(info, Type::System, "首次运行已默认开启开机自启");
    Ok(())
}

pub async fn update_launch() -> Result<()> {
    let _guard = AUTO_LAUNCH_LOCK.lock().await;
    let enable_auto_launch = { Config::verge().await.latest_arc().enable_auto_launch };
    set_launch_enabled(enable_auto_launch.unwrap_or(false)).await
}

#[cfg_attr(not(target_os = "windows"), allow(clippy::unused_async))]
async fn set_launch_enabled(is_enable: bool) -> Result<()> {
    logging!(info, Type::System, "Setting auto-launch enabled state to: {is_enable}");

    #[cfg(target_os = "windows")]
    {
        let is_admin = is_current_app_handle_admin(Handle::app_handle());
        schtasks::set_auto_launch(is_enable, is_admin).await?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let app_handle = Handle::app_handle();
        let autostart_manager = app_handle.autolaunch();
        if is_enable {
            autostart_manager.enable()?;
        } else {
            autostart_manager.disable()?;
        }
    }

    Ok(())
}

pub fn get_launch_status() -> Result<bool> {
    #[cfg(target_os = "windows")]
    {
        let enabled = schtasks::is_auto_launch_enabled();
        if let Ok(status) = enabled {
            logging!(info, Type::System, "Auto-launch status (scheduled task): {status}");
        }
        enabled
    }

    #[cfg(not(target_os = "windows"))]
    {
        let app_handle = Handle::app_handle();
        let autostart_manager = app_handle.autolaunch();
        match autostart_manager.is_enabled() {
            Ok(status) => {
                logging!(info, Type::System, "Auto-launch status: {status}");
                Ok(status)
            }
            Err(e) => {
                logging!(error, Type::System, "Failed to get auto-launch status: {e}");
                Err(anyhow::anyhow!("Failed to get auto-launch status: {}", e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::apply_first_run_launch;
    use crate::config::IVerge;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn first_run_registers_only_once() -> anyhow::Result<()> {
        let mut config = IVerge::first_run_template();
        let calls = AtomicUsize::new(0);
        for _ in 0..2 {
            apply_first_run_launch(&mut config, async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await?;
            config = serde_yaml_ng::from_str(&serde_yaml_ng::to_string(&config)?)?;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(config.enable_auto_launch, Some(true));
        assert_eq!(config.auto_launch_pending, None);
        Ok(())
    }

    #[tokio::test]
    async fn existing_user_choices_are_never_reapplied() -> anyhow::Result<()> {
        let calls = AtomicUsize::new(0);
        for enabled in [None, Some(false), Some(true)] {
            let mut config = IVerge {
                enable_auto_launch: enabled,
                ..IVerge::default()
            };
            apply_first_run_launch(&mut config, async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await?;
            assert_eq!(config.enable_auto_launch, enabled);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn failed_registration_is_not_reported_as_enabled_or_retried() -> anyhow::Result<()> {
        let mut config = IVerge::first_run_template();
        assert!(
            apply_first_run_launch(&mut config, async { Err(anyhow::anyhow!("注册失败")) })
                .await
                .is_err()
        );
        config = serde_yaml_ng::from_str(&serde_yaml_ng::to_string(&config)?)?;
        let calls = AtomicUsize::new(0);
        apply_first_run_launch(&mut config, async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await?;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(config.enable_auto_launch, Some(false));
        assert_eq!(config.auto_launch_pending, None);
        Ok(())
    }

    #[tokio::test]
    async fn manual_disable_survives_restart() -> anyhow::Result<()> {
        let mut config = IVerge::first_run_template();
        apply_first_run_launch(&mut config, async { Ok(()) }).await?;
        config.patch_config(&IVerge {
            enable_auto_launch: Some(false),
            ..IVerge::default()
        });
        config = serde_yaml_ng::from_str(&serde_yaml_ng::to_string(&config)?)?;
        let calls = AtomicUsize::new(0);
        apply_first_run_launch(&mut config, async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await?;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(config.enable_auto_launch, Some(false));
        Ok(())
    }

    #[tokio::test]
    async fn disabled_pending_config_does_not_register() -> anyhow::Result<()> {
        let mut config = IVerge::first_run_template();
        config.enable_auto_launch = Some(false);
        let calls = AtomicUsize::new(0);
        apply_first_run_launch(&mut config, async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await?;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(config.enable_auto_launch, Some(false));
        assert_eq!(config.auto_launch_pending, None);
        Ok(())
    }
}
