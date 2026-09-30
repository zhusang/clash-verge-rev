use super::resolve;
use crate::{
    cmd::is_port_in_use,
    config::{Config, DEFAULT_PAC, IVerge},
    module::lightweight,
    process::AsyncHandler,
    utils::window_manager::WindowManager,
};
use anyhow::{Result, bail};
use clash_verge_logging::{Type, logging, logging_error};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use reqwest::ClientBuilder;
use smartstring::alias::String;
use std::time::Duration;
use tokio::sync::oneshot;
use warp::Filter as _;

#[derive(serde::Deserialize, Debug)]
struct QueryParam {
    param: String,
}

// 关闭 embedded server 的信号发送端
static SHUTDOWN_SENDER: OnceCell<Mutex<Option<oneshot::Sender<()>>>> = OnceCell::new();

/// check whether there is already exists
pub async fn check_singleton() -> Result<()> {
    let port = IVerge::get_singleton_port();
    if !is_port_in_use(port) {
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    if crate::utils::elevate::is_elevated_launch() {
        // 提权重启：旧实例是在新实例启动之后才执行退出清理并释放单例端口的，
        // 因此这里会短暂看到端口仍被占用。等待旧实例释放后继续启动，
        // 而不是直接判定"已有实例"并退出，否则提权重启会永远失败。
        wait_for_singleton_handoff(port).await?;
        // 等待期间旧实例已释放端口，说明当前是唯一实例，继续正常启动。
        if !is_port_in_use(port) {
            return Ok(());
        }
        // 仍被占用：确实是另一个实例在运行（超时兜底），按单例语义处理。
    }

    handle_existing_instance(port).await
}

/// 带退避地等待单例端口被旧实例释放（仅提权启动路径使用）。
#[cfg(target_os = "windows")]
async fn wait_for_singleton_handoff(port: u16) -> Result<()> {
    use crate::constants::timing;

    wait_for_singleton_handoff_with(port, timing::SINGLETON_HANDOFF_MAX, timing::SINGLETON_HANDOFF_INTERVAL).await
}

/// [`wait_for_singleton_handoff`] 的可注入超时版本，便于测试使用短超时。
#[cfg(target_os = "windows")]
async fn wait_for_singleton_handoff_with(port: u16, max_wait: Duration, interval: Duration) -> Result<()> {
    use std::time::Instant;

    let deadline = Instant::now() + max_wait;
    while Instant::now() < deadline {
        if !is_port_in_use(port) {
            logging!(info, Type::Window, "单例端口已释放，提权实例接管成功");
            return Ok(());
        }
        tokio::time::sleep(interval).await;
    }

    // 超时后交给调用方按"已有实例"处理，避免长期双实例并存。
    logging!(warn, Type::Window, "等待单例端口释放超时，按已有实例处理以避免双实例");
    Ok(())
}

/// 处理"已有实例"：转发深链/唤起窗口，然后让当前进程按单例语义结束启动流程。
async fn handle_existing_instance(port: u16) -> Result<()> {
    let client = ClientBuilder::new().timeout(Duration::from_millis(500)).build()?;
    // 收集真正的启动参数，跳过 `--elevated` 等内部标记，避免把标记当作深链转发。
    #[allow(clippy::needless_collect)]
    let args: Vec<std::string::String> = std::env::args().collect();
    let deep_link = args.iter().skip(1).find(|arg| !is_internal_flag(arg));

    match deep_link {
        // 有深链参数：转发给已有实例（仅处理 clash: 协议）
        Some(param) => {
            #[cfg(not(target_os = "macos"))]
            if param.starts_with("clash:") {
                client
                    .get(format!("http://127.0.0.1:{port}/commands/scheme?param={param}"))
                    .send()
                    .await?;
            }
            #[cfg(target_os = "macos")]
            let _ = param;
        }
        // 无深链参数（普通重复启动）：唤起已有实例的窗口
        None => {
            client
                .get(format!("http://127.0.0.1:{port}/commands/visible"))
                .send()
                .await?;
        }
    }

    logging!(error, Type::Window, "failed to setup singleton listen server");
    bail!("app exists");
}

/// 是否为应用内部的启动标记（不应作为深链参数转发）。
fn is_internal_flag(arg: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        arg == crate::utils::elevate::ELEVATED_FLAG
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = arg;
        false
    }
}

