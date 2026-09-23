use super::{CoreManager, RunningMode};
use crate::cmd::StringifyErr as _;
use crate::config::{Config, IVerge};
use crate::core::handle::Handle;
use crate::core::manager::CLASH_LOGGER;
use crate::core::service::{SERVICE_MANAGER, ServiceStatus};
use crate::core::traffic_usage::TrafficUsageCollector;
use anyhow::Result;
use clash_verge_logging::{Type, logging};
use scopeguard::defer;
use smartstring::alias::String;
use tauri_plugin_clash_verge_sysinfo;

impl CoreManager {
    pub async fn start_core(&self) -> Result<()> {
        let _guard = self.lifecycle_lock.lock().await;
        self.start_core_inner().await
    }

    pub(super) async fn start_core_inner(&self) -> Result<()> {
        self.prepare_startup().await?;
        defer! {
            self.after_core_process();
        }

        match *self.get_running_mode() {
            RunningMode::Service => self.start_core_by_service().await,
            RunningMode::NotRunning | RunningMode::Sidecar => self.start_core_by_sidecar().await,
        }
    }

    pub async fn stop_core(&self) -> Result<()> {
        let _guard = self.lifecycle_lock.lock().await;
        self.stop_core_inner().await
    }

    pub(super) async fn stop_core_inner(&self) -> Result<()> {
        CLASH_LOGGER.clear_logs().await;
        // The connections stream dies with the core; persist what we have first.
        let collector = TrafficUsageCollector::global();
        collector.stop_subscription();
        collector.flush_now().await;
        defer! {
            self.after_core_process();
        }

        match *self.get_running_mode() {
            RunningMode::Service => self.stop_core_by_service().await,
            RunningMode::Sidecar => {
                self.stop_core_by_sidecar();
                Ok(())
            }
            RunningMode::NotRunning => Ok(()),
        }
    }

    pub async fn restart_core(&self) -> Result<()> {
        let _guard = self.lifecycle_lock.lock().await;
        logging!(info, Type::Core, "Restarting core");
        self.stop_core_inner().await?;
        super::upgrade::wait_for_shutdown().await?;
        self.start_core_inner().await?;
        super::upgrade::wait_for_ready(None).await
    }

    pub async fn change_core(&self, clash_core: &String) -> Result<(), String> {
        let guard = self.lifecycle_lock.lock().await;
        if !IVerge::VALID_CLASH_CORES.contains(&clash_core.as_str()) {
            return Err(format!("Invalid clash core: {}", clash_core).into());
        }

        Config::verge().await.edit_draft(|d| {
            d.clash_core = Some(clash_core.to_owned());
        });
        Config::verge().await.apply();

        let verge_data = Config::verge().await.latest_arc();
        verge_data.save_file().await.map_err(|e| e.to_string())?;
        drop(guard);

        self.update_config().await.stringify_err()?;
        Ok(())
    }

    async fn prepare_startup(&self) -> Result<()> {
        #[cfg(target_os = "windows")]
        self.wait_for_service_if_needed().await;

        let value = SERVICE_MANAGER.lock().await.current();
        let mode = match value {
            ServiceStatus::Ready => RunningMode::Service,
            _ => RunningMode::Sidecar,
        };

        self.set_running_mode(mode);
        Ok(())
    }

    fn after_core_process(&self) {
        let app_handle = Handle::app_handle();
        let mode = self.get_running_mode();
        tauri_plugin_clash_verge_sysinfo::set_app_core_mode(app_handle, mode.to_string());
        TrafficUsageCollector::global().notify_core_state(!matches!(*mode, RunningMode::NotRunning));
    }

    #[cfg(target_os = "windows")]
    async fn wait_for_service_if_needed(&self) {
        use crate::{config::Config, constants::timing, core::service};
        use backon::{ConstantBuilder, Retryable as _};

        let needs_service = Config::verge().await.latest_arc().enable_tun_mode.unwrap_or(false);

        if !needs_service {
            return;
        }

        let max_times = timing::SERVICE_WAIT_MAX.as_millis() / timing::SERVICE_WAIT_INTERVAL.as_millis();
        let backoff = ConstantBuilder::default()
            .with_delay(timing::SERVICE_WAIT_INTERVAL)
            .with_max_times(max_times as usize);

        let _ = (|| async {
            let mut manager = SERVICE_MANAGER.lock().await;

            if matches!(manager.current(), ServiceStatus::Ready) {
                return Ok(());
            }

            // If the service IPC path is not ready yet, treat it as transient and retry.
            // Running init/refresh too early can mark service state unavailable and break later config reloads.
            if !service::is_service_ipc_path_exists() {
                return Err(anyhow::anyhow!("Service IPC not ready"));
            }

            manager.init().await?;
            let _ = manager.refresh().await;

            if matches!(manager.current(), ServiceStatus::Ready) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("Service not ready"))
            }
        })
        .retry(backoff)
        .await;
    }
}
