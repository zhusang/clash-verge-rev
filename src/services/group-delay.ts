import { delayGroup, healthcheckProxyProvider } from 'tauri-plugin-mihomo-api'

import delayManager from '@/services/delay'

export const DEFAULT_LATENCY_TEST_URL = 'http://cp.cloudflare.com/generate_204'

interface GroupDelayTask {
  /** 任一路径结束即视为本次触发完成，供「测试全部」按钮结束等待 */
  race: Promise<void>
  /** 普通节点的前端缓存全部写完，供首页在决定是否重试前等待 */
  list: Promise<void>
}

const inflight = new Map<string, GroupDelayTask>()

/**
 * 与简洁首页节点弹窗「测试全部」相同的整组延迟测试。
 * 普通节点经 `checkListDelay` 写入 delayManager 缓存（键为「组名::节点名」），
 * 当前节点的订阅会在该节点测完时更新，不必等整组结束。
 * 同一代理组已有进行中的测试时直接复用，避免首页自动测和弹窗手动测叠在一起。
 */
export function testGroupDelay(
  groupName: string,
  proxies: IProxyItem[],
  timeout: number,
  url: string,
): GroupDelayTask {
  const existing = inflight.get(groupName)
  if (existing) return existing

  let resolveRace: () => void = () => {}
  let rejectRace: (error: unknown) => void = () => {}
  let resolveList: () => void = () => {}
  let rejectList: (error: unknown) => void = () => {}

  const race = new Promise<void>((resolve, reject) => {
    resolveRace = resolve
    rejectRace = reject
  })
  const list = new Promise<void>((resolve, reject) => {
    resolveList = resolve
    rejectList = reject
  })

  const task: GroupDelayTask = { race, list }
  inflight.set(groupName, task)

  void (async () => {
    try {
      delayManager.setUrl(groupName, url)

      const providers = [
        ...new Set(
          proxies
            .map((proxy) => proxy.provider)
            .filter((provider): provider is string => Boolean(provider)),
        ),
      ]
      if (providers.length > 0) {
        await Promise.allSettled(
          providers.map((provider) => healthcheckProxyProvider(provider)),
        )
      }

      const names = proxies
        .filter((proxy) => !proxy.provider)
        .map((proxy) => proxy.name)
        .filter(Boolean)

      if (names.length === 0) {
        resolveRace()
        resolveList()
        return
      }

      const listPromise = delayManager.checkListDelay(names, groupName, timeout)
      const groupPromise = delayGroup(
        groupName,
        delayManager.getUrl(groupName),
        timeout,
      ).then(() => undefined)

      void listPromise.then(
        () => resolveList(),
        (error: unknown) => rejectList(error),
      )
      void Promise.race([listPromise, groupPromise]).then(
        () => resolveRace(),
        (error: unknown) => rejectRace(error),
      )
    } catch (error) {
      rejectRace(error)
      rejectList(error)
    }
  })()

  void list.finally(() => {
    if (inflight.get(groupName) === task) inflight.delete(groupName)
  })

  return task
}
