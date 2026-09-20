use crate::{
    config::{Config, IVerge},
    core::{CoreManager, autostart, handle, hotkey, logger::Logger, sysopt, tray},
    module::{auto_backup::AutoBackupManager, lightweight},
};
use anyhow::Result;
use bitflags::bitflags;
use clash_verge_draft::SharedDraft;
use clash_verge_logging::{Type, logging, logging_error};
use serde_yaml_ng::Mapping;

/// Patch Clash configuration
pub async fn patch_clash(patch: &Mapping) -> Result<()> {
    Config::clash().await.edit_draft(|d| d.patch_config(patch));

    let res = {
        // 激活订阅
        if patch.get("secret").is_some() || patch.get("external-controller").is_some() {
            Config::generate().await?;
            CoreManager::global().restart_core().await?;
        } else {
            if patch.get("mode").is_some() {
                tray::Tray::global().update_menu_and_icon().await;
            }
            Config::runtime().await.edit_draft(|d| d.patch_config(patch));
            CoreManager::global().update_config().await?;
        }
        handle::Handle::refresh_clash();
        <Result<()>>::Ok(())
    };
    match res {
        Ok(()) => {
            Config::clash().await.apply();
            // 分离数据获取和异步调用
            let clash_data = Config::clash().await.data_arc();
            clash_data.save_config().await?;
            Ok(())
        }
        Err(err) => {
            Config::clash().await.discard();
            Err(err)
        }
    }
}

// Define update flags as bitflags for better performance
bitflags! {
     #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
     struct UpdateFlags: u16 {
        const RESTART_CORE = 1 << 0;
        const CLASH_CONFIG = 1 << 1;
        const VERGE_CONFIG = 1 << 2;
        const LAUNCH = 1 << 3;
        const SYS_PROXY = 1 << 4;
        const SYSTRAY_ICON = 1 << 5;
        const HOTKEY = 1 << 6;
        const SYSTRAY_MENU = 1 << 7;
        const SYSTRAY_TOOLTIP = 1 << 8;
        const SYSTRAY_CLICK_BEHAVIOR = 1 << 9;
        const LIGHT_WEIGHT = 1 << 10;
        const LANGUAGE = 1 << 11;
        const LOG_LEVEL = 1 << 12;
        const LOG_FILE = 1 << 13;

        const GROUP_SYS_TRAY = Self::SYSTRAY_MENU.bits()
                             | Self::SYSTRAY_TOOLTIP.bits()
                             | Self::SYSTRAY_ICON.bits();
     }
}

