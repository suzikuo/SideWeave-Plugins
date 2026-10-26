export function resetTimeText(timestamp: number): string {
  if (!Number.isFinite(timestamp) || timestamp <= 0 || !Number.isFinite(new Date(timestamp).getTime())) return '工具未提供重置时间'
  return `重置于 ${new Intl.DateTimeFormat('zh-CN', {
    year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false,
    timeZoneName: 'shortOffset'
  }).format(timestamp)}`
}

export function resetCountdown(timestamp: number, now = Date.now()): string {
  if (!Number.isFinite(timestamp) || timestamp <= 0) return '重置时间未知'
  if (timestamp <= now) return '已到计划重置时间，等待更新'
  const minutes = Math.ceil((timestamp - now) / 60_000)
  if (minutes < 60) return `${minutes} 分钟后重置`
  if (minutes < 1_440) return `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分后重置`
  return `${Math.floor(minutes / 1_440)} 天 ${Math.floor(minutes % 1_440 / 60)} 小时后重置`
}
