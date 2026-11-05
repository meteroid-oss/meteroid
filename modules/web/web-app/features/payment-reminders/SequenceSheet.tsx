import { Code } from '@connectrpc/connect'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  cn,
  Form,
  GenericFormField,
  InputFormField,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Switch,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, ArrowDownUp, PlusIcon, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { FieldPath, useFieldArray, useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { EmailPreviewPanel } from '@/features/customer-emails/EmailPreviewPanel'
import { AudienceBuilder } from '@/features/customer-emails/audience/AudienceBuilder'
import {
  groupDepth,
  MAX_GROUP_DEPTH,
  parseAudience,
  sanitizeAudience,
  serializeAudience,
} from '@/features/customer-emails/audience/model'
import { offsetPill } from '@/features/customer-emails/offsetPill'
import {
  isBlank,
  localeLabel,
  pruneLocaleRecord,
  seedLocaleRecord,
} from '@/features/settings/utils/localeRecord'
import { useDebounceValue } from '@/hooks/useDebounce'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { usePermissions } from '@/hooks/usePermissions'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { EmailKind } from '@/rpc/api/customeremails/v1/models_pb'
import { PaymentReminderSequence } from '@/rpc/api/paymentreminders/v1/models_pb'
import {
  createPaymentReminderSequence,
  listPaymentReminderSequences,
  previewPaymentReminderAudience,
  updatePaymentReminderSequence,
} from '@/rpc/api/paymentreminders/v1/paymentreminders-PaymentRemindersService_connectquery'

import { OFFSET_MAX, OFFSET_MIN, StepEditor } from './StepEditor'
import {
  emptyCopy,
  hasCopy,
  localeListLabel,
  missingCopyLocales,
  useStepLocaleTabs,
  usesBuiltInCopy,
} from './stepCopy'

import type { AudiencePredicate } from '@/features/customer-emails/audience/types'

// Server limits (spec §1.4, §1.5, §2.6).
const MAX_STEPS = 20
const MAX_NAME_CHARS = 80
const MAX_SUBJECT_CHARS = 200
const MAX_BODY_CHARS = 5000

export const DISABLED_HELP =
  'Disabled sequences are skipped: their invoices fall through to the next matching sequence. To send nothing to an audience, keep the sequence enabled with no steps.'

const stepSchema = z.object({
  offsetDays: z
    .number({ error: 'Required' })
    .int('Whole days only')
    .min(OFFSET_MIN, `Min ${OFFSET_MIN} days`)
    .max(OFFSET_MAX, `Max ${OFFSET_MAX} days`),
  content: z.record(
    z.string(),
    z.object({
      subject: z
        .string()
        .max(MAX_SUBJECT_CHARS, `Keep the subject under ${MAX_SUBJECT_CHARS} characters`),
      body: z.string().max(MAX_BODY_CHARS, `Keep the body under ${MAX_BODY_CHARS} characters`),
    })
  ),
})

const baseSchema = z.object({
  name: z
    .string()
    .trim()
    .min(1, 'Required')
    .max(MAX_NAME_CHARS, `Keep the name under ${MAX_NAME_CHARS} characters`),
  enabled: z.boolean(),
  audience: z.array(z.custom<AudiencePredicate>()),
  steps: z.array(stepSchema).max(MAX_STEPS, `At most ${MAX_STEPS} steps`),
})

export type SequenceForm = z.infer<typeof baseSchema>
type StepForm = z.infer<typeof stepSchema>

// A step with custom copy needs a subject and body in every enabled locale (the server
// rejects the save otherwise); a step without any uses the built-in copy.
const schemaFor = (locales: string[]) =>
  baseSchema.superRefine((data, ctx) => {
    // Checked on what is saved: single-child groups collapse first, as on the server.
    if (sanitizeAudience(data.audience).some(p => groupDepth(p) > MAX_GROUP_DEPTH)) {
      ctx.addIssue({
        code: 'custom',
        path: ['audience'],
        message: `Groups nest at most ${MAX_GROUP_DEPTH} deep`,
      })
    }
    data.steps.forEach((step, i) => {
      if (i > 0 && step.offsetDays <= data.steps[i - 1].offsetDays) {
        ctx.addIssue({
          code: 'custom',
          path: ['steps', i, 'offsetDays'],
          message: 'Offsets must be strictly increasing',
        })
      }
      for (const locale of missingCopyLocales(step.content, locales)) {
        for (const field of ['subject', 'body'] as const) {
          if (isBlank(step.content[locale]?.[field])) {
            ctx.addIssue({
              code: 'custom',
              path: ['steps', i, 'content', locale, field],
              message: `Required in ${localeLabel(locale)}`,
            })
          }
        }
      }
    })
  })

const toForm = (sequence: PaymentReminderSequence | null): SequenceForm => ({
  name: sequence?.name ?? '',
  enabled: sequence?.enabled ?? true,
  audience: parseAudience(sequence?.audience),
  steps: (sequence?.steps ?? []).map(s => ({
    offsetDays: s.offsetDays,
    content: seedLocaleRecord(
      emptyCopy,
      Object.fromEntries(
        Object.entries(s.content?.byLocale ?? {}).map(([l, c]) => [
          l,
          { subject: c.subject, body: c.body },
        ])
      )
    ),
  })),
})

const toStepInputs = (steps: StepForm[]) =>
  steps.map(s => ({
    offsetDays: s.offsetDays,
    content: { byLocale: pruneLocaleRecord(s.content, hasCopy) },
  }))

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  entityId: string
  /** Enabled locales, the entity's default first; never empty. */
  locales: string[]
  defaultCurrency?: string
  /** `null` creates a new sequence. */
  sequence: PaymentReminderSequence | null
}