/// The embed server only be used to implement singleton process
/// maybe it can be used as pac server later
pub fn embed_server() {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    #[allow(clippy::expect_used)]
    SHUTDOWN_SENDER
        .set(Mutex::new(Some(shutdown_tx)))
        .expect("failed to set shutdown signal for embedded server");
    let port = IVerge::get_singleton_port();

    let visible = warp::path!("commands" / "visible").and_then(|| async {
        logging!(info, Type::Window, "检测到从单例模式恢复应用窗口");
        if !lightweight::exit_lightweight_mode().await {
            WindowManager::show_main_window().await;
        } else {
            logging!(error, Type::Window, "轻量模式退出失败，无法恢复应用窗口");
        };
        Ok::<_, warp::Rejection>(warp::reply::with_status::<std::string::String>(
            "ok".to_string(),
            warp::http::StatusCode::OK,
        ))
    });

    let pac = warp::path!("commands" / "pac").and_then(|| async move {
        let verge_config = Config::verge().await;
        let clash_config = Config::clash().await;

        let pac_content = verge_config
            .data_arc()
            .pac_file_content
            .clone()
            .unwrap_or_else(|| DEFAULT_PAC.into());

        let pac_port = verge_config
            .data_arc()
            .verge_mixed_port
            .unwrap_or_else(|| clash_config.data_arc().get_mixed_port());
        let processed_content = pac_content.replace("%mixed-port%", &format!("{pac_port}"));
        Ok::<_, warp::Rejection>(
            warp::http::Response::builder()
                .header("Content-Type", "application/x-ns-proxy-autoconfig")
                .body(processed_content)
                .unwrap_or_default(),
        )
    });

    // Use map instead of and_then to avoid Send issues
    let scheme = warp::path!("commands" / "scheme")
        .and(warp::query::<QueryParam>())
        .and_then(|query: QueryParam| async move {
            AsyncHandler::spawn(|| async move {
                logging_error!(Type::Setup, resolve::resolve_scheme(&query.param).await);
            });
            Ok::<_, warp::Rejection>(warp::reply::with_status::<std::string::String>(
                "ok".to_string(),
                warp::http::StatusCode::OK,
            ))
        });

    let commands = visible.or(scheme).or(pac);

    AsyncHandler::spawn(move || async move {
        warp::serve(commands)
            .bind(([127, 0, 0, 1], port))
            .await
            .graceful(async {
                shutdown_rx.await.ok();
            })
            .run()
            .await;
    });
}

pub fn shutdown_embedded_server() {
    logging!(info, Type::Window, "shutting down embedded server");
    if let Some(sender) = SHUTDOWN_SENDER.get()
        && let Some(sender) = sender.lock().take()
    {
        sender.send(()).ok();
    }
}

#[cfg(all(test, target_os = "windows"))]
#[allow(clippy::expect_used)]
mod tests {
    use super::wait_for_singleton_handoff_with;
    use std::net::TcpListener;
    use std::time::Duration;

    /// 旧实例在提权实例启动后才释放端口：等待函数必须在端口释放后返回，
    /// 从而让提权实例继续启动，而不是被判定为"已有实例"。
    #[tokio::test]
    async fn handoff_returns_once_port_is_released() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();

        let releaser = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(listener);
        });

        let started = std::time::Instant::now();
        let result = wait_for_singleton_handoff_with(port, Duration::from_secs(5), Duration::from_millis(20)).await;

        assert!(result.is_ok());
        // 端口释放即返回，不应等到超时。
        assert!(started.elapsed() < Duration::from_secs(3));
        releaser.await.expect("releaser task");
    }

    /// 端口始终被占用时，等待函数在超时后正常返回（由调用方按已有实例处理），
    /// 不会 panic 或永久阻塞。
    #[tokio::test]
    async fn handoff_times_out_without_panicking() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();

        let result = wait_for_singleton_handoff_with(port, Duration::from_millis(150), Duration::from_millis(20)).await;

        assert!(result.is_ok());
        drop(listener);
    }
}
