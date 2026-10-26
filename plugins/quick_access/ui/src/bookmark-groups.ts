/** Shared group appearance for filters, bookmark rows, and the native preview. */
export const GROUP_COLORS = [
  '#60a5fa', '#34d399', '#fbbf24', '#f472b6',
  '#a78bfa', '#22d3ee', '#fb923c', '#94a3b8'
]

export function groupColor(tag: string, colors: Record<string, string>): string {
  const custom = Object.hasOwn(colors, tag) ? colors[tag] : ''
  if (/^#[0-9a-f]{6}$/i.test(custom)) return custom
  let hash = 0
  for (const char of tag) hash = (Math.imul(hash, 31) + char.codePointAt(0)!) >>> 0
  return GROUP_COLORS[hash % GROUP_COLORS.length]
}
