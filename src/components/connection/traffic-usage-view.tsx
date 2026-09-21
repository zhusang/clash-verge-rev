import { ArrowBackRounded, InfoOutlined } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Chip,
  LinearProgress,
  MenuItem,
  Tooltip,
  Typography,
  alpha,
  useTheme,
} from '@mui/material'
import { useTranslation } from 'react-i18next'
import { useNavigate } from 'react-router'

import { BaseEmpty, BaseStyledSelect, VirtualList } from '@/components/base'
import {
  TRAFFIC_USAGE_GROUPS,
  TRAFFIC_USAGE_PERIODS,
  type TrafficUsagePeriod,
  type TrafficUsageViewState,
  useTrafficUsage,
  useTrafficUsageStatus,
} from '@/hooks/use-traffic-usage'
import type { TranslationKey } from '@/types/generated/i18n-keys'
import parseTraffic from '@/utils/parse-traffic'

/** Backend marker for connections whose process could not be resolved. */
const UNKNOWN_KEY = 'unknown'
const ROW_HEIGHT = 40
const NUMBER_COL_WIDTH = 96
const SHARE_COL_WIDTH = 150

const PERIOD_LABELS: Record<TrafficUsagePeriod, TranslationKey> = {
  today: 'connections.components.usage.periods.today',
  last24h: 'connections.components.usage.periods.last24h',
  last7d: 'connections.components.usage.periods.last7d',
  last30d: 'connections.components.usage.periods.last30d',
}

const GROUP_LABELS: Record<TrafficUsageGroupBy, TranslationKey> = {
  process: 'connections.components.usage.groupBy.process',
  host: 'connections.components.usage.groupBy.host',
  proxy: 'connections.components.usage.groupBy.proxy',
}

interface Props {
  active: boolean
  state: TrafficUsageViewState
  onStateChange: (next: TrafficUsageViewState) => void
}

const formatBytes = (value: number) => parseTraffic(value).join(' ')

const numberCellSx = {
  flex: `0 0 ${NUMBER_COL_WIDTH}px`,
  textAlign: 'right',
  px: 1,
  whiteSpace: 'nowrap',
} as const

