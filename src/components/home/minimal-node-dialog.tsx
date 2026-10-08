import {
  AccessTimeRounded,
  CheckCircleRounded,
  NetworkCheckRounded,
} from '@mui/icons-material'
import {
  Box,
  Button,
  Chip,
  CircularProgress,
  Dialog,
  DialogContent,
  DialogTitle,
  MenuItem,
  Select,
  Stack,
  Typography,
  alpha,
  useTheme,
} from '@mui/material'
import { useLockFn } from 'ahooks'
import { useCallback, useEffect, useMemo, useReducer, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { VirtualList } from '@/components/base'
import { useProxySelection } from '@/hooks/use-proxy-selection'
import { useVerge } from '@/hooks/use-verge'
import type { DelayUpdate } from '@/services/delay'
import delayManager from '@/services/delay'
import {
  DEFAULT_LATENCY_TEST_URL,
  testGroupDelay,
} from '@/services/group-delay'

interface Props {
  open: boolean
  /** 可选的代理组（通常为主代理组所在的集合） */
  groups: IProxyGroupItem[]
  /** 当前选中的代理组名 */
  activeGroup: string
  /** 当前节点名，用于高亮 */
  activeProxy: string
  onGroupChange: (group: string) => void
  onSelected: () => void
  onClose: () => void
}

const delayColor = (delay: number) => {
  const color = delayManager.formatDelayColor(delay)
  if (!color) return 'default' as const
  const [main] = color.split('.')
  return main as 'success' | 'warning' | 'error' | 'primary' | 'default'
}

/**
 * 节点行：订阅 delayManager 的延迟更新，保持与代理页一致的延迟展示。
 */
const NodeRow = ({
  proxy,
  group,
  selected,
  onSelect,
}: {
  proxy: IProxyItem
  group: string
  selected: boolean
  onSelect: (name: string) => void
}) => {
  const { t } = useTranslation()
  const theme = useTheme()
  const [delayState, setDelayState] = useReducer(
    (_: DelayUpdate, next: DelayUpdate) => next,
    { delay: -1, updatedAt: 0 },
  )

  useEffect(() => {
    delayManager.setListener(proxy.name, group, setDelayState)
    return () => delayManager.removeListener(proxy.name, group, setDelayState)
  }, [proxy.name, group])

  useEffect(() => {
    setDelayState({
      delay: delayManager.getDelayFix(proxy, group),
      updatedAt: 0,
    })
  }, [proxy, group])

  const delay = delayState.delay
  const testing = delay === -2
  const label = testing
    ? t('home.minimal.node.testing')
    : delay === -1
      ? t('home.minimal.node.untested')
      : delayManager.formatDelay(delay)

  return (
    <Box
      onClick={() => onSelect(proxy.name)}
      sx={{
        display: 'flex',
        alignItems: 'center',
        gap: 1,
        px: 1.5,
        py: 1,
        mb: 0.75,
        borderRadius: 1.5,
        cursor: 'pointer',
        border: `1px solid ${
          selected ? alpha(theme.palette.primary.main, 0.5) : 'transparent'
        }`,
        bgcolor: selected
          ? alpha(theme.palette.primary.main, 0.08)
          : 'action.hover',
        transition: 'background-color 0.2s, border-color 0.2s',
        '&:hover': {
          bgcolor: alpha(theme.palette.primary.main, 0.12),
        },
      }}
    >
      <Box sx={{ minWidth: 0, flex: 1 }}>
        <Typography
          variant="body2"
          noWrap
          title={proxy.name}
          sx={{ fontWeight: selected ? 600 : 400 }}
        >
          {proxy.name}
        </Typography>
        {proxy.provider && (
          <Typography variant="caption" color="text.secondary" noWrap>
            {proxy.provider}
          </Typography>
        )}
      </Box>

      {testing ? (
        <CircularProgress size={16} />
      ) : (
        <Chip
          size="small"
          label={label}
          color={delay > 0 ? delayColor(delay) : 'default'}
          variant={delay > 0 ? 'filled' : 'outlined'}
          sx={{ minWidth: 52, height: 22 }}
        />
      )}

      {selected && (
        <CheckCircleRounded
          fontSize="small"
          sx={{ color: 'primary.main', flexShrink: 0 }}
        />
      )}
    </Box>
  )
}

/**
 * 简洁首页的节点选择对话框。
 * 复用 `useProxySelection`，因此选择节点后会同步托盘并持久化到当前订阅。
 */
export const MinimalNodeDialog = ({
  open,
  groups,
  activeGroup,
  activeProxy,
  onGroupChange,
  onSelected,
  onClose,
}: Props) => {
  const { t } = useTranslation()
  const { verge } = useVerge()
  const timeout = verge?.default_latency_timeout || 10000

  const { changeProxy } = useProxySelection({
    onSuccess: () => onSelected(),
    onError: () => onSelected(),
  })

  // 只展示可手动选择节点的代理组；若一个都没有则退回全部组
  const selectableGroups = useMemo(() => {
    const selectableTypes = new Set([
      'Selector',
      'URLTest',
      'Fallback',
      'LoadBalance',
    ])
    const filtered = groups.filter((group) => selectableTypes.has(group.type))
    return filtered.length > 0 ? filtered : groups
  }, [groups])

  const currentGroup = useMemo(
    () =>
      selectableGroups.find((group) => group.name === activeGroup) ??
      selectableGroups[0],
    [selectableGroups, activeGroup],
  )

  const proxyList = useMemo(() => currentGroup?.all ?? [], [currentGroup])

  const [testing, setTesting] = useState(false)

  useEffect(() => {
    if (!currentGroup) return
    const url = verge?.default_latency_test?.trim() || DEFAULT_LATENCY_TEST_URL
    delayManager.setUrl(currentGroup.name, url)
  }, [currentGroup, verge?.default_latency_test])

  const handleSelect = useCallback(
    (name: string) => {
      if (!currentGroup || name === activeProxy) {
        onClose()
        return
      }
      changeProxy(currentGroup.name, name, currentGroup.now)
      onClose()
    },
    [activeProxy, changeProxy, currentGroup, onClose],
  )

  const handleTestAll = useLockFn(async () => {
    if (!currentGroup) return
    setTesting(true)
    try {
      const url =
        verge?.default_latency_test?.trim() || DEFAULT_LATENCY_TEST_URL
      await testGroupDelay(currentGroup.name, proxyList, timeout, url).race
    } catch (error) {
      console.error('[MinimalNodeDialog] 延迟测试失败:', error)
    } finally {
      setTesting(false)
    }
  })

  return (
    <Dialog open={open} onClose={onClose} maxWidth="xs" fullWidth>
      <DialogTitle
        sx={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 1,
          pb: 1,
        }}
      >
        <span>{t('home.minimal.node.dialogTitle')}</span>
        <Button
          size="small"
          variant="text"
          startIcon={
            testing ? (
              <CircularProgress size={14} color="inherit" />
            ) : (
              <NetworkCheckRounded fontSize="small" />
            )
          }
          onClick={handleTestAll}
          disabled={testing || proxyList.length === 0}
          sx={{ textTransform: 'none', flexShrink: 0 }}
        >
          {t('home.minimal.node.testAll')}
        </Button>
      </DialogTitle>

      <DialogContent sx={{ pt: 0 }}>
        {selectableGroups.length > 1 && (
          <Box sx={{ mb: 1.5 }}>
            <Select
              fullWidth
              size="small"
              value={currentGroup?.name ?? ''}
              onChange={(event) => onGroupChange(event.target.value)}
            >
              {selectableGroups.map((group) => (
                <MenuItem key={group.name} value={group.name}>
                  {group.name}
                </MenuItem>
              ))}
            </Select>
          </Box>
        )}

        {proxyList.length === 0 ? (
          <Stack
            alignItems="center"
            justifyContent="center"
            sx={{ py: 6, color: 'text.secondary' }}
          >
            <AccessTimeRounded sx={{ mb: 1, opacity: 0.5 }} />
            <Typography variant="body2">
              {t('home.minimal.node.empty')}
            </Typography>
          </Stack>
        ) : (
          <VirtualList
            count={proxyList.length}
            estimateSize={64}
            overscan={8}
            style={{ height: 420, paddingRight: 4 }}
            getItemKey={(index) =>
              `${currentGroup?.name ?? ''}-${proxyList[index]?.name ?? index}`
            }
            renderItem={(index) => {
              const proxy = proxyList[index]
              if (!proxy || !currentGroup) return null
              return (
                <NodeRow
                  proxy={proxy}
                  group={currentGroup.name}
                  selected={proxy.name === activeProxy}
                  onSelect={handleSelect}
                />
              )
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

export default MinimalNodeDialog