export const SequenceSheet = ({
  open,
  onOpenChange,
  entityId,
  locales,
  defaultCurrency,
  sequence,
}: Props) => {
  const queryClient = useQueryClient()
  const canManage = usePermissions().has('settings:manage')
  const copyEditable = useEntitlement(entitlementCodes.EMAIL_CUSTOMIZATION)
  const isDefault = sequence?.isDefault ?? false
  const homeLocale = locales[0]
  const schema = useMemo(() => schemaFor(locales), [locales])
  const methods = useZodForm({ schema, defaultValues: toForm(null), mode: 'onChange' })
  const { reset, control, setValue, getValues } = methods
  const [activeStep, setActiveStep] = useState(0)

  // Reset on open or when another sequence is shown, not on every refetch: a background refetch
  // hands a new object for the same sequence and must not wipe the edits in progress.
  const sequenceRef = useRef(sequence)
  sequenceRef.current = sequence
  useEffect(() => {
    if (!open) return
    reset(toForm(sequenceRef.current))
    setActiveStep(0)
  }, [open, sequence?.id, reset])

  const { fields, append, remove, replace } = useFieldArray({ control, name: 'steps' })
  const steps = useWatch({ control, name: 'steps' })
  const audience = useWatch({ control, name: 'audience' })
  const enabled = useWatch({ control, name: 'enabled' })

  const shownStep = Math.min(activeStep, Math.max(fields.length - 1, 0))
  const stepContent = steps?.[shownStep]?.content
  const localeTabs = useStepLocaleTabs({ locales, content: stepContent })
  const builtInCopy = usesBuiltInCopy(stepContent)
  const missingByStep = (steps ?? []).map(s => missingCopyLocales(s?.content, locales))
  const missingSummary = missingByStep.flatMap((missing, i) =>
    missing.length > 0
      ? [
          `Step ${offsetPill(steps?.[i]?.offsetDays ?? 0)}: copy missing for ${localeListLabel(missing)}`,
        ]
      : []
  )

  const sortSteps = () =>
    replace([...getValues('steps')].sort((a, b) => a.offsetDays - b.offsetDays))
  const addStep = () => {
    const current = getValues('steps')
    const last = current.length > 0 ? current[current.length - 1].offsetDays : 0
    append({ offsetDays: last + 7, content: seedLocaleRecord(emptyCopy) })
    setActiveStep(current.length)
  }
  const clearLocale = (locale: string) =>
    setValue(`steps.${shownStep}.content.${locale}` as FieldPath<SequenceForm>, emptyCopy(), {
      shouldDirty: true,
      shouldValidate: true,
    })
  const resetToBuiltInCopy = () =>
    setValue(`steps.${shownStep}.content`, seedLocaleRecord(emptyCopy), {
      shouldDirty: true,
      shouldValidate: true,
    })

  // The preview renders what the customer would get: built-in copy when the step has
  // none, else the shown locale (the default while it is empty). Only enabled locales
  // are previewed; a "not enabled" tab previews the default.
  const previewLocale = locales.includes(localeTabs.shown) ? localeTabs.shown : homeLocale
  const shownCopy = stepContent?.[previewLocale]
  const draft = builtInCopy
    ? undefined
    : hasCopy(shownCopy)
      ? shownCopy
      : (stepContent?.[homeLocale] ?? emptyCopy())

  const cleanAudience = useMemo(() => sanitizeAudience(audience ?? []), [audience])
  const audienceJson = useDebounceValue(serializeAudience(cleanAudience), 400)
  const preview = useQuery(
    previewPaymentReminderAudience,
    { invoicingEntityId: entityId, audience: audienceJson, sequenceId: sequence?.id },
    {
      enabled: open && !isDefault && cleanAudience.length > 0,
      retry: (count, err) => err.code !== Code.Unimplemented && count < 2,
    }
  )
  const previewData = preview.data?.preview

  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: listPaymentReminderSequences,
        cardinality: undefined,
      }),
    })
  const onSaved = (message: string) => {
    invalidate()
    onOpenChange(false)
    toast.success(message)
  }
  const onFailed = (err: { code: Code; rawMessage: string }) =>
    toast.error(
      err.code === Code.Unimplemented
        ? 'Payment reminders are not available on this server yet.'
        : err.rawMessage || 'Could not save the sequence'
    )
  const createMut = useMutation(createPaymentReminderSequence, {
    onSuccess: () => onSaved('Sequence created'),
    onError: onFailed,
  })
  const updateMut = useMutation(updatePaymentReminderSequence, {
    onSuccess: () => onSaved('Sequence updated'),
    onError: onFailed,
  })
  const submitting = createMut.isPending || updateMut.isPending

  const onInvalid = () => {
    if (missingSummary.length > 0)
      toast.error(`Add the missing copy before saving. ${missingSummary.join('. ')}.`)
  }

  const onSubmit = (data: SequenceForm) => {
    const audienceJson = isDefault ? undefined : serializeAudience(data.audience)
    if (!isDefault && sanitizeAudience(data.audience).length === 0) {
      methods.setError('audience', {
        message: 'Add at least one condition, or let the default sequence handle it.',
      })
      return
    }
    const steps = toStepInputs(data.steps)
    if (sequence) {
      updateMut.mutate({
        id: sequence.id,
        name: data.name,
        enabled: data.enabled,
        audience: audienceJson,
        steps: { steps },
      })
    } else {
      createMut.mutate({
        invoicingEntityId: entityId,
        name: data.name,
        enabled: data.enabled,
        audience: audienceJson,
        steps,
      })
    }
  }

  const stepsError =
    methods.formState.errors.steps?.root?.message ?? methods.formState.errors.steps?.message
  const stepHasError = (i: number) => !!methods.formState.errors.steps?.[i]

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent size="xlarge" className="sm:max-w-5xl">
        <SheetHeader>
          <SheetTitle>
            {sequence
              ? isDefault
                ? 'Everyone else'
                : `Edit ${sequence.name}`
              : 'New reminder sequence'}
          </SheetTitle>
          <SheetDescription>
            {isDefault
              ? 'Invoices no other sequence claims. Steps are days relative to the due date.'
              : 'Who it applies to, and the emails they get. Steps are days relative to the due date.'}
          </SheetDescription>
        </SheetHeader>
        <Form {...methods}>
          <form
            onSubmit={methods.handleSubmit(onSubmit, onInvalid)}
            className="flex flex-1 flex-col gap-5 overflow-y-auto py-4"
          >
            <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_auto]">
              {!isDefault && (
                <InputFormField
                  name="name"
                  control={control}
                  label="Name"
                  placeholder="Enterprise customers"
                />
              )}
              <GenericFormField
                name="enabled"
                control={control}
                label="Enabled"
                containerClassName={isDefault ? 'sm:col-span-2' : undefined}
                render={({ field }) => (
                  <div className="flex items-center gap-3 pt-1">
                    <Switch checked={field.value} onCheckedChange={field.onChange} />
                    <span className="text-[13px] text-muted-foreground">
                      {field.value ? 'Enrolling matching invoices' : 'Skipped'}
                    </span>
                  </div>
                )}
              />
            </div>
            {!enabled && <p className="-mt-2 text-[13px] text-muted-foreground">{DISABLED_HELP}</p>}

            {!isDefault && (
              <GenericFormField
                name="audience"
                control={control}
                label="Audience"
                render={({ field }) => (
                  <div className="space-y-2 rounded-lg border border-border p-3">
                    <AudienceBuilder
                      predicates={field.value ?? []}
                      onChange={v => {
                        field.onChange(v)
                        methods.clearErrors('audience')
                      }}
                      defaultCurrency={defaultCurrency}
                    />
                    <p className="text-[13px] text-muted-foreground">
                      {cleanAudience.length === 0
                        ? 'Add a condition to see how many open invoices it matches.'
                        : preview.error?.code === Code.Unimplemented
                          ? 'Live counts are not available on this server yet.'
                          : preview.isError
                            ? preview.error.rawMessage || 'Could not count matching invoices.'
                            : previewData
                              ? `Matches ${previewData.invoiceCount} open invoices (${previewData.customerCount} customers) · ${previewData.claimedByEarlier} already claimed by earlier sequences`
                              : 'Counting matching invoices…'}
                    </p>
                  </div>
                )}
              />
            )}

            <div className="grid gap-5 lg:grid-cols-[200px_minmax(0,1fr)_minmax(0,1fr)]">
              <div className="flex flex-col gap-2">
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium">Steps</span>
                  <div className="flex gap-0.5">
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 px-1.5"
                      onClick={sortSteps}
                      title="Sort by offset"
                    >
                      <ArrowDownUp size={14} />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 px-1.5"
                      onClick={addStep}
                      disabled={fields.length >= MAX_STEPS}
                      title="Add step"
                    >
                      <PlusIcon size={14} />
                    </Button>
                  </div>
                </div>
                {stepsError && <p className="text-xs text-destructive">{stepsError}</p>}
                {fields.length === 0 && (
                  <p className="text-[13px] text-muted-foreground">
                    No steps: matching invoices get no reminder.
                  </p>
                )}
                {fields.map((f, i) => (
                  <div
                    key={f.id}
                    role="button"
                    tabIndex={0}
                    onClick={() => setActiveStep(i)}
                    onKeyDown={e => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault()
                        setActiveStep(i)
                      }
                    }}
                    className={cn(
                      'flex items-center gap-2 rounded-md border px-2.5 py-2 text-left text-[13px] transition-colors',
                      i === shownStep
                        ? 'border-primary bg-accent/60'
                        : 'border-border hover:bg-accent/40',
                      stepHasError(i) && 'border-destructive'
                    )}
                  >
                    <Badge variant="outline" size="sm" className="shrink-0 font-normal">
                      {offsetPill(steps?.[i]?.offsetDays ?? 0)}
                    </Badge>
                    <span className="min-w-0 flex-1 truncate text-muted-foreground">
                      {usesBuiltInCopy(steps?.[i]?.content)
                        ? 'Built-in copy'
                        : steps?.[i]?.content?.[homeLocale]?.subject || 'No subject'}
                    </span>
                    {(missingByStep[i]?.length ?? 0) > 0 && (
                      <span title={`Copy missing for ${localeListLabel(missingByStep[i])}`}>
                        <AlertTriangle
                          size={13}
                          className="shrink-0 text-amber-600 dark:text-amber-500"
                        />
                      </span>
                    )}
                    <button
                      type="button"
                      onClick={e => {
                        e.stopPropagation()
                        remove(i)
                        setActiveStep(Math.max(0, Math.min(shownStep, fields.length - 2)))
                      }}
                      aria-label="Remove step"
                      className="text-muted-foreground hover:text-destructive"
                    >
                      <Trash2 size={13} />
                    </button>
                  </div>
                ))}
              </div>

              {fields.length > 0 ? (
                <>
                  <StepEditor
                    key={fields[shownStep]?.id}
                    control={control}
                    setValue={setValue}
                    index={shownStep}
                    homeLocale={homeLocale}
                    localeTabs={localeTabs}
                    missingLocales={missingByStep[shownStep] ?? []}
                    builtInCopy={builtInCopy}
                    copyEditable={copyEditable}
                    onClearLocale={clearLocale}
                    onUseBuiltInCopy={resetToBuiltInCopy}
                  />
                  <EmailPreviewPanel
                    entityId={entityId}
                    kind={EmailKind.PAYMENT_REMINDER}
                    locale={previewLocale}
                    draft={draft}
                    offsetDays={steps?.[shownStep]?.offsetDays ?? 0}
                    onLocaleChange={localeTabs.setActive}
                    className="lg:sticky lg:top-0 lg:self-start"
                  />
                </>
              ) : (
                <div className="lg:col-span-2 rounded-lg border border-dashed border-border p-6 text-center text-[13px] text-muted-foreground">
                  Add a step to write the reminder and preview it.
                </div>
              )}
            </div>

            {missingSummary.length > 0 && (
              <div className="flex items-start gap-1.5 text-[13px] text-amber-700 dark:text-amber-500">
                <AlertTriangle size={14} className="mt-0.5 shrink-0" />
                <div>
                  <p>Steps with custom copy need it in every enabled language:</p>
                  <ul className="list-disc pl-4">
                    {missingSummary.map((line, i) => (
                      <li key={i}>{line}</li>
                    ))}
                  </ul>
                </div>
              </div>
            )}

            <div className="mt-auto flex justify-end gap-2 border-t border-border pt-4">
              <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={submitting || !canManage}>
                {sequence ? 'Save' : 'Create'}
              </Button>
            </div>
          </form>
        </Form>
      </SheetContent>
    </Sheet>
  )
}
