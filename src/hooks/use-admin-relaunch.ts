import { confirm } from '@tauri-apps/plugin-dialog'
import { useLockFn } from 'ahooks'
import { useCallback } from 'react'
import { useTranslation } from 'react-i18next'

import { restartAsAdmin } from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import getSystem from '@/utils/get-system'

import { useSystemState } from './use-system-state'

/**
 * 以管理员身份重启（仅 Windows）。
 *
 * 后端在提权实例成功启动后会让当前进程退出，因此调用成功后通常不会返回；
 * 用户取消 UAC 或提权失败时会 reject，这里统一转成前端提示。
 */
export const useAdminRelaunch = () => {
  const { t } = useTranslation()
  const { isAdminMode, mutateSystemState } = useSystemState()
  const isWindows = getSystem() === 'windows'

  const relaunch = useCallback(async () => {
    if (isAdminMode) {
      showNotice.info(
        'settings.sections.system.notifications.adminRelaunch.alreadyAdmin',
      )
      return
    }

    // 二次确认：明确告知会关闭并重新打开应用。
    const confirmed = await confirm(
      t('settings.sections.system.notifications.adminRelaunch.confirmMessage'),
      {
        title: t(
          'settings.sections.system.notifications.adminRelaunch.confirmTitle',
        ),
        kind: 'warning',
      },
    )
    if (!confirmed) return

    try {
      await restartAsAdmin()
    } catch (err) {
      // 后端用固定文案区分"用户取消"，其余按失败处理。
      const message = String((err as Error)?.message ?? err ?? '')
      if (message.includes('取消')) {
        showNotice.info(
          'settings.sections.system.notifications.adminRelaunch.cancelled',
        )
      } else {
        showNotice.error(err)
      }
      // 提权未成功时应用仍在运行，刷新状态兜底。
      await mutateSystemState()
    }
  }, [t, isAdminMode, mutateSystemState])

  // useLockFn 保证流程进行中重复点击被忽略（按钮同时会禁用）。
  const onRelaunch = useLockFn(relaunch)

  return {
    /** 是否在 Windows 上展示提权入口 */
    canRelaunchAsAdmin: isWindows,
    isAdminMode,
    onRelaunch,
  }
}
