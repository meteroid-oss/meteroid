import {
  Badge,
  Button,
  cn,
  GenericFormField,
  Input,
  InputFormField,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  TextareaFormField,
} from '@md/ui'
import { RotateCcw, XIcon } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Control, FieldPath, UseFormSetValue } from 'react-hook-form'

import { localeLabel } from '@/features/settings/utils/localeRecord'

import { VariableChips } from './VariableChips'

import type { SequenceForm } from './SequenceSheet'
import type { StepLocaleTabs } from './stepCopy'

const STEP_HINT = 'The amount, due date, Pay button and PDF are always included below your message.'

export const OFFSET_MIN = -30
export const OFFSET_MAX = 365

type Direction = 'before' | 'on' | 'after'
const directionOf = (offset: number): Direction =>
  offset < 0 ? 'before' : offset > 0 ? 'after' : 'on'

// The direction is kept apart from the number, so clearing the field to retype it neither
// flips the step to "on the due date" nor sends 0; blur restores the last valid offset.
const OffsetField = ({
  value,
  onChange,
}: {
  value: number
  onChange: (offset: number) => void
}) => {
  const [direction, setDirection] = useState<Direction>(directionOf(value))
  const [text, setText] = useState(value === 0 ? '' : String(Math.abs(value)))

  // Resync when the offset changes from outside (another step shown, a form reset).
  useEffect(() => {
    setDirection(directionOf(value))
    setText(value === 0 ? '' : String(Math.abs(value)))
  }, [value])

  const pick = (d: Direction) => {
    setDirection(d)
    if (d === 'on') {
      setText('')
      onChange(0)
      return
    }
    const magnitude = Math.max(1, Math.abs(value))
    setText(String(magnitude))
    onChange(d === 'before' ? -magnitude : magnitude)
  }

  return (
    <div className="flex items-center gap-2">
      <Input
        type="number"
        min={1}
        max={OFFSET_MAX}
        className="w-24"
        disabled={direction === 'on'}
        value={direction === 'on' ? '' : text}
        onChange={e => {
          const raw = e.target.value
          setText(raw)
          const n = Math.abs(Math.trunc(Number(raw)))
          // Empty or 0 is an unfinished edit: keep the last valid offset.
          if (raw.trim() === '' || !Number.isFinite(n) || n === 0) return
          onChange(direction === 'before' ? -n : n)
        }}
        onBlur={() => setText(value === 0 ? '' : String(Math.abs(value)))}
      />
      <span className="text-sm text-muted-foreground">days</span>
      <Select value={direction} onValueChange={v => pick(v as Direction)}>
        <SelectTrigger className="w-[150px]">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="before">before due date</SelectItem>
          <SelectItem value="on">on the due date</SelectItem>
          <SelectItem value="after">after due date</SelectItem>
        </SelectContent>
      </Select>
    </div>
  )
}

interface Props {
  control: Control<SequenceForm>
  setValue: UseFormSetValue<SequenceForm>
  index: number
  /** The entity's default locale: the first tab, and the fallback for other languages. */
  homeLocale: string
  localeTabs: StepLocaleTabs
  /** Enabled locales this step's custom copy still lacks (empty on built-in copy). */
  missingLocales: string[]
  /** No custom copy in any locale: the step sends the built-in reminder. */
  builtInCopy: boolean
  /** Writing copy needs email customization; without it, stored copy can only be kept or cleared. */
  copyEditable: boolean
  onClearLocale: (locale: string) => void
  onUseBuiltInCopy: () => void
}

