//! macOS 自搬移的平台实现。
//!
//! 设计取舍：这里**不使用 `objc2`/AppKit**，而是复用 macOS 自带命令
//! （`ditto` / `xattr` / `pgrep` / `open` / `hdiutil`）完成全部操作。
//! 原因有二：
//!
//! 1. 所有这些操作都是"跑一条命令"，用命令实现比 FFI 绑定更短、更易审计；
//! 2. 本仓库主力开发环境是 Windows，`#[cfg(target_os = "macos")]` 下的代码
//!    在本地无法编译验证。把决策逻辑抽到 `plan.rs`（跨平台、有单测），
//!    平台层只保留极薄的命令封装，可以把"未经编译验证的代码量"压到最小。
//!
//! 若后续需要更精细的运行时应用信息（如经 Launch Services 查询、而非进程名
//! 匹配），可再引入 `objc2-app-kit` 的 `NSWorkspace` 替换 `is_bundle_running`。

use super::plan;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const DITTO: &str = "/usr/bin/ditto";
const XATTR: &str = "/usr/bin/xattr";
const PGREP: &str = "/usr/bin/pgrep";
const OPEN: &str = "/usr/bin/open";
const HDIDUTIL: &str = "/usr/bin/hdiutil";
const SHELL: &str = "/bin/sh";
const LSREGISTER: &str =
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

/// 执行一次自搬移尝试。
///
/// 返回 `Some(())` 表示**调用方应立即退出当前进程**（已从新位置重启，或已激活
/// 目标位置的在用副本）；返回 `None` 表示无需搬移或搬移失败，调用方应继续正常启动。
pub fn relocate_if_needed() -> Option<()> {
    let exe = std::env::current_exe().ok()?;
    let bundle = plan::app_bundle_from_exe(&exe)?;

    // 仅在易失路径下才动作；已安装在稳定位置的用户零开销、零行为变化。
    if !plan::is_volatile_bundle_path(&bundle) {
        return None;
    }
    log(&format!("检测到从易失路径运行: {}", bundle.display()));

    let source_volume = plan::mounted_volume_for_bundle(&bundle).or_else(|| find_mounted_volume_for_app(&bundle));

    // 1) 目标已存在且正在运行：激活已有实例，绝不覆盖在用的副本。
    if let Some(target) = plan::system_target(&bundle)
        && target.exists()
        && is_bundle_running(&target)
    {
        log("目标副本正在运行，激活已有实例");
        launch(&target, false);
        eject(source_volume);
        return Some(());
    }

    // 2) 装到 /Applications。目录不可写时 ditto 会失败，自然进入降级分支。
    if let Some(target) = plan::system_target(&bundle)
        && install(&bundle, &target)
    {
        log(&format!("已安装到 {}", target.display()));
        launch(&target, true);
        eject(source_volume);
        return Some(());
    }

    // 3) 降级到 ~/Applications（用户级、免授权、不弹密码框）。
    if let Some(home) = home_dir()
        && let Some(target) = plan::user_target(&bundle, &home)
    {
        if let Some(dir) = target.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if install(&bundle, &target) {
            log(&format!("已降级安装到 {}", target.display()));
            launch(&target, true);
            eject(source_volume);
            return Some(());
        }
    }

    // 4) 全部失败：不阻断启动，从原路径继续运行。
    log("自搬移失败，将从原路径继续运行");
    None
}

/// 把 `.app` 安装到 `target`：拷贝 → 清隔离属性 → 注册 Launch Services。
fn install(source: &Path, target: &Path) -> bool {
    // 覆盖前先移除旧副本。正在运行的副本已在上层排除，不会走到这里；
    // 若旧副本属主为 root 导致删除失败，则本次安装失败并触发降级。
    if target.exists() && std::fs::remove_dir_all(target).is_err() {
        log(&format!("无法移除旧副本: {}", target.display()));
        return false;
    }

    // 用 ditto 而非 cp -R：完整保留资源分支与扩展属性，避免破坏已签名 bundle。
    if !run(Command::new(DITTO).arg(source).arg(target)) {
        log(&format!("拷贝失败: {} -> {}", source.display(), target.display()));
        return false;
    }

    // 清隔离属性，否则下次启动又会被关进 App Translocation 随机沙盒。
    let _ = run(Command::new(XATTR).args(["-dr", "com.apple.quarantine"]).arg(target));

    // 进程内拷贝不像 Finder 拖拽那样通知 Launch Services，需显式注册，
    // 否则 Spotlight / 启动台可能搜不到；失败仅记录，不阻断安装。
    if !run(Command::new(LSREGISTER).arg("-f").arg(target)) {
        log("lsregister 注册失败（不阻断安装）");
    }

    true
}

/// 判断 `bundle` 内的主可执行文件是否正在运行。
///
/// 用 `pgrep -f` 匹配**可执行文件的完整路径**：比按名字匹配更精确，
/// 且当前进程自身的命令行是易失路径，不会误判自己。
fn is_bundle_running(bundle: &Path) -> bool {
    let Some(exe) = plan::executable_in_bundle(bundle) else {
        return false;
    };
    Command::new(PGREP)
        .arg("-f")
        .arg(exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// 从新路径启动应用；`new_instance` 为真时强制拉起新实例。
fn launch(path: &Path, new_instance: bool) {
    let mut cmd = Command::new(OPEN);
    if new_instance {
        cmd.arg("-n");
    }
    let _ = run(cmd.arg(path));
}

/// 延迟并强制卸载源 DMG 卷。
///
/// 延后 3 秒是为了等新实例真正启动、Finder 收起 DMG 窗口，
/// 避免抢在拷贝完成前卸载而触发 I/O 错误。后台执行，不阻塞当前进程退出。
fn eject(volume: Option<PathBuf>) {
    let Some(volume) = volume else {
        return;
    };
    let script = format!(
        "sleep 3 && {HDIDUTIL} detach {} -force >/dev/null 2>&1",
        shell_quote(&volume)
    );
    let _ = Command::new(SHELL)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// App Translocation 场景下无法从自身路径反推卷名，改为枚举已挂载镜像，
/// 找出挂载点中含本应用名的 `/Volumes/<X>`。
fn find_mounted_volume_for_app(bundle: &Path) -> Option<PathBuf> {
    let stem = bundle.file_stem()?.to_string_lossy().into_owned();
    let output = Command::new(HDIDUTIL).arg("info").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .flat_map(str::split_whitespace)
        .filter(|token| token.starts_with(plan::VOLUMES_PREFIX))
        .map(PathBuf::from)
        .find(|path| path.to_string_lossy().contains(&stem))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| !home.as_os_str().is_empty())
}

/// 以单引号包裹路径，避免路径含空格时破坏 shell 命令。
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

fn run(cmd: &mut Command) -> bool {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// 该调用发生在日志系统初始化之前，故直接写 stderr（会进入系统统一日志）。
fn log(message: &str) {
    eprintln!("[relocate] {message}");
}
