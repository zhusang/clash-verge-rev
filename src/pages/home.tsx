import {
  BuildOutlined,
  DnsOutlined,
  HistoryEduOutlined,
  RouterOutlined,
  SettingsOutlined,
  SpeedOutlined,
} from '@mui/icons-material'
import {
  Box,
  Button,
  Checkbox,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  FormGroup,
  Grid,
  Skeleton,
  TextField,
  Tooltip,
  Typography,
} from '@mui/material'
import { Suspense, lazy, useCallback, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useNavigate } from 'react-router'

import { BasePage } from '@/components/base'
import { ClashModeCard } from '@/components/home/clash-mode-card'
import { CurrentProxyCard } from '@/components/home/current-proxy-card'
import { EnhancedCard } from '@/components/home/enhanced-card'
import { EnhancedTrafficStats } from '@/components/home/enhanced-traffic-stats'
import { HomeProfileCard } from '@/components/home/home-profile-card'
import { ProxyTunCard } from '@/components/home/proxy-tun-card'
import { useProfiles } from '@/hooks/use-profiles'
import { useVerge } from '@/hooks/use-verge'
import {
  enhanceProfiles,
  importProfile,
  patchProfilesConfig,
  restartCore,
  updateProfile,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'

const LazyTestCard = lazy(() =>
  import('@/components/home/test-card').then((module) => ({
    default: module.TestCard,
  })),
)
const LazyIpInfoCard = lazy(() =>
  import('@/components/home/ip-info-card').then((module) => ({
    default: module.IpInfoCard,
  })),
)
const LazyClashInfoCard = lazy(() =>
  import('@/components/home/clash-info-card').then((module) => ({
    default: module.ClashInfoCard,
  })),
)
const LazySystemInfoCard = lazy(() =>
  import('@/components/home/system-info-card').then((module) => ({
    default: module.SystemInfoCard,
  })),
)

// 定义首页卡片设置接口
interface HomeCardsSettings {
  profile: boolean
  proxy: boolean
  network: boolean
  mode: boolean
  traffic: boolean
  info: boolean
  clashinfo: boolean
  systeminfo: boolean
  test: boolean
  ip: boolean
  [key: string]: boolean
}

// 首页设置对话框组件接口
interface HomeSettingsDialogProps {
  open: boolean
  onClose: () => void
  homeCards: HomeCardsSettings
  onSave: (cards: HomeCardsSettings) => void
}

const serializeCardFlags = (cards: HomeCardsSettings) =>
  Object.keys(cards)
    .sort()
    .map((key) => `${key}:${cards[key] ? 1 : 0}`)
    .join('|')

// 首页设置对话框组件
const HomeSettingsDialog = ({
  open,
  onClose,
  homeCards,
  onSave,
}: HomeSettingsDialogProps) => {
  const { t } = useTranslation()
  const [cards, setCards] = useState<HomeCardsSettings>(homeCards)
  const { patchVerge } = useVerge()

  const handleToggle = (key: string) => {
    setCards((prev: HomeCardsSettings) => ({
      ...prev,
      [key]: !prev[key],
    }))
  }

  const handleSave = async () => {
    await patchVerge({ home_cards: cards })
    onSave(cards)
    onClose()
  }

  return (
    <Dialog open={open} onClose={onClose} maxWidth="xs" fullWidth>
      <DialogTitle>{t('home.page.settings.title')}</DialogTitle>
      <DialogContent>
        <FormGroup>
          <FormControlLabel
            control={
              <Checkbox
                checked={cards.profile || false}
                onChange={() => handleToggle('profile')}
              />
            }
            label={t('home.page.settings.cards.profile')}
          />
          <FormControlLabel
            control={
              <Checkbox
                checked={cards.proxy || false}
                onChange={() => handleToggle('proxy')}
              />
            }
            label={t('home.page.settings.cards.currentProxy')}
          />
          <FormControlLabel
            control={
              <Checkbox
                checked={cards.network || false}
                onChange={() => handleToggle('network')}
              />
            }
            label={t('home.page.settings.cards.network')}
          />
          <FormControlLabel
            control={
              <Checkbox
                checked={cards.mode || false}
                onChange={() => handleToggle('mode')}
              />
            }
            label={t('home.page.settings.cards.proxyMode')}
          />
          <FormControlLabel
            control={
              <Checkbox
                checked={cards.traffic || false}
                onChange={() => handleToggle('traffic')}
              />
            }
            label={t('home.page.settings.cards.traffic')}
          />
          <FormControlLabel
            control={
              <Checkbox
                checked={cards.test || false}
                onChange={() => handleToggle('test')}
              />
            }
            label={t('home.page.settings.cards.tests')}
          />
          <FormControlLabel
            control={
              <Checkbox
                checked={cards.ip || false}
                onChange={() => handleToggle('ip')}
              />
            }
            label={t('home.page.settings.cards.ip')}
          />
          <FormControlLabel
            control={
              <Checkbox
                checked={cards.clashinfo || false}
                onChange={() => handleToggle('clashinfo')}
              />
            }
            label={t('home.page.settings.cards.clashInfo')}
          />
          <FormControlLabel
            control={
              <Checkbox
                checked={cards.systeminfo || false}
                onChange={() => handleToggle('systeminfo')}
              />
            }
            label={t('home.page.settings.cards.systemInfo')}
          />
        </FormGroup>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>{t('shared.actions.cancel')}</Button>
        <Button onClick={handleSave} color="primary">
          {t('shared.actions.save')}
        </Button>
      </DialogActions>
    </Dialog>
  )
}

const HomePage = () => {
  const navigate = useNavigate()
  const { t } = useTranslation()
  const { verge } = useVerge()
  const { profiles, current, mutateProfiles } = useProfiles()

  // Welcome dialog state — derive `welcomeOpen` from profiles + dismissed flag
  // to avoid `setState` calls inside `useEffect` (eslint set-state-in-effect)
  const [welcomeDismissed, setWelcomeDismissed] = useState(false)
  const [subUrl, setSubUrl] = useState('')
  const [importing, setImporting] = useState(false)
  const [importError, setImportError] = useState('')

  const welcomeOpen = useMemo(() => {
    if (welcomeDismissed) return false
    if (!profiles) return false
    const items = profiles.items ?? []
    const realProfiles = items.filter(
      (p) => p.type === 'remote' || p.type === 'local',
    )
    return realProfiles.length === 0
  }, [profiles, welcomeDismissed])

  // Quick-fix button state: prevent re-clicks while update + restart in flight
  const [quickFixLoading, setQuickFixLoading] = useState(false)
  const handleQuickFix = useCallback(async () => {
    const currentUid = profiles?.current
    if (!currentUid) return
    setQuickFixLoading(true)
    try {
      try {
        await updateProfile(currentUid)
      } catch (err) {
        showNotice.error('home.page.quickFix.updateFailed', err)
        return
      }
      try {
        await restartCore()
      } catch (err) {
        showNotice.error('home.page.quickFix.restartFailed', err)
        return
      }
      showNotice.success('home.page.quickFix.success')
    } finally {
      setQuickFixLoading(false)
    }
  }, [profiles])

  const handleImportSub = useCallback(async () => {
    const url = subUrl.trim()
    if (!url) return
    setImporting(true)
    setImportError('')
    try {
      // 1. import the subscription and get the new profile's UID (requires backend update)
      const newUid = await importProfile(url)

      // 2. explicitly activate the new profile
      if (newUid) {
        await patchProfilesConfig({ current: newUid })
      }

      // 3. refresh UI
      await mutateProfiles()

      // 4. reload core engine
      await new Promise((r) => setTimeout(r, 300))
      await enhanceProfiles()

      setWelcomeDismissed(true)
    } catch (e: any) {
      setImportError(String(e?.message || e || '导入失败'))
    } finally {
      setImporting(false)
    }
  }, [subUrl, mutateProfiles])

  // 设置弹窗的状态
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [localHomeCards, setLocalHomeCards] = useState<{
    value: HomeCardsSettings
    baseSignature: string
  } | null>(null)

  // 卡片显示状态
  const defaultCards = useMemo<HomeCardsSettings>(
    () => ({
      info: false,
      profile: true,
      proxy: true,
      network: true,
      mode: true,
      traffic: false,
      clashinfo: true,
      systeminfo: true,
      test: false,
      ip: false,
    }),
    [],
  )

  const vergeHomeCards = useMemo<HomeCardsSettings | null>(
    () => (verge?.home_cards as HomeCardsSettings | undefined) ?? null,
    [verge],
  )

  const remoteHomeCards = useMemo<HomeCardsSettings>(
    () => vergeHomeCards ?? defaultCards,
    [defaultCards, vergeHomeCards],
  )

  const remoteSignature = useMemo(
    () => serializeCardFlags(remoteHomeCards),
    [remoteHomeCards],
  )

  const pendingLocalCards = useMemo<HomeCardsSettings | null>(() => {
    if (!localHomeCards) return null
    return localHomeCards.baseSignature === remoteSignature
      ? localHomeCards.value
      : null
  }, [localHomeCards, remoteSignature])

  const effectiveHomeCards = pendingLocalCards ?? remoteHomeCards

  // 新增：打开设置弹窗
  const openSettings = useCallback(() => {
    setSettingsOpen(true)
  }, [])

  const renderCard = useCallback(
    (cardKey: string, component: React.ReactNode, size: number = 6) => {
      if (!effectiveHomeCards[cardKey]) return null

      return (
        <Grid size={size} key={cardKey}>
          {component}
        </Grid>
      )
    },
    [effectiveHomeCards],
  )

  const criticalCards = useMemo(
    () => [
      renderCard(
        'profile',
        <HomeProfileCard current={current} onProfileUpdated={mutateProfiles} />,
      ),
      renderCard('proxy', <CurrentProxyCard />),
      renderCard('network', <NetworkSettingsCard />),
      renderCard('mode', <ClashModeEnhancedCard />),
    ],
    [current, mutateProfiles, renderCard],
  )

  // 新增：保存设置时用requestIdleCallback/setTimeout
  const handleSaveSettings = (newCards: HomeCardsSettings) => {
    if (window.requestIdleCallback) {
      window.requestIdleCallback(() =>
        setLocalHomeCards({
          value: newCards,
          baseSignature: remoteSignature,
        }),
      )
    } else {
      setTimeout(
        () =>
          setLocalHomeCards({
            value: newCards,
            baseSignature: remoteSignature,
          }),
        0,
      )
    }
  }

  const nonCriticalCards = useMemo(
    () => [
      renderCard(
        'traffic',
        <EnhancedCard
          title={t('home.page.cards.trafficStats')}
          icon={<SpeedOutlined />}
          iconColor="secondary"
        >
          <EnhancedTrafficStats />
        </EnhancedCard>,
        12,
      ),
      renderCard(
        'test',
        <Suspense fallback={<Skeleton variant="rectangular" height={200} />}>
          <LazyTestCard />
        </Suspense>,
      ),
      renderCard(
        'ip',
        <Suspense fallback={<Skeleton variant="rectangular" height={200} />}>
          <LazyIpInfoCard />
        </Suspense>,
      ),
      renderCard(
        'clashinfo',
        <Suspense fallback={<Skeleton variant="rectangular" height={200} />}>
          <LazyClashInfoCard />
        </Suspense>,
      ),
      renderCard(
        'systeminfo',
        <Suspense fallback={<Skeleton variant="rectangular" height={200} />}>
          <LazySystemInfoCard />
        </Suspense>,
      ),
    ],
    [t, renderCard],
  )
  const dialogKey = useMemo(
    () => `${serializeCardFlags(effectiveHomeCards)}:${settingsOpen ? 1 : 0}`,
    [effectiveHomeCards, settingsOpen],
  )
  return (
    <BasePage
      title={t('home.page.title')}
      contentStyle={{ padding: 2 }}
      header={
        <Box sx={{ display: 'flex', alignItems: 'center' }}>
          <Tooltip
            title={
              !profiles?.current ? t('home.page.quickFix.tooltipNoProfile') : ''
            }
            arrow
            disableHoverListener={!!profiles?.current}
            disableFocusListener={!!profiles?.current}
            disableTouchListener={!!profiles?.current}
          >
            <span>
              <Button
                variant="text"
                color="inherit"
                size="small"
                onClick={handleQuickFix}
                disabled={quickFixLoading || !profiles?.current}
                startIcon={
                  quickFixLoading ? (
                    <CircularProgress size={16} color="inherit" />
                  ) : (
                    <BuildOutlined />
                  )
                }
                sx={{ fontWeight: 'bold' }}
              >
                {t('home.page.quickFix.button')}
              </Button>
            </span>
          </Tooltip>
          <Button
            variant="text"
            color="inherit"
            size="small"
            onClick={() => navigate('/connections')}
            startIcon={<DnsOutlined />}
            sx={{ fontWeight: 'bold' }}
          >
            连接
          </Button>
          <Button
            variant="text"
            color="inherit"
            size="small"
            onClick={() => navigate('/logs')}
            startIcon={<HistoryEduOutlined />}
            sx={{ mr: 1, fontWeight: 'bold' }}
          >
            日志
          </Button>
          <Button
            variant="text"
            color="inherit"
            size="small"
            onClick={openSettings}
            startIcon={<SettingsOutlined />}
            sx={{ fontWeight: 'bold' }}
          >
            {t('home.page.tooltips.settings')}
          </Button>
        </Box>
      }
    >
      <Grid container spacing={1.5} columns={{ xs: 6, sm: 6, md: 12 }}>
        {criticalCards}

        {nonCriticalCards}
      </Grid>

      {/* 首页设置弹窗 */}
      <HomeSettingsDialog
        key={dialogKey}
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        homeCards={effectiveHomeCards}
        onSave={handleSaveSettings}
      />

      {/* 首次启动欢迎弹窗 */}
      <Dialog open={welcomeOpen} maxWidth="sm" fullWidth disableEscapeKeyDown>
        <DialogTitle sx={{ fontWeight: 700, fontSize: 22 }}>
          🎉 欢迎使用Dino-VPN
        </DialogTitle>
        <DialogContent>
          <Typography sx={{ mb: 2, color: 'text.secondary' }}>
            检测到您是首次启动，请粘贴您的订阅链接以便快速开始：
          </Typography>
          <TextField
            autoFocus
            fullWidth
            variant="outlined"
            placeholder="https://example.com/subscribe?token=xxx"
            value={subUrl}
            onChange={(e) => setSubUrl(e.target.value)}
            disabled={importing}
            error={!!importError}
            helperText={importError || ''}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && subUrl.trim()) {
                handleImportSub()
              }
            }}
          />
        </DialogContent>
        <DialogActions sx={{ px: 3, pb: 2 }}>
          <Button
            onClick={() => setWelcomeDismissed(true)}
            disabled={importing}
          >
            稍后手动添加
          </Button>
          <Button
            variant="contained"
            onClick={handleImportSub}
            disabled={importing || !subUrl.trim()}
            startIcon={importing ? <CircularProgress size={16} /> : null}
          >
            {importing ? '正在导入...' : '确认导入'}
          </Button>
        </DialogActions>
      </Dialog>
    </BasePage>
  )
}

// 增强版网络设置卡片组件
const NetworkSettingsCard = () => {
  const { t } = useTranslation()
  return (
    <EnhancedCard
      title={t('home.page.cards.networkSettings')}
      icon={<DnsOutlined />}
      iconColor="primary"
      action={null}
    >
      <ProxyTunCard />
    </EnhancedCard>
  )
}

// 增强版 Clash 模式卡片组件
const ClashModeEnhancedCard = () => {
  const { t } = useTranslation()
  return (
    <EnhancedCard
      title={t('home.page.cards.proxyMode')}
      icon={<RouterOutlined />}
      iconColor="info"
      action={null}
    >
      <ClashModeCard />
    </EnhancedCard>
  )
}

export default HomePage