fn determine_update_flags(patch: &IVerge) -> UpdateFlags {
    let tun_mode = patch.enable_tun_mode;
    let auto_launch = patch.enable_auto_launch;
    let system_proxy = patch.enable_system_proxy;
    let pac = patch.proxy_auto_config;
    let pac_content = &patch.pac_file_content;
    let proxy_bypass = &patch.system_proxy_bypass;
    let language = &patch.language;
    let mixed_port = patch.verge_mixed_port;
    #[cfg(target_os = "macos")]
    let tray_icon = &patch.tray_icon;
    #[cfg(not(target_os = "macos"))]
    let tray_icon: Option<String> = None;
    let common_tray_icon = patch.common_tray_icon;
    let sysproxy_tray_icon = patch.sysproxy_tray_icon;
    let tun_tray_icon = patch.tun_tray_icon;
    #[cfg(not(target_os = "windows"))]
    let redir_enabled = patch.verge_redir_enabled;
    #[cfg(not(target_os = "windows"))]
    let redir_port = patch.verge_redir_port;
    #[cfg(target_os = "linux")]
    let tproxy_enabled = patch.verge_tproxy_enabled;
    #[cfg(target_os = "linux")]
    let tproxy_port = patch.verge_tproxy_port;
    let socks_enabled = patch.verge_socks_enabled;
    let socks_port = patch.verge_socks_port;
    let http_enabled = patch.verge_http_enabled;
    let http_port = patch.verge_port;
    let enable_tray_speed = patch.enable_tray_speed;
    // let enable_tray_icon = patch.enable_tray_icon;
    let enable_global_hotkey = patch.enable_global_hotkey;
    let tray_event = &patch.tray_event;
    let home_cards = patch.home_cards.as_ref();
    let enable_auto_light_weight = patch.enable_auto_light_weight_mode;
    let enable_external_controller = patch.enable_external_controller;
    let tray_proxy_groups_display_mode = &patch.tray_proxy_groups_display_mode;
    let tray_inline_outbound_modes = patch.tray_inline_outbound_modes;
    let enable_proxy_guard = patch.enable_proxy_guard;
    let proxy_guard_duration = patch.proxy_guard_duration;
    let log_level = &patch.app_log_level;
    let log_max_size = patch.app_log_max_size;
    let log_max_count = patch.app_log_max_count;

    #[cfg(target_os = "windows")]
    let restart_core_needed = socks_enabled.is_some()
        || http_enabled.is_some()
        || socks_port.is_some()
        || http_port.is_some()
        || mixed_port.is_some()
        || enable_external_controller.is_some();
    #[cfg(not(target_os = "windows"))]
    let mut restart_core_needed = socks_enabled.is_some()
        || http_enabled.is_some()
        || socks_port.is_some()
        || http_port.is_some()
        || mixed_port.is_some()
        || enable_external_controller.is_some();
    #[cfg(not(target_os = "windows"))]
    {
        restart_core_needed |= redir_enabled.is_some() || redir_port.is_some();
    }
    #[cfg(target_os = "linux")]
    {
        restart_core_needed |= tproxy_enabled.is_some() || tproxy_port.is_some();
    }

    let mut update_flags = UpdateFlags::empty();
    if restart_core_needed {
        update_flags.insert(UpdateFlags::RESTART_CORE);
    }
    if tun_mode.is_some() {
        update_flags.insert(UpdateFlags::CLASH_CONFIG | UpdateFlags::GROUP_SYS_TRAY);
    }
    if enable_global_hotkey.is_some() || home_cards.is_some() {
        update_flags.insert(UpdateFlags::VERGE_CONFIG);
    }
    if auto_launch.is_some() {
        update_flags.insert(UpdateFlags::LAUNCH);
    }
    if system_proxy.is_some() {
        update_flags.insert(UpdateFlags::SYS_PROXY | UpdateFlags::GROUP_SYS_TRAY);
    }
    if proxy_bypass.is_some()
        || pac_content.is_some()
        || pac.is_some()
        || enable_proxy_guard.is_some()
        || proxy_guard_duration.is_some()
    {
        update_flags.insert(UpdateFlags::SYS_PROXY);
    }
    if language.is_some() {
        update_flags.insert(UpdateFlags::LANGUAGE | UpdateFlags::SYSTRAY_MENU | UpdateFlags::SYSTRAY_TOOLTIP);
    }
    if common_tray_icon.is_some()
        || sysproxy_tray_icon.is_some()
        || tun_tray_icon.is_some()
        || tray_icon.is_some()
        || enable_tray_speed.is_some()
    {
        update_flags.insert(UpdateFlags::SYSTRAY_ICON);
    }
    if patch.hotkeys.is_some() {
        update_flags.insert(UpdateFlags::HOTKEY | UpdateFlags::SYSTRAY_MENU);
    }
    if tray_event.is_some() {
        update_flags.insert(UpdateFlags::SYSTRAY_CLICK_BEHAVIOR);
    }
    if enable_auto_light_weight.is_some() {
        update_flags.insert(UpdateFlags::LIGHT_WEIGHT);
    }
    if tray_proxy_groups_display_mode.is_some() {
        update_flags.insert(UpdateFlags::SYSTRAY_MENU);
    }
    if log_level.is_some() {
        update_flags.insert(UpdateFlags::LOG_LEVEL);
    }
    if log_max_size.is_some() || log_max_count.is_some() {
        update_flags.insert(UpdateFlags::LOG_FILE);
    }
    if tray_inline_outbound_modes.is_some() {
        update_flags.insert(UpdateFlags::SYSTRAY_MENU);
    }

    update_flags
}