export const TrafficUsageView = ({ active, state, onStateChange }: Props) => {
  const { t } = useTranslation()
  const theme = useTheme()
  const navigate = useNavigate()
  const { period, groupBy, processFilter } = state

  const { data: status } = useTrafficUsageStatus(active)
  const { data: rows = [], isLoading } = useTrafficUsage({
    period,
    groupBy,
    filter: processFilter ? { process: processFilter } : undefined,
    enabled: active,
  })

  const grandTotal = rows.reduce((sum, row) => sum + row.total, 0)
  const canDrillDown = groupBy === 'process' && !processFilter

  const unknownProcessLabel = t('connections.components.usage.unknownProcess')
  const processLabel = (key: string) =>
    key === UNKNOWN_KEY ? unknownProcessLabel : key
  const labelOf = (key: string) =>
    groupBy === 'process' ? processLabel(key) : key

  const drillInto = (row: ITrafficUsageRow) => {
    if (!canDrillDown) return
    onStateChange({ ...state, groupBy: 'host', processFilter: row.key })
  }

  const goBack = () =>
    onStateChange({ ...state, groupBy: 'process', processFilter: null })

  const renderRow = (index: number) => {
    const row = rows[index]
    if (!row) return null
    const share = grandTotal > 0 ? (row.total / grandTotal) * 100 : 0
    const isUnknown = groupBy === 'process' && row.key === UNKNOWN_KEY
    const name = (
      <Typography
        variant="body2"
        noWrap
        sx={{ flex: 1, minWidth: 0, px: 1 }}
        title={row.key}
      >
        {labelOf(row.key)}
      </Typography>
    )

    return (
      <Box
        onClick={() => drillInto(row)}
        sx={{
          display: 'flex',
          alignItems: 'center',
          height: ROW_HEIGHT,
          fontSize: 13,
          borderBottom: `1px solid ${theme.palette.divider}`,
          cursor: canDrillDown ? 'pointer' : 'default',
          '&:hover': canDrillDown
            ? { backgroundColor: theme.palette.action.hover }
            : undefined,
        }}
      >
        {isUnknown ? (
          <Tooltip
            title={t('connections.components.usage.tooltips.unknownProcess')}
            placement="top-start"
          >
            {name}
          </Tooltip>
        ) : (
          name
        )}
        <Box sx={numberCellSx}>{formatBytes(row.upload)}</Box>
        <Box sx={numberCellSx}>{formatBytes(row.download)}</Box>
        <Box sx={{ ...numberCellSx, fontWeight: 600 }}>
          {formatBytes(row.total)}
        </Box>
        <Box
          sx={{
            flex: `0 0 ${SHARE_COL_WIDTH}px`,
            display: 'flex',
            alignItems: 'center',
            gap: 1,
            px: 1,
          }}
        >
          <LinearProgress
            variant="determinate"
            value={Math.min(100, share)}
            sx={{
              flex: 1,
              height: 6,
              borderRadius: 3,
              backgroundColor: alpha(theme.palette.primary.main, 0.12),
            }}
          />
          <Typography
            variant="caption"
            sx={{ width: 44, textAlign: 'right', whiteSpace: 'nowrap' }}
          >
            {share.toFixed(1)}%
          </Typography>
        </Box>
      </Box>
    )
  }

  return (
    <Box
      sx={{
        display: 'flex',
        flexDirection: 'column',
        flex: 1,
        minHeight: 0,
        mx: '10px',
      }}
    >
      <Box sx={{ display: 'flex', alignItems: 'center', gap: 1, mb: 1 }}>
        <BaseStyledSelect
          value={period}
          onChange={(e) =>
            onStateChange({
              ...state,
              period: e.target.value as TrafficUsagePeriod,
            })
          }
        >
          {TRAFFIC_USAGE_PERIODS.map((option) => (
            <MenuItem key={option} value={option}>
              <span style={{ fontSize: 14 }}>{t(PERIOD_LABELS[option])}</span>
            </MenuItem>
          ))}
        </BaseStyledSelect>

        <BaseStyledSelect
          value={groupBy}
          onChange={(e) =>
            onStateChange({
              ...state,
              groupBy: e.target.value as TrafficUsageGroupBy,
              processFilter: null,
            })
          }
        >
          {TRAFFIC_USAGE_GROUPS.map((option) => (
            <MenuItem key={option} value={option}>
              <span style={{ fontSize: 14 }}>{t(GROUP_LABELS[option])}</span>
            </MenuItem>
          ))}
        </BaseStyledSelect>

        {processFilter && (
          <>
            <Button
              size="small"
              startIcon={<ArrowBackRounded />}
              onClick={goBack}
            >
              {t('connections.components.usage.actions.back')}
            </Button>
            <Chip
              size="small"
              label={processLabel(processFilter)}
              title={processFilter}
            />
          </>
        )}

        <Box sx={{ flex: 1 }} />

        <Tooltip title={t('connections.components.usage.tooltips.approximate')}>
          <InfoOutlined fontSize="small" sx={{ opacity: 0.6 }} />
        </Tooltip>
      </Box>

      {status && !status.enabled && (
        <Alert
          severity="info"
          sx={{ mb: 1 }}
          action={
            <Button
              color="inherit"
              size="small"
              onClick={() => navigate('/settings')}
            >
              {t('connections.components.usage.statuses.openSettings')}
            </Button>
          }
        >
          {t('connections.components.usage.statuses.disabled')}
        </Alert>
      )}

      {status?.enabled && !status.collecting && (
        <Alert severity="warning" sx={{ mb: 1 }}>
          {t('connections.components.usage.statuses.notCollecting')}
        </Alert>
      )}

      <Box
        sx={{
          display: 'flex',
          alignItems: 'center',
          height: 32,
          fontSize: 13,
          fontWeight: 600,
          color: 'text.secondary',
          borderBottom: `1px solid ${theme.palette.divider}`,
          userSelect: 'none',
        }}
      >
        <Tooltip
          title={
            canDrillDown
              ? t('connections.components.usage.tooltips.drillDown')
              : ''
          }
          placement="top-start"
        >
          <Typography
            variant="body2"
            sx={{ flex: 1, minWidth: 0, px: 1, fontWeight: 'inherit' }}
          >
            {t(GROUP_LABELS[groupBy])}
          </Typography>
        </Tooltip>
        <Box sx={numberCellSx}>{t('shared.labels.uploaded')}</Box>
        <Box sx={numberCellSx}>{t('shared.labels.downloaded')}</Box>
        <Box sx={numberCellSx}>
          {t('connections.components.usage.fields.total')}
        </Box>
        <Box sx={{ flex: `0 0 ${SHARE_COL_WIDTH}px`, px: 1 }}>
          {t('connections.components.usage.fields.share')}
        </Box>
      </Box>

      {rows.length === 0 ? (
        !isLoading && <BaseEmpty textKey="connections.components.usage.empty" />
      ) : (
        <VirtualList
          count={rows.length}
          estimateSize={ROW_HEIGHT}
          renderItem={renderRow}
          style={{
            flex: 1,
            WebkitOverflowScrolling: 'touch',
            overscrollBehavior: 'contain',
          }}
        />
      )}
    </Box>
  )
}
