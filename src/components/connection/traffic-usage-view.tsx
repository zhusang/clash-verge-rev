import { ArrowBackRounded, InfoOutlined } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Chip,
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
  TRAFFIC_USAGE_ROUTES,
  type TrafficUsagePeriod,
  type TrafficUsageRouteFilter,
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
const ROUTE_COL_WIDTH = 132
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

const ROUTE_LABELS: Record<TrafficUsageRouteFilter, TranslationKey> = {
  all: 'connections.components.usage.routes.all',
  proxy: 'connections.components.usage.routes.proxy',
  direct: 'connections.components.usage.routes.direct',
}

/** Where a click on a row leads; `proxy` rows are the deepest level. */
const DRILL_DOWN_TOOLTIPS: Partial<
  Record<TrafficUsageGroupBy, TranslationKey>
> = {
  process: 'connections.components.usage.tooltips.drillDown',
  host: 'connections.components.usage.tooltips.drillDownHost',
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
  const { period, groupBy, route, processFilter, hostFilter } = state

  const { data: status } = useTrafficUsageStatus(active)
  const { data: rows = [], isLoading } = useTrafficUsage({
    period,
    groupBy,
    filter: {
      process: processFilter ?? undefined,
      host: hostFilter ?? undefined,
      route: route === 'all' ? undefined : route,
    },
    enabled: active,
  })

  const grandTotal = rows.reduce((sum, row) => sum + row.total, 0)
  const drillDownTooltip = DRILL_DOWN_TOOLTIPS[groupBy]
  const canDrillDown = drillDownTooltip !== undefined
  const proxyColor = theme.palette.primary.main
  const directColor = theme.palette.success.main

  const unknownProcessLabel = t('connections.components.usage.unknownProcess')
  const processLabel = (key: string) =>
    key === UNKNOWN_KEY ? unknownProcessLabel : key
  const labelOf = (key: string) =>
    groupBy === 'process' ? processLabel(key) : key

  const drillInto = (row: ITrafficUsageRow) => {
    if (groupBy === 'process') {
      onStateChange({ ...state, groupBy: 'host', processFilter: row.key })
    } else if (groupBy === 'host') {
      onStateChange({ ...state, groupBy: 'proxy', hostFilter: row.key })
    }
  }

  const goBack = () =>
    onStateChange(
      hostFilter
        ? { ...state, groupBy: 'host', hostFilter: null }
        : { ...state, groupBy: 'process', processFilter: null },
    )

  const routeLabelOf = (row: ITrafficUsageRow) => {
    if (row.direct <= 0) {
      return t('connections.components.usage.routes.proxy')
    }
    if (row.direct >= row.total) {
      return t('connections.components.usage.routes.direct')
    }
    return t('connections.components.usage.routes.mixed', {
      percent: Math.round((row.direct / row.total) * 100),
    })
  }

  const renderRow = (index: number) => {
    const row = rows[index]
    if (!row) return null
    const proxied = row.total - row.direct
    const routeLabel = routeLabelOf(row)
    const routeSplit = t('connections.components.usage.tooltips.routeSplit', {
      proxy: formatBytes(proxied),
      direct: formatBytes(row.direct),
    })
    const proxyShare = grandTotal > 0 ? (proxied / grandTotal) * 100 : 0
    const directShare = grandTotal > 0 ? (row.direct / grandTotal) * 100 : 0
    const share = proxyShare + directShare
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
        role={canDrillDown ? 'button' : undefined}
        tabIndex={canDrillDown ? 0 : undefined}
        onClick={() => drillInto(row)}
        onKeyDown={(event) => {
          if (canDrillDown && (event.key === 'Enter' || event.key === ' ')) {
            event.preventDefault()
            drillInto(row)
          }
        }}
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
        <Tooltip title={`${routeLabel} · ${routeSplit}`} placement="top">
          <Box
            sx={{
              flex: `0 0 ${ROUTE_COL_WIDTH}px`,
              px: 1,
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              color:
                row.direct <= 0
                  ? proxyColor
                  : proxied <= 0
                    ? directColor
                    : undefined,
            }}
          >
            {routeLabel}
          </Box>
        </Tooltip>
        <Tooltip title={routeSplit} placement="top">
          <Box
            sx={{
              flex: `0 0 ${SHARE_COL_WIDTH}px`,
              display: 'flex',
              alignItems: 'center',
              gap: 1,
              px: 1,
            }}
          >
            <Box
              sx={{
                flex: 1,
                display: 'flex',
                height: 6,
                borderRadius: 3,
                overflow: 'hidden',
                backgroundColor: alpha(proxyColor, 0.12),
              }}
            >
              <Box
                sx={{
                  width: `${Math.min(100, proxyShare)}%`,
                  backgroundColor: proxyColor,
                }}
              />
              <Box
                sx={{
                  width: `${Math.min(100, directShare)}%`,
                  backgroundColor: directColor,
                }}
              />
            </Box>
            <Typography
              variant="caption"
              sx={{ width: 44, textAlign: 'right', whiteSpace: 'nowrap' }}
            >
              {share.toFixed(1)}%
            </Typography>
          </Box>
        </Tooltip>
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
      <Box
        sx={{
          display: 'flex',
          flexWrap: 'wrap',
          alignItems: 'center',
          gap: 1,
          mb: 1,
        }}
      >
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
              hostFilter: null,
            })
          }
        >
          {TRAFFIC_USAGE_GROUPS.map((option) => (
            <MenuItem key={option} value={option}>
              <span style={{ fontSize: 14 }}>{t(GROUP_LABELS[option])}</span>
            </MenuItem>
          ))}
        </BaseStyledSelect>

        <BaseStyledSelect
          value={route}
          onChange={(e) =>
            onStateChange({
              ...state,
              route: e.target.value as TrafficUsageRouteFilter,
            })
          }
        >
          {TRAFFIC_USAGE_ROUTES.map((option) => (
            <MenuItem key={option} value={option}>
              <span style={{ fontSize: 14 }}>{t(ROUTE_LABELS[option])}</span>
            </MenuItem>
          ))}
        </BaseStyledSelect>

        {(processFilter || hostFilter) && (
          <Button
            size="small"
            startIcon={<ArrowBackRounded />}
            onClick={goBack}
          >
            {t('connections.components.usage.actions.back')}
          </Button>
        )}
        {processFilter && (
          <Chip
            size="small"
            label={processLabel(processFilter)}
            title={processFilter}
            sx={{ maxWidth: 200 }}
          />
        )}
        {hostFilter && (
          <Chip
            size="small"
            label={hostFilter}
            title={hostFilter}
            sx={{ maxWidth: 200 }}
          />
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
          title={drillDownTooltip ? t(drillDownTooltip) : ''}
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
        <Box sx={{ flex: `0 0 ${ROUTE_COL_WIDTH}px`, px: 1 }}>
          {t('connections.components.usage.fields.route')}
        </Box>
        <Box sx={{ flex: `0 0 ${SHARE_COL_WIDTH}px`, px: 1 }}>
          {t('connections.components.usage.fields.share')}
        </Box>
      </Box>

      {rows.length === 0 ? (
        !isLoading && <BaseEmpty textKey="connections.components.usage.empty" />
      ) : (
        <VirtualList
          key={JSON.stringify([
            period,
            groupBy,
            route,
            processFilter,
            hostFilter,
          ])}
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
