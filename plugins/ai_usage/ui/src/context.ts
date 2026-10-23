import type { InjectionKey } from 'vue'
import type { useAiUsage } from './composables/useAiUsage'

export type AiUsageContext = ReturnType<typeof useAiUsage>
export const aiUsageKey: InjectionKey<AiUsageContext> = Symbol('ai-usage')
