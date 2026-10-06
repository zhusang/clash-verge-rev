import {
  BuildRounded,
  CheckRounded,
  ComputerRounded,
  MenuRounded,
  PowerSettingsNewRounded,
  SwapVertRounded,
  TroubleshootRounded,
} from '@mui/icons-material'
import {
  Box,
  Button,
  Chip,
  CircularProgress,
  IconButton,
  Menu,
  MenuItem,
  Tooltip,
  Typography,
  alpha,
  useTheme,
} from '@mui/material'
import { useLockFn } from 'ahooks'
import { useCallback, useEffect, useMemo, useReducer, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useNavigate } from 'react-router'
import { closeAllConnections } from 'tauri-plugin-mihomo-api'

import { MinimalNodeDialog } from '@/components/home/minimal-node-dialog'
import { useServiceInstaller } from '@/hooks/use-service-installer'
import { useSystemProxyState } from '@/hooks/use-system-proxy-state'
import { useSystemState } from '@/hooks/use-system-state'
import { useTunModeToggle } from '@/hooks/use-tun-mode-toggle'
import { useVerge } from '@/hooks/use-verge'
import { useAppData } from '@/providers/app-data-context'
import { patchClashMode } from '@/services/cmds'
import delayManager from '@/services/delay'
import { showNotice } from '@/services/notice-service'
import type { TranslationKey } from '@/types/generated/i18n-keys'

interface Props {
  onSwitchToTraditional: () => void
  onOpenSettings: () => void
}

const BIG_BUTTON_SIZE = 220

type ClashModeKey = 'rule' | 'global' | 'direct'

/** 简洁首页可选的连接方式 */
type ConnectionMode = 'tun' | 'system_proxy'

const CONNECTION_MODE_OPTIONS: {
  value: ConnectionMode
  labelKey: TranslationKey
  icon: typeof ComputerRounded
}[] = [
  {
    value: 'system_proxy',
    labelKey: 'home.minimal.connection.systemProxy',
    icon: ComputerRounded,
  },
  {
    value: 'tun',
    labelKey: 'home.minimal.connection.virtualNic',
    icon: TroubleshootRounded,
  },
]

/** 复用现有代理模式标签文案，避免重复维护 */
const MODE_LABEL_KEY: Record<ClashModeKey, TranslationKey> = {
  rule: 'home.components.clashMode.labels.rule',
  global: 'home.components.clashMode.labels.global',
  direct: 'home.components.clashMode.labels.direct',
}

/**
 * 简洁风格首页。
 *
 * 与传统首页互相独立的视觉设计，只保留：
 * - 中央大圆钮：开关当前选中的连接方式（系统代理 / 虚拟网卡）
 * - 连接方式选择：大圆钮下方切换系统代理与虚拟网卡，偏好持久化
 * - 切换节点：打开节点选择对话框
 * - 只读代理模式标签
 * - 汉堡菜单：切回传统风格与跳转其它页面
 */
