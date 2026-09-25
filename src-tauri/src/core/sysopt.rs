use crate::{
    config::{Config, IVerge},
    core::handle::Handle,
    singleton,
};
use anyhow::Result;
use clash_verge_logging::{Type, logging};
use parking_lot::RwLock;
use smartstring::alias::String;
use std::{sync::Arc, time::Duration};
use sysproxy::{Autoproxy, Sysproxy};
use tokio::{sync::Mutex as TokioMutex, task::JoinHandle};

pub struct Sysopt {
    update_lock: TokioMutex<()>,
    inner_proxy: Arc<RwLock<(Sysproxy, Autoproxy)>>,
    guard: RwLock<Option<JoinHandle<()>>>,
}

impl Default for Sysopt {
    fn default() -> Self {
        Self {
            update_lock: TokioMutex::new(()),
            inner_proxy: Arc::new(RwLock::new((Sysproxy::default(), Autoproxy::default()))),
            guard: RwLock::new(None),
        }
    }
}

#[cfg(target_os = "windows")]
static DEFAULT_BYPASS: &str = "localhost;127.*;192.168.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;<local>";
#[cfg(target_os = "linux")]
static DEFAULT_BYPASS: &str = "localhost,127.0.0.1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12,::1";
#[cfg(target_os = "macos")]
static DEFAULT_BYPASS: &str =
    "127.0.0.1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12,localhost,*.local,*.crashlytics.com,<local>";

async fn get_bypass() -> String {
    let use_default = Config::verge().await.latest_arc().use_default_bypass.unwrap_or(true);
    let res = {
        let verge = Config::verge().await;
        let verge = verge.latest_arc();
        verge.system_proxy_bypass.clone()
    };
    let custom_bypass = match res {
        Some(bypass) => bypass,
        None => "".into(),
    };

    if custom_bypass.is_empty() {
        DEFAULT_BYPASS.into()
    } else if use_default {
        format!("{DEFAULT_BYPASS},{custom_bypass}").into()
    } else {
        custom_bypass
    }
}

singleton!(Sysopt, SYSOPT);

impl Sysopt {
    fn new() -> Self {
        Self::default()
    }

    fn stop_guard(&self) {
        let task = self.guard.write().take();
        if let Some(task) = task {
            task.abort();
        }
    }

    pub async fn refresh_guard(&self) {
        let _lock = self.update_lock.lock().await;
        self.stop_guard();
        if Handle::global().is_exiting() {
            return;
        }
        let verge = Config::verge().await.latest_arc();
        if !verge.enable_system_proxy.unwrap_or_default() || !verge.enable_proxy_guard.unwrap_or_default() {
            return;
        }
        let duration = Duration::from_secs(verge.proxy_guard_duration.unwrap_or(30).max(1));
        logging!(info, Type::Core, "启动系统代理守护，间隔: {}秒", duration.as_secs());
        *self.guard.write() = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(duration);
            loop {
                interval.tick().await;
                Self::global().guard_proxy().await;
            }
        }));
    }

    async fn guard_proxy(&self) {
        // 守护写入与关闭共用锁，关闭操作会等待已开始的守护写入完成。
        let _lock = self.update_lock.lock().await;
        if Handle::global().is_exiting() {
            return;
        }
        let (sys, auto) = &*self.inner_proxy.read();
        if sys.enable
            && let Ok(actual) = Sysproxy::get_system_proxy()
            && actual != *sys
            && let Err(e) = sys.set_system_proxy()
        {
            logging!(warn, Type::Core, "恢复系统代理失败: {e}");
        }
        if auto.enable
            && let Ok(actual) = Autoproxy::get_auto_proxy()
            && actual != *auto
            && let Err(e) = auto.set_auto_proxy()
        {
            logging!(warn, Type::Core, "恢复PAC代理失败: {e}");
        }
    }

    /// init the sysproxy
    #[allow(clippy::unused_async)]
    pub async fn update_sysproxy(&self) -> Result<()> {
        let _lock = self.update_lock.lock().await;
        if Handle::global().is_exiting() {
            return Ok(());
        }

        let verge = Config::verge().await.latest_arc();
        let port = match verge.verge_mixed_port {
            Some(port) => port,
            None => Config::clash().await.latest_arc().get_mixed_port(),
        };
        let pac_port = IVerge::get_singleton_port();
        let (sys_enable, pac_enable, proxy_host) = (
            verge.enable_system_proxy.unwrap_or_default(),
            verge.proxy_auto_config.unwrap_or_default(),
            verge.proxy_host.clone().unwrap_or_else(|| String::from("127.0.0.1")),
        );
        // 先 await, 避免持有锁导致的 Send 问题
        let bypass = get_bypass().await;

        let (sys, auto) = &mut *self.inner_proxy.write();
        sys.enable = false;
        sys.host = proxy_host.clone().into();
        sys.port = port;
        sys.bypass = bypass.into();

        auto.enable = false;
        auto.url = format!("http://{proxy_host}:{pac_port}/commands/pac");

        if !verge.enable_proxy_guard.unwrap_or_default() {
            self.stop_guard();
        }

        if !sys_enable && !pac_enable {
            // disable proxy
            sys.set_system_proxy()?;
            auto.set_auto_proxy()?;
            return Ok(());
        }

        if pac_enable {
            sys.enable = false;
            auto.enable = true;
            sys.set_system_proxy()?;
            auto.set_auto_proxy()?;
            return Ok(());
        }

        if sys_enable {
            auto.enable = false;
            sys.enable = true;
            auto.set_auto_proxy()?;
            sys.set_system_proxy()?;
            return Ok(());
        }

        Ok(())
    }

    /// reset the sysproxy
    #[allow(clippy::unused_async)]
    pub async fn reset_sysproxy(&self) -> Result<()> {
        let _lock = self.update_lock.lock().await;

        // close proxy guard
        self.stop_guard();

        // 直接关闭所有代理
        let (sys, auto) = &mut *self.inner_proxy.write();
        sys.enable = false;
        auto.enable = false;
        // 两种代理都要尝试关闭，不能因其中一种失败而跳过另一种。
        let sys_result = sys.set_system_proxy();
        let auto_result = auto.set_auto_proxy();
        sys_result?;
        auto_result?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Sysopt;

    #[tokio::test]
    async fn stopping_guard_cancels_a_pending_task() {
        let sysopt = Sysopt::default();
        let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
        *sysopt.guard.write() = Some(tokio::spawn(async move {
            std::future::pending::<()>().await;
            let _ = sender.send(());
        }));
        sysopt.stop_guard();
        assert!(sysopt.guard.read().is_none());
        assert!(receiver.await.is_err());
        sysopt.stop_guard();
    }
}
