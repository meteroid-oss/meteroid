import { useTypedParams } from '@/lib/utils/params'

/** The device copy of an editor state, written on every autosave whether or not the server took it. */
export interface LocalDraft {
  payload: string
  savedAt: number
  /** The server holds this payload too. */
  synced: boolean
}

export type LocalDraftTarget = { quoteId: string } | { draftId: string } | 'new'

const PREFIX = 'meteroid.quoteDraft.v1'

export const localDraftKey = (scope: string, target: LocalDraftTarget) =>
  `${PREFIX}.${scope}.${
    target === 'new'
      ? 'new'
      : 'quoteId' in target
        ? `quote:${target.quoteId}`
        : `draft:${target.draftId}`
  }`

export function readLocalDraft(key: string): LocalDraft | undefined {
  try {
    const raw = window.localStorage.getItem(key)
    if (!raw) return undefined
    const parsed = JSON.parse(raw) as LocalDraft
    return typeof parsed?.payload === 'string' && typeof parsed.savedAt === 'number'
      ? parsed
      : undefined
  } catch {
    return undefined
  }
}

export function writeLocalDraft(key: string, draft: LocalDraft) {
  try {
    window.localStorage.setItem(key, JSON.stringify(draft))
  } catch {
    // Storage full or blocked: the server copy is all there is.
  }
}

export function removeLocalDraft(key: string) {
  try {
    window.localStorage.removeItem(key)
  } catch {
    // Storage blocked: nothing was stored either.
  }
}

export const useLocalDraftScope = () => {
  const { organizationSlug, tenantSlug } = useTypedParams()
  return `${organizationSlug ?? ''}.${tenantSlug ?? ''}`
}