export const MinimalHome = ({
  onSwitchToTraditional,
  onOpenSettings,
}: Props) => {
  const { t } = useTranslation()
  const theme = useTheme()
  const navigate = useNavigate()
  const { verge, patchVerge } = useVerge()

  const {
    proxies,
    clashConfig,
    isCoreDataPending,
    refreshProxy,
    refreshClashConfig,
  } = useAppData()
  const { isTunModeAvailable } = useSystemState()
  const { installServiceAndRestartCore } = useServiceInstaller()
  const { indicator: systemProxyEnabled, toggleSystemProxy } =
    useSystemProxyState()

  const { tunEnabled, tunToggle } = useTunModeToggle()
  const [switching, setSwitching] = useState(false)
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null)
  const [nodeDialogOpen, setNodeDialogOpen] = useState(false)

  // 用户选择的连接方式，持久化到 verge 配置；未知值兜底为 TUN（默认）
  const connectionMode: ConnectionMode =
    verge?.home_connection_mode === 'system_proxy' ? 'system_proxy' : 'tun'
  const isTunMethod = connectionMode === 'tun'

  // ---- 代理模式归一为「规则」 ----------------------------------------------
  const currentMode = clashConfig?.mode?.toLowerCase()
  const currentModeKey: ClashModeKey =
    currentMode === 'global' || currentMode === 'direct' ? currentMode : 'rule'

  useEffect(() => {
    // 内核数据未就绪时不误切；已经是 rule 时不重复请求
    if (!currentMode || currentMode === 'rule') return
    let disposed = false
    // 与传统首页的模式卡片保持一致：用户开启了「切换代理时自动关闭连接」才清理连接
    if (verge?.auto_close_connection) {
      closeAllConnections().catch(() => {})
    }
    void patchClashMode('rule')
      .then(() => {
        if (!disposed) refreshClashConfig()
      })
      .catch((error) => {
        console.error('[MinimalHome] 归一代理模式为规则失败:', error)
      })
    return () => {
      disposed = true
    }
  }, [currentMode, refreshClashConfig, verge?.auto_close_connection])

  // ---- 当前节点 -------------------------------------------------------------
  const groups = useMemo<IProxyGroupItem[]>(
    () => (proxies?.groups as IProxyGroupItem[] | undefined) ?? [],
    [proxies],
  )

  const primaryGroup = useMemo(() => {
    if (groups.length === 0) return undefined
    const keywords = ['auto', 'select', 'proxy', '节点选择', '自动选择']
    return (
      groups.find((group) =>
        keywords.some((keyword) =>
          group.name.toLowerCase().includes(keyword.toLowerCase()),
        ),
      ) ?? groups[0]
    )
  }, [groups])

  const [selectedGroup, setSelectedGroup] = useState<string | undefined>()

  // 选中的组若在数据刷新后消失，则回退到主代理组
  const activeGroup = useMemo(
    () => groups.find((group) => group.name === selectedGroup) ?? primaryGroup,
    [groups, selectedGroup, primaryGroup],
  )

  const activeGroupName = activeGroup?.name

  const activeProxyName = activeGroup?.now || ''

  const activeProxy = useMemo(
    () => (activeProxyName ? proxies?.records?.[activeProxyName] : undefined),
    [proxies, activeProxyName],
  )

  // 用 reducer 承载延迟值：延迟来自 delayManager 的外部订阅，
  // 与 `ProxyItem` 保持同一模式，避免在 effect 中同步 setState。
  const [activeDelay, setActiveDelay] = useReducer(
    (_: number, next: number) => next,
    -1,
  )

  useEffect(() => {
    if (!activeProxyName || !activeGroupName) {
      setActiveDelay(-1)
      return
    }

    setActiveDelay(
      delayManager.getDelayFix(
        activeProxy ?? ({} as IProxyItem),
        activeGroupName,
      ),
    )
    delayManager.setListener(activeProxyName, activeGroupName, (next) =>
      setActiveDelay(next.delay),
    )

    // provider 节点的 getDelayFix 读的是内核 history（上次订阅自动健康检查的陈旧
    // 结果），缓存里又没有实时测试记录时会一直显示陈旧的 Timeout。
    // 此时主动测一次当前节点，结果经上面的 listener 刷新，与节点弹窗/代理页一致。
    if (
      delayManager.getDelayUpdate(activeProxyName, activeGroupName) ===
      undefined
    ) {
      const timeout = verge?.default_latency_timeout || 10000
      delayManager
        .checkDelay(activeProxyName, activeGroupName, timeout)
        .catch(() => {})
    }

    return () => delayManager.removeListener(activeProxyName, activeGroupName)
  }, [
    activeProxy,
    activeGroupName,
    activeProxyName,
    verge?.default_latency_timeout,
  ])

  const delayText =
    activeProxy && activeProxyName
      ? delayManager.formatDelay(activeDelay, verge?.default_latency_timeout)
      : ''

  // ---- 连接开关（系统代理 / 虚拟网卡） ----------------------------------------
  const handleConnectionToggle = useCallback(async () => {
    setSwitching(true)
    try {
      if (isTunMethod) {
        await tunToggle(!tunEnabled)
      } else {
        await toggleSystemProxy(!systemProxyEnabled)
      }
    } catch (error) {
      // TUN 不可用时 hook 内部已提示；系统代理自身吞掉错误并回滚，
      // 其余错误在此兜底
      if (!isTunMethod || isTunModeAvailable) {
        showNotice.error(error)
      }
    } finally {
      setSwitching(false)
    }
  }, [
    isTunMethod,
    isTunModeAvailable,
    systemProxyEnabled,
    toggleSystemProxy,
    tunEnabled,
    tunToggle,
  ])

  // 切换连接方式：仅持久化偏好，不改动开关状态
  const handleConnectionModeChange = useCallback(
    (mode: ConnectionMode) => {
      if (mode === connectionMode) return
      void patchVerge({ home_connection_mode: mode }).catch((error) => {
        showNotice.error(error)
      })
    },
    [connectionMode, patchVerge],
  )

  const onInstallService = useLockFn(async () => {
    try {
      await installServiceAndRestartCore()
    } catch (error) {
      showNotice.error(error)
    }
  })

  const closeMenu = () => setMenuAnchor(null)

  const handleSwitchToTraditional = () => {
    closeMenu()
    onSwitchToTraditional()
  }

  const handleNavigate = (path: string) => {
    closeMenu()
    navigate(path)
  }

  const handleOpenSettings = () => {
    closeMenu()
    onOpenSettings()
  }

  const handleSelected = useCallback(() => {
    refreshProxy()
  }, [refreshProxy])

  // ---- 大圆钮视觉 -----------------------------------------------------------
  // 虚拟网卡方式的可用性由服务/管理员模式决定；系统代理方式始终可用
  const unavailable = isTunMethod && !isTunModeAvailable
  const methodEnabled = isTunMethod ? tunEnabled : systemProxyEnabled
  const active = methodEnabled && !unavailable
  const methodLabel = isTunMethod
    ? t('home.minimal.connection.virtualNic')
    : t('home.minimal.connection.systemProxy')

  const buttonLabel = switching
    ? t('home.minimal.connection.busy')
    : unavailable
      ? t('home.minimal.connection.unavailable')
      : t(
          methodEnabled
            ? 'home.minimal.connection.disable'
            : 'home.minimal.connection.enable',
          { mode: methodLabel },
        )

  const statusText = unavailable
    ? ''
    : methodEnabled
      ? t('home.minimal.connection.enabled')
      : t('home.minimal.connection.disabled')

  return (
    <Box
      sx={{
        position: 'relative',
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
        background: `radial-gradient(120% 80% at 50% 0%, ${alpha(
          theme.palette.primary.main,
          0.12,
        )} 0%, transparent 60%)`,
      }}
    >
      {/* 顶部：汉堡菜单 + 只读模式标签 */}
      <Box
        sx={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          px: 2,
          pt: 1.5,
          flexShrink: 0,
        }}
        data-tauri-drag-region="true"
      >
        <Box
          sx={{
            display: 'flex',
            alignItems: 'center',
            gap: 1,
            pointerEvents: 'auto',
          }}
        >
          <IconButton
            size="large"
            onClick={(event) => setMenuAnchor(event.currentTarget)}
            aria-label={t('home.minimal.menu.openMenu')}
            sx={{ color: 'text.primary' }}
          >
            <MenuRounded />
          </IconButton>
          <Typography variant="h6" sx={{ fontWeight: 700 }}>
            {t('home.minimal.title')}
          </Typography>
        </Box>

        <Tooltip title={t('home.minimal.mode.label')}>
          <Chip
            size="small"
            variant="outlined"
            label={`${t('home.minimal.mode.label')} · ${t(MODE_LABEL_KEY[currentModeKey])}`}
            sx={{ fontWeight: 500, pointerEvents: 'auto' }}
          />
        </Tooltip>
      </Box>

      {/* 中央：大圆钮 */}
      <Box
        sx={{
          flex: 1,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 3,
          px: 3,
          minHeight: 0,
        }}
      >
        <Box
          sx={{
            position: 'relative',
            width: BIG_BUTTON_SIZE,
            height: BIG_BUTTON_SIZE,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          {/* 开启时的外圈光晕 */}
          {active && (
            <Box
              sx={{
                position: 'absolute',
                inset: -12,
                borderRadius: '50%',
                background: alpha(theme.palette.primary.main, 0.18),
                animation: 'minimalPulse 2.6s ease-in-out infinite',
                '@keyframes minimalPulse': {
                  '0%': { transform: 'scale(0.96)', opacity: 0.8 },
                  '50%': { transform: 'scale(1.06)', opacity: 0.4 },
                  '100%': { transform: 'scale(0.96)', opacity: 0.8 },
                },
              }}
            />
          )}

          <Box
            role="button"
            tabIndex={0}
            aria-pressed={methodEnabled}
            aria-disabled={unavailable || switching}
            title={
              unavailable ? t('home.minimal.connection.unavailableHint') : ''
            }
            onClick={handleConnectionToggle}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault()
                void handleConnectionToggle()
              }
            }}
            sx={{
              position: 'relative',
              width: '100%',
              height: '100%',
              borderRadius: '50%',
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              gap: 1,
              px: 2,
              textAlign: 'center',
              cursor: unavailable || switching ? 'not-allowed' : 'pointer',
              userSelect: 'none',
              color: active
                ? theme.palette.primary.contrastText
                : 'text.primary',
              background: active
                ? `linear-gradient(145deg, ${theme.palette.primary.main}, ${theme.palette.primary.dark})`
                : alpha(theme.palette.primary.main, 0.08),
              border: `2px solid ${
                active
                  ? 'transparent'
                  : alpha(theme.palette.primary.main, unavailable ? 0.15 : 0.35)
              }`,
              opacity: unavailable ? 0.65 : 1,
              transition: 'all 0.25s ease-in-out',
              '&:hover': unavailable
                ? {}
                : {
                    transform: 'translateY(-2px)',
                    boxShadow: `0 12px 32px ${alpha(
                      theme.palette.primary.main,
                      active ? 0.45 : 0.2,
                    )}`,
                  },
              '&:active': unavailable
                ? {}
                : { transform: 'translateY(1px) scale(0.985)' },
            }}
          >
            {switching ? (
              <CircularProgress
                size={44}
                color={active ? 'inherit' : 'primary'}
              />
            ) : active ? (
              <CheckRounded sx={{ fontSize: 46 }} />
            ) : (
              <PowerSettingsNewRounded sx={{ fontSize: 46 }} />
            )}

            <Typography
              variant="subtitle1"
              sx={{ fontWeight: 700, px: 1, lineHeight: 1.25 }}
            >
              {buttonLabel}
            </Typography>

            {statusText && (
              <Typography variant="caption" sx={{ opacity: 0.85 }}>
                {statusText}
              </Typography>
            )}
          </Box>
        </Box>

        {/* 虚拟网卡不可用时的服务安装引导 */}
        {unavailable && (
          <Button
            size="small"
            variant="outlined"
            startIcon={<BuildRounded fontSize="small" />}
            onClick={onInstallService}
            sx={{ textTransform: 'none' }}
          >
            {t('home.minimal.connection.installService')}
          </Button>
        )}

        {/* 连接方式：系统代理 / 虚拟网卡 */}
        <Box
          sx={{
            display: 'flex',
            alignItems: 'center',
            gap: 1,
            p: 0.5,
            borderRadius: 99,
            bgcolor: alpha(theme.palette.primary.main, 0.06),
          }}
        >
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ pl: 1.5, pr: 0.5, whiteSpace: 'nowrap' }}
          >
            {t('home.minimal.connection.label')}
          </Typography>

          <Box
            role="radiogroup"
            aria-label={t('home.minimal.connection.label')}
            sx={{ display: 'flex', gap: 0.5 }}
          >
            {CONNECTION_MODE_OPTIONS.map(({ value, labelKey, icon: Icon }) => {
              const selected = connectionMode === value
              return (
                <Box
                  key={value}
                  role="radio"
                  tabIndex={0}
                  aria-checked={selected}
                  onClick={() => handleConnectionModeChange(value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault()
                      handleConnectionModeChange(value)
                    }
                  }}
                  sx={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 0.5,
                    px: 1.5,
                    py: 0.5,
                    borderRadius: 99,
                    cursor: 'pointer',
                    userSelect: 'none',
                    fontSize: 13,
                    fontWeight: 600,
                    color: selected
                      ? theme.palette.primary.contrastText
                      : 'text.secondary',
                    bgcolor: selected
                      ? theme.palette.primary.main
                      : 'transparent',
                    transition: 'all 0.2s ease-in-out',
                    '&:hover': selected
                      ? {}
                      : { bgcolor: alpha(theme.palette.primary.main, 0.1) },
                  }}
                >
                  <Icon sx={{ fontSize: 16 }} />
                  {t(labelKey)}
                </Box>
              )
            })}
          </Box>
        </Box>

        {/* 当前节点 + 切换节点 */}
        <Box
          sx={{
            width: '100%',
            maxWidth: 360,
            display: 'flex',
            alignItems: 'center',
            gap: 1,
            p: 1,
            pl: 2,
            borderRadius: 99,
            bgcolor: alpha(theme.palette.primary.main, 0.06),
          }}
        >
          <Box sx={{ minWidth: 0, flex: 1 }}>
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ display: 'block', lineHeight: 1.2 }}
            >
              {activeProxyName
                ? t('home.minimal.node.label')
                : t('home.minimal.node.none')}
            </Typography>
            <Typography variant="body2" noWrap title={activeProxyName}>
              <Box
                component="span"
                sx={{ fontWeight: 600, color: 'text.primary' }}
              >
                {activeProxyName || '—'}
              </Box>
              {delayText && delayText !== '-' && (
                <Box
                  component="span"
                  sx={{ ml: 1, color: 'text.secondary', fontSize: 12 }}
                >
                  {delayText}
                </Box>
              )}
            </Typography>
          </Box>

          <Button
            size="small"
            variant="contained"
            disableElevation
            startIcon={<SwapVertRounded fontSize="small" />}
            onClick={() => setNodeDialogOpen(true)}
            sx={{ textTransform: 'none', borderRadius: 99, flexShrink: 0 }}
          >
            {t('home.minimal.node.switch')}
          </Button>
        </Box>
      </Box>

      {/* 汉堡菜单 */}
      <Menu
        anchorEl={menuAnchor}
        open={Boolean(menuAnchor)}
        onClose={closeMenu}
        transitionDuration={200}
      >
        <MenuItem onClick={handleSwitchToTraditional}>
          {t('home.minimal.menu.switchToTraditional')}
        </MenuItem>
        <MenuItem onClick={() => handleNavigate('/connections')}>
          {t('layout.components.navigation.tabs.connections')}
        </MenuItem>
        <MenuItem onClick={() => handleNavigate('/profile')}>
          {t('layout.components.navigation.tabs.profiles')}
        </MenuItem>
        <MenuItem onClick={() => handleNavigate('/logs')}>
          {t('layout.components.navigation.tabs.logs')}
        </MenuItem>
        <MenuItem onClick={handleOpenSettings}>
          {t('layout.components.navigation.tabs.settings')}
        </MenuItem>
      </Menu>

      <MinimalNodeDialog
        open={nodeDialogOpen}
        groups={groups}
        activeGroup={activeGroupName ?? ''}
        activeProxy={activeProxyName}
        onGroupChange={(group) => setSelectedGroup(group)}
        onSelected={handleSelected}
        onClose={() => setNodeDialogOpen(false)}
      />

      {/* 内核数据加载中的轻量占位 */}
      {isCoreDataPending && groups.length === 0 && (
        <Box
          sx={{
            position: 'absolute',
            bottom: 12,
            left: 0,
            right: 0,
            display: 'flex',
            justifyContent: 'center',
          }}
        >
          <CircularProgress size={18} />
        </Box>
      )}
    </Box>
  )
}

export default MinimalHome
