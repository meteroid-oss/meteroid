import {
  Badge,
  Button,
  Input,
  Label,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { ChevronDown, Pencil, PlusIcon, Trash2 } from 'lucide-react'
import { useId, useState } from 'react'

import { PlanLockChip } from '@/features/settings/components/PlanLockChip'

import { useGrantNames } from './GrantChips'
import {
  AddOnGrantDialog,
  CreditGrantDialog,
  DiscountGrantDialog,
  FeatureGrantDialog,
} from './GrantDialogs'
import {
  ANCHOR_LABELS,
  DEFAULT_WINDOWS,
  DURATION_LABELS,
  GRANT_KIND_LABELS,
  KIND_TONES,
  WINDOW_OPTIONS,
  grantDefinitionLabel,
  hasCount,
  newGrantKey,
  windowIssue,
  windowLabel,
} from './grants'

import type { GrantDefinition, GrantForm, GrantKind, WindowDuration, WindowForm } from './grants'

const ADD_LABELS: Record<GrantKind, string> = {
  credit: 'Add credit',
  entitlement: 'Add feature',
  discount: 'Add discount',
  addOn: 'Add add-on',
}

type Editing = { kind: GrantKind; key?: string }

interface GrantsFieldProps {
  value: GrantForm[]
  onChange: (grants: GrantForm[]) => void
  kinds: GrantKind[]
  /** Kinds shown with a lock: the organization's plan doesn't include them. */
  lockedKinds?: GrantKind[]
  /** The currency credit and fixed discounts start in. */
  defaultCurrency?: string
  disabled?: boolean
  error?: string
}

/** An offer's grants, each with its kind, definition and window; edited one by one. */
export const GrantsField = ({
  value,
  onChange,
  kinds,
  lockedKinds = [],
  defaultCurrency,
  disabled,
  error,
}: GrantsFieldProps) => {
  const names = useGrantNames(value)
  const [editing, setEditing] = useState<Editing>()
  const current = editing?.key ? value.find(grant => grant.key === editing.key) : undefined

  const save = (definition: GrantDefinition<GrantKind>) => {
    if (current) {
      onChange(
        value.map(grant =>
          grant.key === current.key
            ? ({ ...definition, key: grant.key, window: grant.window } as GrantForm)
            : grant
        )
      )
    } else {
      onChange([
        ...value,
        { ...definition, key: newGrantKey(), window: DEFAULT_WINDOWS[definition.kind] } as GrantForm,
      ])
    }
  }
  const setWindow = (key: string, window: WindowForm) =>
    onChange(value.map(grant => (grant.key === key ? { ...grant, window } : grant)))
  const remove = (key: string) => onChange(value.filter(grant => grant.key !== key))

  const dialogProps = {
    onSave: save,
    onClose: () => setEditing(undefined),
  }
  const initialOf = <K extends GrantKind>(kind: K) =>
    current?.kind === kind ? (current as unknown as GrantDefinition<K>) : undefined

  return (
    <div className="space-y-3">
      {value.length > 0 && (
        <ul className="divide-y divide-border rounded-md border border-border">
          {value.map(grant => (
            <li key={grant.key} className="flex items-center gap-3 px-3 py-2">
              <Badge variant={KIND_TONES[grant.kind]} size="sm" className="w-16 justify-center">
                {GRANT_KIND_LABELS[grant.kind]}
              </Badge>
              <span className="min-w-0 flex-1 truncate text-sm">
                {grantDefinitionLabel(grant, names)}
              </span>
              <WindowPicker
                kind={grant.kind}
                value={grant.window}
                disabled={disabled}
                onChange={window => setWindow(grant.key, window)}
              />
              {!disabled && (
                <div className="flex shrink-0 items-center">
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label="Edit grant"
                    onClick={() => setEditing({ kind: grant.kind, key: grant.key })}
                  >
                    <Pencil size={13} />
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="size-7 text-muted-foreground hover:text-destructive"
                    aria-label="Remove grant"
                    onClick={() => remove(grant.key)}
                  >
                    <Trash2 size={13} />
                  </Button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
      {!disabled && (
        <div className="flex flex-wrap items-center gap-2">
          {kinds.map(kind => {
            const locked = lockedKinds.includes(kind)
            return (
              <Button
                key={kind}
                type="button"
                variant="outline"
                size="sm"
                hasIcon
                disabled={locked}
                onClick={() => setEditing({ kind })}
              >
                <PlusIcon size={12} /> {ADD_LABELS[kind]}
                {locked && <PlanLockChip noLabel className="ml-1" />}
              </Button>
            )
          })}
        </div>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}

      {editing?.kind === 'credit' && (
        <CreditGrantDialog
          {...dialogProps}
          initial={initialOf('credit')}
          defaultCurrency={defaultCurrency}
        />
      )}
      {editing?.kind === 'discount' && (
        <DiscountGrantDialog
          {...dialogProps}
          initial={initialOf('discount')}
          defaultCurrency={defaultCurrency}
        />
      )}
      {editing?.kind === 'entitlement' && (
        <FeatureGrantDialog
          {...dialogProps}
          initial={initialOf('entitlement')}
          excludeFeatureIds={
            new Set(
              value.flatMap(g =>
                g.kind === 'entitlement' && g.key !== current?.key ? [g.featureId] : []
              )
            )
          }
        />
      )}
      {editing?.kind === 'addOn' && (
        <AddOnGrantDialog {...dialogProps} initial={initialOf('addOn')} />
      )}
    </div>
  )
}

/** The window's label, opening its anchor, duration and count. */
const WindowPicker = ({
  kind,
  value,
  onChange,
  disabled,
}: {
  kind: GrantKind
  value: WindowForm
  onChange: (window: WindowForm) => void
  disabled?: boolean
}) => {
  const id = useId()
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState<WindowForm>(value)
  const options = WINDOW_OPTIONS[kind]
  const durations: WindowDuration[] =
    value.duration === 'invoices' ? [...options.durations, 'invoices'] : options.durations
  const issue = windowIssue(kind, draft)

  if (disabled) {
    return <span className="shrink-0 text-xs text-muted-foreground">{windowLabel(value)}</span>
  }

  return (
    <Popover
      open={open}
      onOpenChange={next => {
        setOpen(next)
        if (next) setDraft(value)
      }}
    >
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 shrink-0 gap-1 px-2 text-xs text-muted-foreground"
        >
          {windowLabel(value)}
          <ChevronDown size={12} />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 space-y-3 p-3">
        <div className="space-y-1.5">
          <Label htmlFor={`${id}-duration`} className="text-xs">
            For
          </Label>
          <div className="flex items-center gap-2">
            {hasCount(draft.duration) && (
              <Input
                type="number"
                inputMode="numeric"
                min={1}
                step={1}
                aria-label="Count"
                className="w-20"
                value={draft.count ?? ''}
                onChange={event =>
                  setDraft({
                    ...draft,
                    count: event.target.value === '' ? undefined : Number(event.target.value),
                  })
                }
              />
            )}
            <Select
              value={draft.duration}
              onValueChange={duration =>
                setDraft({
                  ...draft,
                  anchor: duration === 'untilBilling' ? 'start' : draft.anchor,
                  duration: duration as WindowDuration,
                  count: hasCount(duration as WindowDuration)
                    ? (draft.count ?? DEFAULT_WINDOWS[kind].count ?? 1)
                    : undefined,
                })
              }
            >
              <SelectTrigger id={`${id}-duration`} className="flex-1">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {durations.map(duration => (
                  <SelectItem key={duration} value={duration} disabled={duration === 'invoices'}>
                    {DURATION_LABELS[duration]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
        {options.anchors.length > 1 && draft.duration !== 'untilBilling' && (
          <div className="space-y-1.5">
            <Label htmlFor={`${id}-anchor`} className="text-xs">
              From
            </Label>
            <Select
              value={draft.anchor}
              onValueChange={anchor =>
                setDraft({ ...draft, anchor: anchor as WindowForm['anchor'] })
              }
            >
              <SelectTrigger id={`${id}-anchor`}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {options.anchors.map(anchor => (
                  <SelectItem key={anchor} value={anchor}>
                    {ANCHOR_LABELS[anchor]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
        {issue && <p className="text-xs text-destructive">{issue}</p>}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="outline" size="sm" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            disabled={!!issue}
            onClick={() => {
              onChange(draft)
              setOpen(false)
            }}
          >
            Apply
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  )
}
