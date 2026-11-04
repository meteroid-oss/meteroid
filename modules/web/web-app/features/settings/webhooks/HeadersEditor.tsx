import { Button, Checkbox, Input } from '@md/ui'
import { PlusIcon, Trash2Icon } from 'lucide-react'

import { type WebhookHeader } from '@/rpc/api/webhooksout/v1/models_pb'

/** One header row being edited. `set`: a sensitive header that already has a value. */
export interface HeaderDraft {
  name: string
  value: string
  sensitive: boolean
  set: boolean
}

export const draftsFrom = (headers: WebhookHeader[]): HeaderDraft[] =>
  headers.map(h => ({
    name: h.name,
    value: h.sensitive ? '' : (h.value ?? ''),
    sensitive: h.sensitive,
    set: h.sensitive && h.set,
  }))

/** Why the drafts cannot be saved, or null. A sensitive header left blank keeps its value. */
export const headersError = (drafts: HeaderDraft[]): string | null => {
  const names = new Set<string>()
  for (const draft of drafts) {
    const name = draft.name.trim()
    if (name === '') return 'Every header needs a name.'
    if (!/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(name)) return `“${name}” is not a valid header name.`
    if (names.has(name.toLowerCase())) return `“${name}” is listed twice.`
    names.add(name.toLowerCase())
    if (draft.value === '' && !(draft.sensitive && draft.set)) return `“${name}” needs a value.`
  }
  return null
}

export const toHeaderInputs = (drafts: HeaderDraft[]) =>
  drafts.map(d => ({
    name: d.name.trim(),
    value: d.value === '' ? undefined : d.value,
    sensitive: d.sensitive,
  }))

interface Props {
  value: HeaderDraft[]
  onChange: (value: HeaderDraft[]) => void
}

/**
 * Custom headers sent with every delivery. A sensitive value is write-only: it is never
 * shown again, and leaving its field blank keeps the current value.
 */
export const HeadersEditor = ({ value, onChange }: Props) => {
  const patch = (index: number, change: Partial<HeaderDraft>) =>
    onChange(value.map((draft, i) => (i === index ? { ...draft, ...change } : draft)))

  return (
    <div className="space-y-2">
      {value.length > 0 && (
        <div className="space-y-2">
          {value.map((draft, index) => {
            const missing = draft.sensitive && !draft.set && draft.value === ''
            return (
              <div key={index} className="flex items-center gap-2">
                <Input
                  className="w-[38%] font-mono text-xs"
                  value={draft.name}
                  placeholder="Authorization"
                  onChange={e => patch(index, { name: e.target.value })}
                />
                <Input
                  className={`flex-1 font-mono text-xs ${missing ? 'border-warning' : ''}`}
                  type={draft.sensitive ? 'password' : 'text'}
                  autoComplete="off"
                  value={draft.value}
                  placeholder={
                    draft.sensitive
                      ? draft.set
                        ? '•••••••• (unchanged)'
                        : 'Value required'
                      : 'Value'
                  }
                  onChange={e => patch(index, { value: e.target.value })}
                />
                <label
                  className="flex shrink-0 cursor-pointer items-center gap-1.5 text-xs text-muted-foreground"
                  title="Sensitive values are never shown again"
                >
                  <Checkbox
                    checked={draft.sensitive}
                    onCheckedChange={checked =>
                      patch(index, { sensitive: checked === true, set: false, value: '' })
                    }
                  />
                  Secret
                </label>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-8 w-8 shrink-0 p-0"
                  onClick={() => onChange(value.filter((_, i) => i !== index))}
                >
                  <Trash2Icon size={14} />
                </Button>
              </div>
            )
          })}
        </div>
      )}
      <Button
        type="button"
        variant="outline"
        size="sm"
        hasIcon
        onClick={() => onChange([...value, { name: '', value: '', sensitive: false, set: false }])}
      >
        <PlusIcon size={12} /> Add header
      </Button>
    </div>
  )
}
