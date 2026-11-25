export const MAX_HIGHLIGHTS = 12
export const MAX_HIGHLIGHT_LENGTH = 120

/** A highlight being edited; `key` keeps a row's identity while the list is reordered. */
export type DraftHighlight =
  | { key: string; kind: 'feature'; featureId: string }
  | { key: string; kind: 'text'; text: string }

export type HighlightInit =
  | { item: { case: 'featureId'; value: string } }
  | { item: { case: 'text'; value: string } }

let nextKey = 0
export const draftKey = () => `h${nextKey++}`

export const fromHighlights = (
  highlights: { item: { case?: string; value?: string } }[]
): DraftHighlight[] =>
  highlights.flatMap((highlight): DraftHighlight[] => {
    const { item } = highlight
    if (item.case === 'featureId' && item.value)
      return [{ key: draftKey(), kind: 'feature', featureId: item.value }]
    if (item.case === 'text') return [{ key: draftKey(), kind: 'text', text: item.value ?? '' }]
    return []
  })

/** What is saved: lines trimmed, blank lines dropped. */
export const toHighlights = (drafts: DraftHighlight[]): HighlightInit[] =>
  drafts.flatMap((draft): HighlightInit[] => {
    if (draft.kind === 'feature') return [{ item: { case: 'featureId', value: draft.featureId } }]
    const text = draft.text.trim()
    return text ? [{ item: { case: 'text', value: text } }] : []
  })

/** The list with the row at `index` moved by `delta`; unchanged when it would leave the list. */
export const moveItem = <T>(list: T[], index: number, delta: number): T[] => {
  const target = index + delta
  if (index < 0 || index >= list.length || target < 0 || target >= list.length) return list
  const next = [...list]
  next.splice(target, 0, ...next.splice(index, 1))
  return next
}
