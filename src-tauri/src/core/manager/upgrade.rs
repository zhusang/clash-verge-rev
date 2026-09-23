use super::CoreManager;
use crate::{
    config::Config,
    core::handle::Handle,
    utils::network::{NetworkManager, ProxyType},
};
use anyhow::{Context as _, Result, bail, ensure};
use clash_verge_logging::{Type, logging};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    future::Future,
    io::{Cursor, Read as _},
    path::Path,
    time::Duration,
};
use tauri_plugin_mihomo::IpcConnectionPool;
use tokio::fs;

const MAX_DOWNLOAD: usize = 64 * 1024 * 1024;
const MAX_BINARY: u64 = 128 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: usize,
    digest: Option<String>,
}

fn asset_prefix(os: &str, arch: &str, alpha: bool) -> Result<String> {
    let platform = match os {
        "windows" => "windows",
        "macos" => "darwin",
        "linux" => "linux",
        _ => bail!("此平台暂不支持在线升级内核"),
    };
    let cpu = match (os, arch, alpha) {
        ("macos", "x86_64", true) => "amd64-v1-go122",
        ("macos", "x86_64", false) => "amd64-v2-go122",
        ("macos", "aarch64", _) => "arm64-go122",
        (_, "x86_64", _) => "amd64-v2",
        (_, "aarch64", _) => "arm64",
        (_, "x86", _) => "386",
        ("linux", "arm", _) => "armv7",
        ("linux", "riscv64", _) => "riscv64",
        ("linux", "loongarch64", _) => "loong64",
        _ => bail!("此架构暂不支持在线升级内核"),
    };
    Ok(format!("mihomo-{platform}-{cpu}-"))
}

fn select_asset<'a>(release: &'a Release, prefix: &str, extension: &str, alpha: bool) -> Result<(&'a Asset, String)> {
    let matches: Vec<_> = release
        .assets
        .iter()
        .filter_map(|asset| {
            let version = asset.name.strip_prefix(prefix)?.strip_suffix(extension)?;
            let valid = if alpha {
                version.starts_with("alpha-")
            } else {
                version == release.tag_name
            };
            (valid && version.bytes().all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)))
                .then_some((asset, version.to_owned()))
        })
        .collect();
    ensure!(matches.len() == 1, "发布中没有唯一匹配当前平台的内核安装包");
    matches.into_iter().next().context("未找到内核安装包")
}

fn verify_digest(bytes: &[u8], expected: Option<&str>) -> Result<()> {
    let expected = expected
        .and_then(|s| s.strip_prefix("sha256:"))
        .context("发布附件缺少 SHA-256 校验值，已取消升级")?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    ensure!(actual == expected, "内核下载校验失败，已保留原内核");
    Ok(())
}

fn unpack(bytes: Vec<u8>, extension: &str) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    if extension == ".zip" {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
        // 不解压归档路径，仅提取根目录中的唯一内核程序。
        let names: Vec<_> = archive
            .file_names()
            .filter(|name| !name.contains(['/', '\\']) && name.ends_with(".exe"))
            .map(str::to_owned)
            .collect();
        ensure!(names.len() == 1, "内核归档内容不符合预期");
        let name = names.first().context("内核归档为空")?;
        archive.by_name(name)?.take(MAX_BINARY + 1).read_to_end(&mut output)?;
    } else {
        flate2::read::GzDecoder::new(Cursor::new(bytes))
            .take(MAX_BINARY + 1)
            .read_to_end(&mut output)?;
    }
    ensure!(
        !output.is_empty() && output.len() as u64 <= MAX_BINARY,
        "内核文件大小异常"
    );
    Ok(output)
}

