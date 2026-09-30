use std::time::Duration;

pub mod network {
    pub const DEFAULT_EXTERNAL_CONTROLLER: &str = "127.0.0.1:9097";

    pub mod ports {
        #[cfg(not(target_os = "windows"))]
        pub const DEFAULT_REDIR: u16 = 7895;
        #[cfg(target_os = "linux")]
        pub const DEFAULT_TPROXY: u16 = 7896;
        pub const DEFAULT_MIXED: u16 = 7897;
        pub const DEFAULT_SOCKS: u16 = 7898;
        pub const DEFAULT_HTTP: u16 = 7899;

        #[cfg(not(feature = "verge-dev"))]
        pub const SINGLETON_SERVER: u16 = 33331;
        #[cfg(feature = "verge-dev")]
        pub const SINGLETON_SERVER: u16 = 11233;
    }
}

pub mod timing {
    use super::Duration;

    pub const CONFIG_UPDATE_DEBOUNCE: Duration = Duration::from_millis(300);
    pub const STARTUP_ERROR_DELAY: Duration = Duration::from_secs(2);

    #[cfg(target_os = "windows")]
    pub const SERVICE_WAIT_MAX: Duration = Duration::from_millis(3000);
    #[cfg(target_os = "windows")]
    pub const SERVICE_WAIT_INTERVAL: Duration = Duration::from_millis(200);

    /// 提权实例接管单例端口的最长等待时间与轮询间隔。
    ///
    /// 注意：旧实例是在**退出清理的最后一步**才释放单例端口的，而该清理最长可能
    /// 耗时约 15s（系统代理 5s + TUN/内核 10s，见 `feat::window::clean_network`）。
    /// 因此这里的上限必须显著大于清理预算，否则提权实例会超时退出、
    /// 导致"旧实例已退、新实例也退"的零实例状态。
    #[cfg(target_os = "windows")]
    pub const SINGLETON_HANDOFF_MAX: Duration = Duration::from_millis(25000);
    #[cfg(target_os = "windows")]
    pub const SINGLETON_HANDOFF_INTERVAL: Duration = Duration::from_millis(100);
}

pub mod files {
    pub const RUNTIME_CONFIG: &str = "clash-verge.yaml";
    pub const CHECK_CONFIG: &str = "clash-verge-check.yaml";
    pub const DNS_CONFIG: &str = "dns_config.yaml";
    pub const WINDOW_STATE: &str = "window_state.json";
}

pub mod tun {
    pub const DEFAULT_STACK: &str = "gvisor";

    pub const DNS_HIJACK: &[&str] = &["any:53"];
}
