import { Box, Typography, alpha, useTheme } from '@mui/material'
import dayjs from 'dayjs'
import { useTranslation } from 'react-i18next'

import { useProfiles } from '@/hooks/use-profiles'
import parseTraffic from '@/utils/parse-traffic'

/**
 * 简洁风格首页的订阅信息条。
 *
 * 只读展示当前订阅的名称、剩余流量与到期时间，位于顶部标题栏下方。
 * 数据直接取自 `useProfiles()`（与订阅页/传统订阅卡共享同一份 react-query 缓存，
 * 不产生额外请求），因此切换或更新订阅后会自动刷新。
 *
 * 兜底约定（与后端 `PrfExtra` 语义一致）：
 * - 无 `extra` 或 `total <= 0` → 视为不限量，展示「流量不限」
 * - 无 `extra` 或 `expire <= 0` → 视为长期有效，展示「长期有效」
 * - 无当前订阅 → 整条不渲染（避免用「不限量」误导未订阅用户）
 */
export const MinimalSubscriptionBar = () => {
  const { t } = useTranslation()
  const theme = useTheme()
  const { current } = useProfiles()

  // 尚未导入/选择订阅时不做任何展示
  if (!current) return null

  const extra = current.extra
  const remaining =
    extra && extra.total > 0
      ? Math.max(extra.total - (extra.upload + extra.download), 0)
      : null
  const expire = extra && extra.expire > 0 ? extra.expire : null

  // 剩余为 0 是合法值（已用尽），只有 null（不限量）才走占位文案
  const remainingText =
    remaining !== null
      ? `${t('home.minimal.subscription.remaining')} ${parseTraffic(remaining).join(' ')}`
      : t('home.minimal.subscription.unlimited')
  const expireText =
    expire !== null
      ? `${t('shared.labels.expireTime')} ${dayjs(expire * 1000).format('YYYY-MM-DD')}`
      : t('home.minimal.subscription.longTerm')

  return (
    <Box
      aria-label={t('home.minimal.subscription.label')}
      sx={{
        display: 'flex',
        alignItems: 'center',
        gap: 1.5,
        mx: 2,
        mt: 0.5,
        px: 2,
        py: 0.5,
        borderRadius: 2,
        bgcolor: alpha(theme.palette.primary.main, 0.06),
        flexShrink: 0,
      }}
    >
      <Typography
        variant="caption"
        noWrap
        title={current.name}
        sx={{ fontWeight: 600, minWidth: 0, flex: 1 }}
      >
        {current.name}
      </Typography>

      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ whiteSpace: 'nowrap', flexShrink: 0 }}
      >
        {remainingText}
      </Typography>

      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ whiteSpace: 'nowrap', flexShrink: 0 }}
      >
        {expireText}
      </Typography>
    </Box>
  )
}

export default MinimalSubscriptionBar
