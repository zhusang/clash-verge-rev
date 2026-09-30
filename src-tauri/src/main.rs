#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    // macOS「双击即安装」：若正从 DMG 挂载卷或 App Translocation 易失路径运行，
    // 先把自己安装到 /Applications（降级 ~/Applications）并从新位置重启。
    //
    // 必须放在 app_lib::run() 之前 —— 早于任何 Tauri / WebView 初始化。
    // 搬移成功时本函数返回 true，此时新实例已在稳定路径运行，当前进程直接退出；
    // 非 macOS 平台恒为 false，行为不变。
    if app_lib::utils::relocate::relocate_if_needed() {
        return;
    }

    #[cfg(feature = "tokio-trace")]
    console_subscriber::init();

    app_lib::run();
}