export const StepEditor = ({
  control,
  setValue,
  index,
  homeLocale,
  localeTabs,
  missingLocales,
  builtInCopy,
  copyEditable,
  onClearLocale,
  onUseBuiltInCopy,
}: Props) => {
  const bodyWrap = useRef<HTMLDivElement>(null)
  const { tabs, shown, setActive, isEnabled } = localeTabs
  // Copy kept for languages the entity doesn't send in stays behind one toggle.
  const [showDormant, setShowDormant] = useState(false)
  const dormantCount = tabs.filter(l => !isEnabled(l)).length
  const visibleTabs = tabs.filter(l => isEnabled(l) || showDormant || l === shown)

  const subjectName = `steps.${index}.content.${shown}.subject` as FieldPath<SequenceForm>
  const bodyName = `steps.${index}.content.${shown}.body` as FieldPath<SequenceForm>

  const insertVariable = (name: string) => {
    const textarea = bodyWrap.current?.querySelector('textarea')
    const token = `{{${name}}}`
    const current = textarea?.value ?? ''
    const start = textarea?.selectionStart ?? current.length
    const end = textarea?.selectionEnd ?? current.length
    const next = `${current.slice(0, start)}${token}${current.slice(end)}`
    setValue(bodyName, next, { shouldDirty: true, shouldValidate: true })
    requestAnimationFrame(() => {
      if (!textarea) return
      textarea.focus()
      textarea.setSelectionRange(start + token.length, start + token.length)
    })
  }

  const offsetField = (
    <GenericFormField
      control={control}
      name={`steps.${index}.offsetDays`}
      label="When"
      render={({ field }) => (
        <OffsetField value={Number(field.value ?? 0)} onChange={field.onChange} />
      )}
    />
  )

  if (!copyEditable && builtInCopy) {
    return (
      <div className="flex flex-col gap-4">
        {offsetField}
        <div className="space-y-1.5">
          <div className="text-sm font-medium">Message</div>
          <p className="text-[13px] text-muted-foreground">
            Sends the built-in reminder, translated into every language. {STEP_HINT}
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-4">
      {offsetField}

      <div className="space-y-1.5">
        <div className="flex flex-wrap items-center gap-1.5">
          <div className="inline-flex flex-wrap items-center gap-1 rounded-lg border border-border bg-muted/40 p-0.5">
            {visibleTabs.map(locale => {
              const active = locale === shown
              const dormant = !isEnabled(locale)
              const missing = missingLocales.includes(locale)
              return (
                <div
                  key={locale}
                  className={cn(
                    'flex items-center rounded-md text-[13px] font-medium transition-colors',
                    active
                      ? 'bg-background text-foreground shadow-sm'
                      : 'text-muted-foreground hover:text-foreground'
                  )}
                >
                  <button
                    type="button"
                    onClick={() => setActive(locale)}
                    className={cn('flex items-center gap-1.5 py-1 pl-3', dormant ? 'pr-1' : 'pr-3')}
                  >
                    {localeLabel(locale)}
                    {locale === homeLocale && (
                      <span className="font-normal text-muted-foreground">default</span>
                    )}
                    {missing && (
                      <Badge variant="warning" size="sm" className="font-normal shadow-none">
                        Missing
                      </Badge>
                    )}
                    {dormant && (
                      <Badge variant="ghost" size="sm" className="font-normal">
                        Not enabled
                      </Badge>
                    )}
                  </button>
                  {dormant && (
                    <button
                      type="button"
                      onClick={() => onClearLocale(locale)}
                      aria-label={`Remove the ${localeLabel(locale)} copy`}
                      title={`Remove the ${localeLabel(locale)} copy`}
                      className="mr-1 rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                    >
                      <XIcon size={12} />
                    </button>
                  )}
                </div>
              )
            })}
          </div>
          {dormantCount > 0 && (
            <button
              type="button"
              onClick={() => setShowDormant(v => !v)}
              className="rounded-md px-2 py-1 text-[13px] text-muted-foreground hover:text-foreground"
            >
              {showDormant ? 'Hide not enabled' : `+${dormantCount} not enabled`}
            </button>
          )}
          {!builtInCopy && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 gap-1 text-[13px]"
              onClick={onUseBuiltInCopy}
              title="Clear the custom copy in every language"
            >
              <RotateCcw size={13} />
              Use built-in copy
            </Button>
          )}
        </div>
        <p className="text-[13px] text-muted-foreground">
          {!copyEditable
            ? 'This step keeps sending the copy stored for it. Use the built-in copy to replace it with the built-in reminder.'
            : builtInCopy
              ? 'Sends the built-in reminder, translated into every language. Write a subject and message to replace it; custom copy is then needed in every enabled language.'
              : !isEnabled(shown)
                ? `${localeLabel(shown)} is not enabled for this entity, so this copy is never sent. Enable it under Branding → Customer-facing language, or remove it.`
                : missingLocales.length > 0
                  ? `Custom copy is needed in every enabled language: ${missingLocales.map(localeLabel).join(', ')} still missing.`
                  : shown === homeLocale
                    ? 'Sent to customers whose preferred language is not enabled.'
                    : `Sent to customers whose preferred language is ${localeLabel(shown)}.`}
        </p>
      </div>

      <InputFormField
        key={`${shown}-subject`}
        name={subjectName}
        control={control}
        label="Subject"
        placeholder="Your invoice {{invoice_number}} is due soon"
        readOnly={!copyEditable}
      />
      <div ref={bodyWrap} className="space-y-2">
        <TextareaFormField
          key={`${shown}-body`}
          name={bodyName}
          control={control}
          label="Message"
          rows={10}
          className="font-mono text-[13px]"
          placeholder="Hi {{customer_name}},"
          readOnly={!copyEditable}
        />
        {copyEditable && <VariableChips onInsert={insertVariable} />}
        <p className="text-[13px] text-muted-foreground">{STEP_HINT}</p>
      </div>
    </div>
  )
}
