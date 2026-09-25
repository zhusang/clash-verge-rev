use crate::{APP_HANDLE, singleton};
use smartstring::alias::String;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;
use tauri_plugin_mihomo::{Mihomo, MihomoExt as _};
use tokio::sync::RwLockReadGuard;

use super::notification::{FrontendEvent, NotificationSystem};

#[derive(Debug)]
pub struct Handle {
    is_exiting: AtomicBool,
    exit_ready: AtomicBool,
}

impl Default for Handle {
    fn default() -> Self {
        Self {
            is_exiting: AtomicBool::new(false),
            exit_ready: AtomicBool::new(false),
        }
    }
}

singleton!(Handle, HANDLE);

impl Handle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn app_handle() -> &'static AppHandle {
        #[allow(clippy::expect_used)]
        APP_HANDLE.get().expect("App handle not initialized")
    }

    pub async fn mihomo() -> RwLockReadGuard<'static, Mihomo> {
        Self::app_handle().mihomo().read().await
    }

    pub fn refresh_clash() {
        Self::send_event(FrontendEvent::RefreshClash);
    }

    pub fn refresh_verge() {
        Self::send_event(FrontendEvent::RefreshVerge);
    }

    pub fn notify_profile_changed(profile_id: &String) {
        Self::send_event(FrontendEvent::ProfileChanged {
            current_profile_id: profile_id,
        });
    }

    pub fn notify_timer_updated(profile_index: &String) {
        Self::send_event(FrontendEvent::TimerUpdated { profile_index });
    }

    pub fn notify_profile_update_started(uid: &String) {
        Self::send_event(FrontendEvent::ProfileUpdateStarted { uid });
    }

    pub fn notify_profile_update_completed(uid: &String) {
        Self::send_event(FrontendEvent::ProfileUpdateCompleted { uid });
    }

    pub fn notice_message<S: AsRef<str>, M: Into<String>>(status: S, msg: M) {
        let status_str = status.as_ref();
        let msg_str = msg.into();

        Self::send_event(FrontendEvent::NoticeMessage {
            status: status_str,
            message: msg_str,
        });
    }

    pub fn try_begin_exit(&self) -> bool {
        self.is_exiting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn cancel_exit(&self) {
        self.exit_ready.store(false, Ordering::Release);
        self.is_exiting.store(false, Ordering::Release);
    }

    pub fn complete_exit_cleanup(&self) {
        self.exit_ready.store(true, Ordering::Release);
    }

    pub fn is_exit_ready(&self) -> bool {
        self.exit_ready.load(Ordering::Acquire)
    }

    pub fn is_exiting(&self) -> bool {
        self.is_exiting.load(Ordering::Acquire)
    }

    fn send_event(event: FrontendEvent) {
        let handle = Self::global();
        if handle.is_exiting() {
            return;
        }

        NotificationSystem::send_event(event);
    }
}

#[cfg(target_os = "macos")]
impl Handle {
    pub fn set_activation_policy(&self, policy: tauri::ActivationPolicy) -> Result<(), String> {
        Self::app_handle()
            .set_activation_policy(policy)
            .map_err(|e| e.to_string().into())
    }

    pub fn set_activation_policy_regular(&self) {
        let _ = self.set_activation_policy(tauri::ActivationPolicy::Regular);
    }

    pub fn set_activation_policy_accessory(&self) {
        let _ = self.set_activation_policy(tauri::ActivationPolicy::Accessory);
    }
}

#[cfg(test)]
mod tests {
    use super::Handle;

    #[test]
    fn exit_waits_for_cleanup_and_rejects_duplicate_requests() {
        let handle = Handle::default();
        assert!(!handle.is_exiting());
        assert!(!handle.is_exit_ready());
        assert!(handle.try_begin_exit());
        assert!(handle.is_exiting());
        assert!(!handle.is_exit_ready());
        assert!(!handle.try_begin_exit());
        handle.complete_exit_cleanup();
        assert!(handle.is_exit_ready());
        assert!(!handle.try_begin_exit());
    }

    #[test]
    fn failed_exit_can_be_retried() {
        let handle = Handle::default();
        assert!(handle.try_begin_exit());
        handle.cancel_exit();
        assert!(!handle.is_exiting());
        assert!(!handle.is_exit_ready());
        assert!(handle.try_begin_exit());
    }
}