async fn download(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    if let Some(length) = response.content_length() {
        ensure!(length <= limit as u64, "下载响应过大");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(bytes.len().saturating_add(chunk.len()) <= limit, "下载响应过大");
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn download_with_fallback(clients: &[reqwest::Client], url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut last_error = anyhow::anyhow!("没有可用的下载客户端");
    for client in clients {
        match download(client, url, limit).await {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                logging!(debug, Type::Core, "内核下载通道失败: {err:#}");
                last_error = err;
            }
        }
    }
    Err(last_error).context("无法下载内核，请检查 GitHub 连接后重试")
}

async fn prepare_download(core: &str, current_version: &str, staged: &Path) -> Result<String> {
    let alpha = core == "verge-mihomo-alpha";
    let endpoint = if alpha { "tags/Prerelease-Alpha" } else { "latest" };
    // 下载阶段旧内核仍在运行，优先使用本地代理，再尝试系统代理或直连。
    let network = NetworkManager::new();
    let mut clients = Vec::new();
    for proxy in [ProxyType::Localhost, ProxyType::System] {
        clients.push(
            network
                .create_request(proxy, Some(180), Some("DinoVPN-Core-Updater".into()), false)
                .await?,
        );
    }
    let metadata = download_with_fallback(
        &clients,
        &format!("https://api.github.com/repos/MetaCubeX/mihomo/releases/{endpoint}"),
        4 * 1024 * 1024,
    )
    .await?;
    let release: Release = serde_json::from_slice(&metadata)?;
    ensure!(
        release
            .tag_name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)),
        "发布标签无效"
    );
    let prefix = asset_prefix(std::env::consts::OS, std::env::consts::ARCH, alpha)?;
    let extension = if cfg!(windows) { ".zip" } else { ".gz" };
    let (asset, version) = select_asset(&release, &prefix, extension, alpha)?;
    ensure!(version != current_version, "already using latest version");
    ensure!(asset.size > 0 && asset.size <= MAX_DOWNLOAD, "内核下载大小异常");
    // 固定下载来源，不能通过发布元数据注入任意下载地址。
    let url = format!(
        "https://github.com/MetaCubeX/mihomo/releases/download/{}/{}",
        release.tag_name, asset.name
    );
    let bytes = download_with_fallback(&clients, &url, MAX_DOWNLOAD).await?;
    ensure!(bytes.len() == asset.size, "内核下载不完整");
    verify_digest(&bytes, asset.digest.as_deref())?;
    let binary = tokio::task::spawn_blocking(move || unpack(bytes, extension)).await??;
    fs::write(staged, binary).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(staged, std::fs::Permissions::from_mode(0o755)).await?;
    }
    let mut command = tokio::process::Command::new(staged);
    command.arg("-v").kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output()).await??;
    ensure!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .any(|v| v == version),
        "下载的内核无法运行或版本不匹配"
    );
    Ok(version)
}

