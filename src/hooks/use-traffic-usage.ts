import { useQuery } from '@tanstack/react-query'

import { getTrafficUsage, getTrafficUsageStatus } from '@/services/cmds'

export type TrafficUsagePeriod = 'today' | 'last24h' | 'last7d' | 'last30d'

export const TRAFFIC_USAGE_PERIODS: readonly TrafficUsagePeriod[] = [
  'today',
  'last24h',
  'last7d',
  'last30d',
]

export const TRAFFIC_USAGE_GROUPS: readonly TrafficUsageGroupBy[] = [
  'process',
  'host',
  'proxy',
]

export type TrafficUsageRouteFilter = TrafficUsageRoute | 'all'

export const TRAFFIC_USAGE_ROUTES: readonly TrafficUsageRouteFilter[] = [
  'all',
  'proxy',
  'direct',
]

/** View state of the traffic usage tab, owned by the connections page. */
export interface TrafficUsageViewState {
  period: TrafficUsagePeriod
  groupBy: TrafficUsageGroupBy
  route: TrafficUsageRouteFilter
  /** Set while drilling down from a process into the hosts it accessed. */
  processFilter: string | null
  /** Set while drilling down from a host into the nodes it went through. */
  hostFilter: string | null
}

export const initialTrafficUsageViewState: TrafficUsageViewState = {
  period: 'today',
  groupBy: 'process',
  route: 'all',
  processFilter: null,
  hostFilter: null,
}

/** Matches the backend flush cadence; polling faster shows nothing new. */
const REFRESH_INTERVAL_MS = 30_000
const STALE_TIME_MS = 15_000
const DAY_SECS = 86_400
const PERIOD_DAYS: Record<Exclude<TrafficUsagePeriod, 'today'>, number> = {
  last24h: 1,
  last7d: 7,
  last30d: 30,
}

export const resolveUsageRange = (
  period: TrafficUsagePeriod,
  now: number,
): ITrafficUsageRange => {
  const untilTs = Math.ceil(now / 1000)
  if (period === 'today') {
    const start = new Date(now)
    start.setHours(0, 0, 0, 0)
    return { sinceTs: Math.floor(start.getTime() / 1000), untilTs }
  }
  return { sinceTs: untilTs - PERIOD_DAYS[period] * DAY_SECS, untilTs }
}

interface UseTrafficUsageOptions {
  period: TrafficUsagePeriod
  groupBy: TrafficUsageGroupBy
  filter?: ITrafficUsageFilter
  enabled?: boolean
}

export const useTrafficUsage = ({
  period,
  groupBy,
  filter,
  enabled = true,
}: UseTrafficUsageOptions) => {
  const normalizedFilter: ITrafficUsageFilter = {
    ...(filter?.process ? { process: filter.process } : {}),
    ...(filter?.host ? { host: filter.host } : {}),
    ...(filter?.proxy ? { proxy: filter.proxy } : {}),
    ...(filter?.route ? { route: filter.route } : {}),
  }
  const hasFilter = Object.keys(normalizedFilter).length > 0

  return useQuery({
    queryKey: ['getTrafficUsage', period, groupBy, normalizedFilter],
    // The range is resolved when the query runs so periodic refetches always
    // cover "now" without churning the query key.
    queryFn: () =>
      getTrafficUsage(
        resolveUsageRange(period, Date.now()),
        groupBy,
        hasFilter ? normalizedFilter : undefined,
      ),
    enabled,
    staleTime: STALE_TIME_MS,
    refetchInterval: enabled ? REFRESH_INTERVAL_MS : false,
  })
}

export const useTrafficUsageStatus = (enabled = true) =>
  useQuery({
    queryKey: ['getTrafficUsageStatus'],
    queryFn: getTrafficUsageStatus,
    enabled,
    staleTime: STALE_TIME_MS,
    refetchInterval: enabled ? REFRESH_INTERVAL_MS : false,
  })
