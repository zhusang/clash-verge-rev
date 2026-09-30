//! Windows 提权重启支持。
//!
//! 与 `runas` crate 的关键差异：`runas` 通过 `ShellExecuteExW(runas)` 后会
//! `WaitForSingleObject(hProcess, INFINITE)` 阻塞到子进程结束，只适合服务安装器
//! 这类"等待型"子进程。启动 GUI 新实例必须"发射后不管"，否则当前进程会一直卡住，
//! 也就无法完成退出清理（释放单例端口）。
//!
//! 因此本模块直接调用 `ShellExecuteExW`，仅设置 `SEE_MASK_NOCLOSEPROCESS`
//! 拿到新进程句柄后立即返回，不等待、并关闭句柄避免泄漏。

/// 内部启动标记：提权新实例携带该参数启动，用于单例接管等待与自启级别同步。
#[cfg(target_os = "windows")]
pub const ELEVATED_FLAG: &str = "--elevated";

/// 提权重启的失败原因，供上层区分"用户取消"与真实错误。
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevateError {
    /// 用户在 UAC 提示处点了取消
    Cancelled,
    /// 其他 Win32 失败
    Win32(u32),
}

#[cfg(target_os = "windows")]
impl std::fmt::Display for ElevateError {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "user cancelled the UAC elevation prompt"),
            Self::Win32(code) => write!(f, "ShellExecuteExW failed with error code {code}"),
        }
    }
}

#[cfg(target_os = "windows")]
impl std::error::Error for ElevateError {}

/// 当前进程是否以管理员（高完整性）身份运行。
#[cfg(target_os = "windows")]
#[inline]
pub fn is_process_elevated() -> bool {
    use deelevate::{PrivilegeLevel, Token};
    Token::with_current_process()
        .and_then(|token| token.privilege_level())
        .map(|level| level != PrivilegeLevel::NotPrivileged)
        .unwrap_or(false)
}

/// 本次启动是否由提权重启产生（即命令行含 `--elevated`）。
#[cfg(target_os = "windows")]
#[inline]
pub fn is_elevated_launch() -> bool {
    std::env::args_os().any(|arg| arg == ELEVATED_FLAG)
}

/// 以管理员身份启动 `exe_path`，并附加 `extra_args`。
///
/// 该函数**不会**等待新进程退出：拿到句柄后关闭并立即返回，
/// 使调用方可以继续完成退出清理。用户取消 UAC 时返回 [`ElevateError::Cancelled`]。
#[cfg(target_os = "windows")]
pub fn launch_elevated(exe_path: &std::path::Path, extra_args: &[&str]) -> Result<(), ElevateError> {
    use std::{iter, os::windows::ffi::OsStrExt as _};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError},
        UI::{
            Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };

    /// 把 `OsStr` 转成以 NUL 结尾的宽字符序列（避免经 `String` 造成有损转换）。
    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(iter::once(0)).collect()
    }

    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe_path.as_os_str());

    // 参数需拼成单个命令行字符串，并按 Windows 规则加引号。
    let params_str = extra_args
        .iter()
        .map(|arg| format!("\"{arg}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let params = wide(std::ffi::OsStr::new(&params_str));

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: std::ptr::null_mut(),
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: if extra_args.is_empty() {
            std::ptr::null()
        } else {
            params.as_ptr()
        },
        lpDirectory: std::ptr::null(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };

    // SAFETY: `info` 是合法的、按 Win32 布局初始化的结构体，指向的宽字符串在
    // 调用期间保持存活（借用自上面的局部变量）。
    let launched = unsafe { ShellExecuteExW(&mut info) };
    if launched == 0 {
        // SAFETY: GetLastError 无参数、无前置条件。
        let code = unsafe { GetLastError() };
        return Err(if code == ERROR_CANCELLED {
            ElevateError::Cancelled
        } else {
            ElevateError::Win32(code)
        });
    }

    // 设置了 SEE_MASK_NOCLOSEPROCESS，句柄所有权转移给我们，必须显式关闭。
    if !info.hProcess.is_null() {
        // SAFETY: 句柄由 ShellExecuteExW 返回且非空，此处只关闭一次。
        unsafe {
            CloseHandle(info.hProcess);
        }
    }

    Ok(())
}
