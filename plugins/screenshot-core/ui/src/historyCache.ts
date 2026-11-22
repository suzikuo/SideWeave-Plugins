import { reactive, ref } from 'vue'
import { getHistory, readImageDataUrl, type HistoryItem } from './api'

/**
 * Shared in-memory cache for History items and thumbnails across tab switches
 * (HistoryPanel <-> ImageEditorPanel). Backed on disk by `%APPDATA%\SideWeave\thumb-cache`.
 */
export const cachedHistoryItems = ref<HistoryItem[]>([])
export const cachedHistoryThumbs = reactive<Record<string, string>>({})
export const historyCacheReady = ref(false)

const inFlightThumbs = new Set<string>()

export function isImagePath(path: string): boolean {
  return /\.(png|jpe?g|webp|bmp)$/i.test(path)
}

export async function loadHistoryCached(force = false): Promise<HistoryItem[]> {
  if (!force && historyCacheReady.value && cachedHistoryItems.value.length > 0) {
    return cachedHistoryItems.value
  }
  const fresh = await getHistory({ includeDirScan: true })
  // Preserve already-discovered dimensions from cached items if fresh lacks them
  const prevDims = new Map<string, { w: number; h: number }>()
  for (const old of cachedHistoryItems.value) {
    if (old.width && old.height) {
      prevDims.set(old.path, { w: old.width, h: old.height })
    }
  }
  for (const it of fresh) {
    if ((!it.width || !it.height) && prevDims.has(it.path)) {
      const d = prevDims.get(it.path)!
      it.width = d.w
      it.height = d.h
    }
  }
  cachedHistoryItems.value = fresh
  historyCacheReady.value = true
  return fresh
}

export async function ensureThumbnails(items: HistoryItem[], limit = 60): Promise<void> {
  const queue = items
    .filter((it) => isImagePath(it.path) && !cachedHistoryThumbs[it.path] && !inFlightThumbs.has(it.path))
    .slice(0, limit)
  // Load in gentle batches of 4 so IPC stays responsive
  const concurrency = 4
  let cursor = 0
  async function worker() {
    while (cursor < queue.length) {
      const item = queue[cursor++]
      if (!item || cachedHistoryThumbs[item.path] || inFlightThumbs.has(item.path)) continue
      inFlightThumbs.add(item.path)
      try {
        const res = await readImageDataUrl(item.path, 340)
        cachedHistoryThumbs[item.path] = res.dataUrl
        if (!item.width && res.width) {
          item.width = res.width
          item.height = res.height
        }
      } catch {
        // ignore broken/deleted files
      } finally {
        inFlightThumbs.delete(item.path)
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, queue.length) }, () => worker()))
}

export function evictHistoryCacheItem(path: string): void {
  cachedHistoryItems.value = cachedHistoryItems.value.filter((x) => x.path !== path)
  delete cachedHistoryThumbs[path]
}

export function clearHistoryMemoryCache(): void {
  for (const key of Object.keys(cachedHistoryThumbs)) {
    delete cachedHistoryThumbs[key]
  }
  historyCacheReady.value = false
}