#[allow(clippy::cognitive_complexity)]
async fn process_terminated_flags(update_flags: UpdateFlags, patch: &IVerge) -> Result<()> {
    // Process updates based on flags
    if update_flags.contains(UpdateFlags::RESTART_CORE) {
        Config::generate().await?;
        CoreManager::global().restart_core().await?;
    }
    if update_flags.contains(UpdateFlags::CLASH_CONFIG) {
        CoreManager::global().update_config().await?;
        handle::Handle::refresh_clash();
    }
    if update_flags.contains(UpdateFlags::VERGE_CONFIG) {
        Config::verge()
            .await
            .edit_draft(|d| d.enable_global_hotkey = patch.enable_global_hotkey);
        handle::Handle::refresh_verge();
    }
    if update_flags.contains(UpdateFlags::LAUNCH) {
        autostart::update_launch().await?;
    }
    if update_flags.contains(UpdateFlags::LANGUAGE)
        && let Some(language) = &patch.language
    {
        clash_verge_i18n::set_locale(language.as_str());
    }
    if update_flags.contains(UpdateFlags::SYS_PROXY) {
        sysopt::Sysopt::global().update_sysproxy().await?;
        sysopt::Sysopt::global().refresh_guard().await;
    }
    if update_flags.contains(UpdateFlags::HOTKEY)
        && let Some(hotkeys) = &patch.hotkeys
    {
        hotkey::Hotkey::global().update(hotkeys.to_owned()).await?;
    }
    if update_flags.contains(UpdateFlags::SYSTRAY_MENU) {
        tray::Tray::global().update_menu().await?;
    }
    if update_flags.contains(UpdateFlags::SYSTRAY_ICON) {
        tray::Tray::global()
            .update_icon(&Config::verge().await.latest_arc())
            .await?;
    }
    if update_flags.contains(UpdateFlags::SYSTRAY_TOOLTIP) {
        tray::Tray::global().update_tooltip().await?;
    }
    if update_flags.contains(UpdateFlags::SYSTRAY_CLICK_BEHAVIOR) {
        tray::Tray::global().update_click_behavior().await?;
    }
    if update_flags.contains(UpdateFlags::LIGHT_WEIGHT) {
        if patch.enable_auto_light_weight_mode.unwrap_or(false) {
            lightweight::enable_auto_light_weight_mode().await;
        } else {
            lightweight::disable_auto_light_weight_mode();
        }
    }
    if update_flags.contains(UpdateFlags::LOG_LEVEL) {
        Logger::global().update_log_level(patch.get_log_level())?;
    }
    if update_flags.contains(UpdateFlags::LOG_FILE) {
        let log_max_size = patch.app_log_max_size.unwrap_or(128);
        let log_max_count = patch.app_log_max_count.unwrap_or(8);
        Logger::global().update_log_config(log_max_size, log_max_count).await?;
    }
    Ok(())
}

/// Which proxy mode was forced off because the other one was turned on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForcedOff {
    SystemProxy,
    TunMode,
}

impl ForcedOff {
    /// Notice status consumed by the frontend notification handler.
    const fn notice_status(self) -> &'static str {
        match self {
            Self::SystemProxy => "proxy_mode::system_proxy_auto_disabled",
            Self::TunMode => "proxy_mode::tun_mode_auto_disabled",
        }
    }
}

/// System proxy and TUN mode are mutually exclusive.
///
/// When `patch` turns one of them on while the other one is currently on (or is
/// turned on by the same patch), the other one is forced off *inside the patch*
/// so that its side effects (clearing the OS proxy / reloading the core config)
/// run through the regular update-flag pipeline. When both are turned on by the
/// same patch, TUN wins.
///
/// Returns `None` when nothing needs to change.
fn resolve_exclusive_modes(patch: &IVerge, sysproxy_on: bool, tun_on: bool) -> Option<(IVerge, ForcedOff)> {
    let wants_sysproxy = patch.enable_system_proxy == Some(true);
    let wants_tun = patch.enable_tun_mode == Some(true);

    let forced = if wants_tun && (wants_sysproxy || (patch.enable_system_proxy.is_none() && sysproxy_on)) {
        ForcedOff::SystemProxy
    } else if wants_sysproxy && patch.enable_tun_mode.is_none() && tun_on {
        ForcedOff::TunMode
    } else {
        return None;
    };

    let mut resolved = patch.clone();
    match forced {
        ForcedOff::SystemProxy => resolved.enable_system_proxy = Some(false),
        ForcedOff::TunMode => resolved.enable_tun_mode = Some(false),
    }
    Some((resolved, forced))
}