pub(super) async fn wait_for_shutdown() -> Result<()> {
    let socket = crate::config::IClashTemp::guard_external_controller_ipc();
    for _ in 0..40 {
        #[cfg(windows)]
        let result = tokio::net::windows::named_pipe::ClientOptions::new().open(&socket);
        #[cfg(unix)]
        let result = tokio::net::UnixStream::connect(&socket).await;
        match result {
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Ok(());
            }
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    bail!("旧内核仍占用控制管道，已取消操作；请退出应用并重启系统后重试")
}

pub(super) async fn wait_for_ready(expected_version: Option<&str>) -> Result<()> {
    IpcConnectionPool::global()?.clear_pool();
    let check = async {
        loop {
            let result = async {
                let mihomo = Handle::mihomo().await;
                let version = mihomo.get_version().await?;
                if expected_version.is_some_and(|expected| expected != version.version) {
                    bail!("内核版本未切换");
                }
                mihomo.get_base_config().await?;
                mihomo.get_proxies().await?;
                mihomo.get_proxy_providers().await?;
                drop(mihomo);
                Ok::<(), anyhow::Error>(())
            }
            .await;
            match result {
                Ok(()) => return,
                Err(err) => logging!(debug, Type::Core, "等待内核就绪: {err}"),
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), check)
        .await
        .context("内核启动后未能提供兼容的配置和代理信息")
}

// 调用前必须确认旧内核已停止。注入启停操作，让文件交换与回滚可独立验证。
async fn activate_prepared<F, Fut, S, StopFut>(
    binary: &Path,
    staged: &Path,
    backup: &Path,
    mut start_and_check: F,
    stop_and_check: S,
) -> Result<()>
where
    F: FnMut(bool) -> Fut,
    Fut: Future<Output = Result<()>>,
    S: FnOnce() -> StopFut,
    StopFut: Future<Output = Result<()>>,
{
    if let Err(err) = fs::rename(binary, backup).await {
        start_and_check(true).await.context("备份失败且原内核未能重新启动")?;
        return Err(err).context("无法备份原内核");
    }
    let activation = async {
        fs::rename(staged, binary).await?;
        start_and_check(false).await
    }
    .await;
    if let Err(err) = activation {
        logging!(error, Type::Core, "升级失败，正在回滚: {err:#}");
        stop_and_check().await.context("升级失败且新内核无法停止；备份已保留")?;
        if fs::try_exists(binary).await? {
            fs::rename(binary, staged)
                .await
                .context("无法移走失败的新内核；备份已保留")?;
        }
        fs::rename(backup, binary).await.context("恢复旧内核失败；备份已保留")?;
        start_and_check(true)
            .await
            .context("旧内核已恢复，但启动或通信检查失败")?;
        return Err(err).context("升级失败，已恢复原内核");
    }
    Ok(())
}

impl CoreManager {
    /// 先下载验证，再由应用管理停止、替换和启动；不调用会自行重启的 /upgrade。
    pub async fn upgrade_core(&self) -> Result<()> {
        let _guard = self.lifecycle_lock.lock().await;
        ensure!(!Handle::global().is_exiting(), "应用正在退出");
        let core = Config::verge().await.latest_arc().get_valid_clash_core();
        let current_version = Handle::mihomo().await.get_version().await?.version;
        let binary =
            tauri::utils::platform::current_exe()?.with_file_name(format!("{core}{}", std::env::consts::EXE_SUFFIX));
        ensure!(fs::metadata(&binary).await?.is_file(), "找不到当前内核文件");
        let staged = binary.with_file_name(format!(
            "{core}-upgrade-{}{}",
            nanoid::nanoid!(),
            std::env::consts::EXE_SUFFIX
        ));
        // 写入权限在停止旧内核前检查；安装目录受保护时明确报错，不中断现有连接。
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .await
            .context(
                "无法写入内核安装目录；Windows 请以管理员身份启动 DinoVPN 后重试，其他平台请使用有写入权限的安装方式",
            )?;
        drop(file);
        let result = self.perform_upgrade(&core, &current_version, &binary, &staged).await;
        if let Err(err) = fs::remove_file(&staged).await
            && err.kind() != std::io::ErrorKind::NotFound
        {
            logging!(warn, Type::Core, "清理内核暂存文件失败: {err}");
        }
        Handle::refresh_clash();
        result
    }

    async fn perform_upgrade(&self, core: &str, current_version: &str, binary: &Path, staged: &Path) -> Result<()> {
        let version = prepare_download(core, current_version, staged).await?;
        ensure!(!Handle::global().is_exiting(), "应用正在退出，已取消升级");
        logging!(info, Type::Core, "准备升级 {core}: {current_version} -> {version}");
        self.stop_core_inner().await?;
        // 检测历史版本遗留的孤儿进程，不能再启动第二份内核。
        wait_for_shutdown().await?;
        let backup = binary.with_file_name(format!(
            "{core}-backup-{}{}",
            nanoid::nanoid!(),
            std::env::consts::EXE_SUFFIX
        ));
        let new_version = version.as_str();
        activate_prepared(
            binary,
            staged,
            &backup,
            |original| async move {
                self.start_core_inner().await?;
                wait_for_ready(Some(if original { current_version } else { new_version })).await
            },
            || async {
                self.stop_core_inner().await?;
                wait_for_shutdown().await
            },
        )
        .await?;
        logging!(
            info,
            Type::Core,
            "内核升级完成: {version}，原内核备份: {}",
            backup.display()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Asset, Release, activate_prepared, asset_prefix, select_asset, unpack, verify_digest};
    use anyhow::{Result, anyhow};
    use sha2::{Digest as _, Sha256};
    use std::{
        future::ready,
        io::{Cursor, Write as _},
    };
    use tokio::fs;

    #[test]
    fn checksum_is_required_and_validated() {
        let digest = format!("sha256:{:x}", Sha256::digest(b"core"));
        assert!(verify_digest(b"core", Some(&digest)).is_ok());
        assert!(verify_digest(b"bad", Some(&digest)).is_err());
        assert!(verify_digest(b"core", None).is_err());
    }

    #[test]
    fn selects_exact_platform_and_channel() -> Result<()> {
        let release = Release {
            tag_name: "v1.19.31".into(),
            assets: vec![
                Asset {
                    name: "mihomo-windows-amd64-v2-v1.19.31.zip".into(),
                    size: 1,
                    digest: None,
                },
                Asset {
                    name: "mihomo-windows-amd64-v3-v1.19.31.zip".into(),
                    size: 1,
                    digest: None,
                },
            ],
        };
        let prefix = asset_prefix("windows", "x86_64", false)?;
        assert_eq!(select_asset(&release, &prefix, ".zip", false)?.1, "v1.19.31");
        assert!(select_asset(&release, &prefix, ".zip", true).is_err());
        assert_eq!(asset_prefix("macos", "aarch64", false)?, "mihomo-darwin-arm64-go122-");
        Ok(())
    }

    #[test]
    fn invalid_archives_do_not_produce_binaries() {
        assert!(unpack(b"not a zip".to_vec(), ".zip").is_err());
        assert!(unpack(b"not gzip".to_vec(), ".gz").is_err());
    }

    #[test]
    fn rejects_ambiguous_or_wrong_release_assets() -> Result<()> {
        let mut release = Release {
            tag_name: "v1.19.31".into(),
            assets: vec![],
        };
        for _ in 0..2 {
            release.assets.push(Asset {
                name: "mihomo-windows-amd64-v2-v1.19.31.zip".into(),
                size: 1,
                digest: None,
            });
        }
        let prefix = asset_prefix("windows", "x86_64", false)?;
        assert!(select_asset(&release, &prefix, ".zip", false).is_err());
        release.assets.pop();
        release.tag_name = "v1.19.32".into();
        assert!(select_asset(&release, &prefix, ".zip", false).is_err());
        release.assets[0].name = "mihomo-windows-amd64-v2-alpha-123abc.zip".into();
        assert_eq!(select_asset(&release, &prefix, ".zip", true)?.1, "alpha-123abc");
        Ok(())
    }

    fn zip_bytes(names: &[&str]) -> Result<Vec<u8>> {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in names {
            archive.start_file(*name, zip::write::SimpleFileOptions::default())?;
            archive.write_all(b"core")?;
        }
        Ok(archive.finish()?.into_inner())
    }

    #[test]
    fn archive_requires_one_root_executable() -> Result<()> {
        assert_eq!(unpack(zip_bytes(&["mihomo.exe"])?, ".zip")?, b"core");
        for names in [vec!["../mihomo.exe"], vec!["dir\\mihomo.exe"], vec!["a.exe", "b.exe"]] {
            assert!(unpack(zip_bytes(&names)?, ".zip").is_err());
        }
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(b"core")?;
        assert_eq!(unpack(gzip.finish()?, ".gz")?, b"core");
        Ok(())
    }

    // 所有启停均为模拟操作，只在测试专用目录交换普通文件，不接触真实内核。
    #[tokio::test]
    async fn successful_activation_keeps_backup() -> Result<()> {
        let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../target"))?;
        let binary = dir.path().join("core");
        let staged = dir.path().join("staged");
        let backup = dir.path().join("backup");
        fs::write(&binary, b"old").await?;
        fs::write(&staged, b"new").await?;
        activate_prepared(
            &binary,
            &staged,
            &backup,
            |original| {
                assert!(!original);
                ready(Ok(()))
            },
            || ready(Err(anyhow!("成功时不应停止新内核"))),
        )
        .await?;
        assert_eq!(fs::read(&binary).await?, b"new");
        assert_eq!(fs::read(&backup).await?, b"old");
        assert!(!fs::try_exists(&staged).await?);
        Ok(())
    }

    #[tokio::test]
    async fn failed_activation_restores_original() -> Result<()> {
        let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../target"))?;
        let binary = dir.path().join("core");
        let staged = dir.path().join("staged");
        let backup = dir.path().join("backup");
        fs::write(&binary, b"old").await?;
        fs::write(&staged, b"new").await?;
        let mut starts = Vec::new();
        let result = activate_prepared(
            &binary,
            &staged,
            &backup,
            |original| {
                starts.push(original);
                ready(if original {
                    Ok(())
                } else {
                    Err(anyhow!("模拟通信失败"))
                })
            },
            || ready(Ok(())),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(starts, vec![false, true]);
        assert_eq!(fs::read(&binary).await?, b"old");
        assert_eq!(fs::read(&staged).await?, b"new");
        assert!(!fs::try_exists(&backup).await?);
        Ok(())
    }

    #[tokio::test]
    async fn failed_stop_preserves_backup_without_overwriting_running_binary() -> Result<()> {
        let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../target"))?;
        let binary = dir.path().join("core");
        let staged = dir.path().join("staged");
        let backup = dir.path().join("backup");
        fs::write(&binary, b"old").await?;
        fs::write(&staged, b"new").await?;
        let result = activate_prepared(
            &binary,
            &staged,
            &backup,
            |_| ready(Err(anyhow!("模拟启动失败"))),
            || ready(Err(anyhow!("模拟停止失败"))),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fs::read(&binary).await?, b"new");
        assert_eq!(fs::read(&backup).await?, b"old");
        Ok(())
    }

    #[tokio::test]
    async fn failed_file_swap_restores_original() -> Result<()> {
        let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../target"))?;
        let binary = dir.path().join("core");
        let staged = dir.path().join("missing");
        let backup = dir.path().join("backup");
        fs::write(&binary, b"old").await?;
        let mut starts = Vec::new();
        let result = activate_prepared(
            &binary,
            &staged,
            &backup,
            |original| {
                starts.push(original);
                ready(Ok(()))
            },
            || ready(Ok(())),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(starts, vec![true]);
        assert_eq!(fs::read(&binary).await?, b"old");
        Ok(())
    }
}
