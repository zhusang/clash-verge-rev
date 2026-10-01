import { useCallback } from 'react'
import { useTranslation } from 'react-i18next'

import { useSystemState } from '@/hooks/use-system-state'
import { useVerge } from '@/hooks/use-verge'
import { showNotice } from '@/services/notice-service'

/**
 * TUN 不可用时统一使用的 i18n key。
 * 抽成常量是为了让设置页、传统首页卡片与简洁首页复用同一份提示文案。
 */
export const TUN_UNAVAILABLE_KEY =
  'settings.sections.proxyControl.tooltips.tunUnavailable'

/**
 * 虚拟网卡（TUN）模式开关的共享逻辑。
 *
 * 统一封装了：
 * - 可用性校验（非管理员且服务不可用时拒绝开启、弹出提示并抛错）
 * - 与系统代理互斥的乐观更新（开启 TUN 时同一次更新里把系统代理置为关闭）
 * - 配置持久化
 *
 * 由设置页 / 传统首页的 `ProxyControlSwitches` 与简洁首页的大圆钮共同消费，
 * 避免两处出现行为分叉。失败时向调用方抛错，由调用方决定回滚与提示。
 */
export const useTunModeToggle = () => {
  const { t } = useTranslation()
  const { verge, mutateVerge, patchVerge } = useVerge()
  const { isTunModeAvailable } = useSystemState()

  const tunEnabled = verge?.enable_tun_mode ?? false

  const tunToggle = useCallback(
    async (value: boolean) => {
      if (!isTunModeAvailable) {
        showNotice.error(TUN_UNAVAILABLE_KEY)
        throw new Error(t(TUN_UNAVAILABLE_KEY))
      }

      mutateVerge(
        {
          ...verge,
          enable_tun_mode: value,
          // 系统代理与 TUN 模式互斥。后端会关闭系统代理，这里同步镜像，
          // 让另一个开关在前端立刻反映最终状态。
          ...(value ? { enable_system_proxy: false } : {}),
        },
        false,
      )

      await patchVerge({ enable_tun_mode: value })
    },
    [isTunModeAvailable, mutateVerge, patchVerge, t, verge],
  )

  return { tunEnabled, tunAvailable: isTunModeAvailable, tunToggle }
}