pub async fn patch_verge(patch: &IVerge, not_save_file: bool) -> Result<()> {
    let resolved = {
        let current = Config::verge().await.latest_arc();
        resolve_exclusive_modes(
            patch,
            current.enable_system_proxy.unwrap_or(false),
            current.enable_tun_mode.unwrap_or(false),
        )
    };
    let forced_off = resolved.as_ref().map(|(_, forced)| *forced);
    let patch = resolved.as_ref().map_or(patch, |(resolved_patch, _)| resolved_patch);
    if let Some(forced) = forced_off {
        logging!(
            info,
            Type::ProxyMode,
            "System proxy and TUN mode are mutually exclusive, forcing off: {forced:?}",
        );
    }

    Config::verge().await.edit_draft(|d| d.patch_config(patch));

    let update_flags = determine_update_flags(patch);
    logging!(debug, Type::Setup, "Determined update flags: {:?}", update_flags);
    let process_flag_result: std::result::Result<(), anyhow::Error> = {
        process_terminated_flags(update_flags, patch).await?;
        Ok(())
    };

    if let Err(err) = process_flag_result {
        Config::verge().await.discard();
        return Err(err);
    }
    Config::verge().await.apply();
    if let Some(forced) = forced_off {
        // Tell the user which switch was turned off for them and make the
        // frontend re-read the verge config plus the real OS proxy state.
        handle::Handle::notice_message(forced.notice_status(), "");
        handle::Handle::refresh_verge();
    }
    logging_error!(Type::Backup, AutoBackupManager::global().refresh_settings().await);
    // A forced-off switch must always reach disk, even when the caller already
    // persisted the (pre-resolution) config itself, e.g. backup restore.
    if !not_save_file || forced_off.is_some() {
        // 分离数据获取和异步调用
        let verge_data = Config::verge().await.data_arc();
        logging!(debug, Type::Setup, "Saving Verge configuration to file...");
        verge_data.save_file().await?;
    }
    Ok(())
}

pub async fn fetch_verge_config() -> Result<SharedDraft<IVerge>> {
    let draft = Config::verge().await;
    let data = draft.data_arc();
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::{ForcedOff, IVerge, resolve_exclusive_modes};

    fn patch(sysproxy: Option<bool>, tun: Option<bool>) -> IVerge {
        IVerge {
            enable_system_proxy: sysproxy,
            enable_tun_mode: tun,
            ..IVerge::default()
        }
    }

    /// Returns the effective `(enable_system_proxy, enable_tun_mode, forced_off)`.
    fn resolve(patch: &IVerge, sysproxy_on: bool, tun_on: bool) -> (Option<bool>, Option<bool>, Option<ForcedOff>) {
        match resolve_exclusive_modes(patch, sysproxy_on, tun_on) {
            Some((resolved, forced)) => (resolved.enable_system_proxy, resolved.enable_tun_mode, Some(forced)),
            None => (patch.enable_system_proxy, patch.enable_tun_mode, None),
        }
    }

    #[test]
    fn enabling_tun_forces_system_proxy_off() {
        assert_eq!(
            resolve(&patch(None, Some(true)), true, false),
            (Some(false), Some(true), Some(ForcedOff::SystemProxy)),
        );
    }

    #[test]
    fn enabling_system_proxy_forces_tun_off() {
        assert_eq!(
            resolve(&patch(Some(true), None), false, true),
            (Some(true), Some(false), Some(ForcedOff::TunMode)),
        );
    }

    #[test]
    fn enabling_one_while_other_is_off_is_untouched() {
        assert_eq!(
            resolve(&patch(None, Some(true)), false, false),
            (None, Some(true), None),
        );
        assert_eq!(
            resolve(&patch(Some(true), None), false, false),
            (Some(true), None, None),
        );
    }

    #[test]
    fn both_enabled_in_same_patch_keeps_tun() {
        assert_eq!(
            resolve(&patch(Some(true), Some(true)), false, false),
            (Some(false), Some(true), Some(ForcedOff::SystemProxy)),
        );
    }

    #[test]
    fn disabling_never_touches_the_other_switch() {
        assert_eq!(
            resolve(&patch(None, Some(false)), true, true),
            (None, Some(false), None),
        );
        assert_eq!(
            resolve(&patch(Some(false), None), true, true),
            (Some(false), None, None),
        );
        assert_eq!(resolve(&patch(None, None), true, true), (None, None, None));
    }

    #[test]
    fn explicit_false_for_the_other_switch_is_respected() {
        assert_eq!(
            resolve(&patch(Some(false), Some(true)), true, false),
            (Some(false), Some(true), None),
        );
        assert_eq!(
            resolve(&patch(Some(true), Some(false)), false, true),
            (Some(true), Some(false), None),
        );
    }
}
