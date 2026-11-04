/**
 * EntitlementDialog — reusable dialog shell for adding/editing entitlements.
 * Contains the full form (feature selection, value fields).
 * Mode (Grant/Override) is resolved server-side from the owning entity, so the form no longer
 * exposes it. Callers provide onSubmit; this component does not make API calls.
 */
import {
  Button,
  ComboboxFormField,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  Label,
  RadioGroup,
  RadioGroupItem,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Separator,
  Textarea,
} from '@md/ui'
import { PlusIcon } from 'lucide-react'
import { type FormEvent, useEffect, useMemo, useRef, useState } from 'react'
import { z } from 'zod'

import { EntitlementValueFields } from '@/features/entitlements/creation/EntitlementValueFields'
import {
  type ConfigEntry,
  configMapIssues,
  CONFIG_KINDS,
  type ConfigKind,
  groupBySchema,
  checkMeteredPolicy,
  meteredPolicySchemaShape,
  type OverageType,
  RESET_PERIOD_TYPES,
  type ResetPeriodType,
} from '@/features/entitlements/creation/types'
import {
  FEATURE_CODE_CHARSET_MESSAGE,
  FEATURE_CODE_LENGTH_MESSAGE,
  FEATURE_CODE_MAX_LENGTH,
  FEATURE_CODE_REGEX,
  featureKindFromProto,
  slugifyCode,
} from '@/features/entitlements/utils'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { listFeatures } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { CalendarUnit, Feature, FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'

// ── Shared form value type ─────────────────────────────────────────────────────

export type EntitlementFormValues = {
  featureId?: string
  /** The picked existing feature, set on submit. */
  selectedFeature?: Feature
  featureName?: string
  featureCode?: string
  featureDescription?: string
  featureType: 'boolean' | 'metered' | 'config'
  metricId?: string
  boolEnabled?: boolean
  limit?: string
  resetPeriodType: ResetPeriodType
  resetUnit?: CalendarUnit
  resetInterval?: number
  meteredEnabled?: boolean
  groupBy?: string
  overageType?: OverageType
  gracePct?: string
  thresholds?: string
  configKind?: ConfigKind
  configNumber?: string
  configBoolean?: boolean
  configText?: string
  configJson?: string
  configEntries?: ConfigEntry[]
  configOptions?: string[]
}

function isValidJson(s: string | undefined): boolean {
  if (!s || s.trim() === '') return false
  try {
    JSON.parse(s)
    return true
  } catch {
    return false
  }
}

export interface EntitlementDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Caller handles saving (API call or state update) */
  onSubmit: (data: EntitlementFormValues) => void | Promise<void>
  /** Pre-fill values (for edit mode) */
  initialValues?: Partial<EntitlementFormValues>
  /** Lock the feature — shows name as display, skips selector */
  lockedFeature?: { id: string; name: string; featureType: 'boolean' | 'metered' | 'config' }
  existingFeatureIds?: Set<string>
  title?: string
  submitLabel?: string
  isSubmitting?: boolean
}

// ── Schema ─────────────────────────────────────────────────────────────────────

const schema = z
  .object({
    featureId: z.string().optional(),
    featureName: z.string().optional(),
    featureCode: z.string().optional(),
    featureDescription: z.string().optional(),
    featureType: z.enum(['boolean', 'metered', 'config']),
    metricId: z.string().optional(),
    boolEnabled: z.boolean().optional(),
    limit: z.string().optional(),
    resetPeriodType: z.enum(RESET_PERIOD_TYPES),
    resetUnit: z.nativeEnum(CalendarUnit).optional(),
    resetInterval: z.coerce.number().int().min(1).optional(),
    meteredEnabled: z.boolean().optional(),
    groupBy: groupBySchema,
    ...meteredPolicySchemaShape,
    configKind: z.enum(CONFIG_KINDS).optional(),
    configNumber: z.string().optional(),
    configBoolean: z.boolean().optional(),
    configText: z.string().optional(),
    configJson: z.string().optional(),
    configEntries: z
      .array(
        z.object({
          key: z.string(),
          type: z.enum(['number', 'text', 'boolean']),
          value: z.string(),
        })
      )
      .optional(),
    configOptions: z.array(z.string()).optional(),
  })
  .superRefine((d, ctx) => {
    if (d.featureType === 'metered') checkMeteredPolicy(d, ctx)
    if (d.featureType === 'config' && d.configKind === 'map') {
      const issue = configMapIssues(d.configEntries)
      if (issue) ctx.addIssue({ code: z.ZodIssueCode.custom, message: issue, path: ['configEntries'] })
    }
  })
  .refine(d => !d.featureName || d.featureType !== 'metered' || !!d.metricId, {
    message: 'Metric is required for metered features',
    path: ['metricId'],
  })
  .refine(
    d => d.featureType !== 'config' || d.configKind !== 'number' || (d.configNumber ?? '').trim() !== '',
    { message: 'A number value is required', path: ['configNumber'] }
  )
  .refine(d => d.featureType !== 'config' || d.configKind !== 'json' || isValidJson(d.configJson), {
    message: 'Must be valid JSON',
    path: ['configJson'],
  })
  // A new select feature must declare at least one option.
  .refine(
    d =>
      d.featureType !== 'config' ||
      d.configKind !== 'select' ||
      !d.featureName ||
      (d.configOptions?.length ?? 0) > 0,
    { message: 'Add at least one option', path: ['configOptions'] }
  )
  .refine(
    d => d.featureType !== 'config' || d.configKind !== 'select' || !!d.configText,
    { message: 'Choose a value', path: ['configText'] }
  )
  // Code is only collected when creating a new feature (featureName set). Mirror the
  // feature catalog's charset and length rules so the backend never rejects it.
  .refine(d => !d.featureName || (d.featureCode ?? '').length <= FEATURE_CODE_MAX_LENGTH, {
    message: FEATURE_CODE_LENGTH_MESSAGE,
    path: ['featureCode'],
  })
  .refine(d => !d.featureName || FEATURE_CODE_REGEX.test(d.featureCode ?? ''), {
    message: FEATURE_CODE_CHARSET_MESSAGE,
    path: ['featureCode'],
  })

const MAX_METRIC_SHORTCUTS = 3

const defaultValues: Partial<EntitlementFormValues> = {
  featureType: 'boolean',
  boolEnabled: true,
  resetPeriodType: 'billingCycle',
  resetUnit: CalendarUnit.MONTH,
  resetInterval: 1,
  meteredEnabled: true,
  overageType: 'block',
  gracePct: '',
  thresholds: '',
  configKind: 'number',
  configEntries: [{ key: '', type: 'number', value: '' }],
}

// ── Component ──────────────────────────────────────────────────────────────────

export function EntitlementDialog({
  open,
  onOpenChange,
  onSubmit,
  initialValues,
  lockedFeature,
  existingFeatureIds,
  title,
  submitLabel = 'Add entitlement',
  isSubmitting,
}: EntitlementDialogProps) {
  const [isCreatingNew, setIsCreatingNew] = useState(!lockedFeature && !!initialValues?.featureName)

  // Server-side search: debounce keystrokes and pass to `listFeatures` so tenants with
  // more features than the page size still find what they need.
  const [featureSearch, setFeatureSearch] = useState('')
  const debouncedFeatureSearch = useDebounceValue(featureSearch, 300)
  // Only Active features are pickable for live entitlement attachment.
  const featuresQuery = useQuery(listFeatures, {
    pagination: { page: 0, perPage: 100 },
    statuses: [FeatureStatus.ACTIVE, FeatureStatus.DISABLED],
    search: debouncedFeatureSearch || undefined,
  })
  const features = useMemo(
    () => (featuresQuery.data?.features ?? []).slice().sort((a, b) => a.name.localeCompare(b.name)),
    [featuresQuery.data]
  )
  const moreFeatures = (featuresQuery.data?.paginationMeta?.totalItems ?? 0) > features.length
  // Every feature the picker has shown, so a pick survives a later search.
  const seenFeatures = useRef(new Map<string, Feature>())
  features.forEach(f => seenFeatures.current.set(f.id, f))

  const metricsQuery = useQuery(listBillableMetrics, { pagination: { page: 0, perPage: 500 } })
  const metrics = (metricsQuery.data?.billableMetrics ?? []).slice().sort((a, b) => a.name.localeCompare(b.name))

  // Metrics no feature measures yet, whatever its status: offered as a new metered feature.
  const allFeaturesQuery = useQuery(
    listFeatures,
    {
      pagination: { page: 0, perPage: 500 },
      statuses: [FeatureStatus.ACTIVE, FeatureStatus.DISABLED, FeatureStatus.ARCHIVED],
    },
    { enabled: open && !lockedFeature }
  )
  const uncoveredMetrics = useMemo(() => {
    if (!allFeaturesQuery.data) return []
    const measured = new Set(
      allFeaturesQuery.data.features.flatMap(f =>
        f.featureType?.Inner?.case === 'metered' ? [f.featureType.Inner.value.metricId] : []
      )
    )
    const search = featureSearch.trim().toLowerCase()
    return metrics.filter(
      m => !m.archivedAt && !measured.has(m.id) && (!search || m.name.toLowerCase().includes(search))
    )
  }, [allFeaturesQuery.data, metrics, featureSearch])

  const form = useZodForm({
    schema,
    defaultValues: { ...defaultValues, ...initialValues },
  })

  const selectedFeatureId = form.watch('featureId')
  const featureType = form.watch('featureType')

  // Capture props in a ref so the open-edge effect can read the latest values without
  // re-running every time the parent passes a fresh object identity. Re-running on prop
  // identity would wipe in-flight user edits.
  const propsRef = useRef({ initialValues, lockedFeature })
  propsRef.current = { initialValues, lockedFeature }

  useEffect(() => {
    if (open) {
      const { initialValues: iv, lockedFeature: lf } = propsRef.current
      form.reset({ ...defaultValues, ...iv })
      setIsCreatingNew(!lf && !!iv?.featureName)
    }
    // form excluded: its identity changes with every form state change, so including it
    // would reset user edits as they type. form.reset is stable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  const { setValue } = form
  useEffect(() => {
    if (selectedFeatureId && !lockedFeature) {
      const feature = features.find(f => f.id === selectedFeatureId)
      if (!feature) return
      const kind = featureKindFromProto(feature.featureType)
      setValue('featureType', kind.type)
      // The config value type is fixed by the feature — seed it so the right value editor shows.
      if (kind.type === 'config') {
        setValue('configKind', kind.valueType)
        if (kind.options) {
          setValue('configOptions', kind.options)
        }
      }
    }
  }, [selectedFeatureId, lockedFeature, features, setValue])

  const effectiveFeatureType: 'boolean' | 'metered' | 'config' = lockedFeature
    ? lockedFeature.featureType
    : featureType

  const showValueFields = !!lockedFeature || !!selectedFeatureId || isCreatingNew

  const startFeatureForMetric = (metric: { id: string; name: string }) => {
    form.setValue('featureId', undefined)
    form.setValue('featureName', metric.name)
    form.setValue('featureCode', slugifyCode(metric.name))
    form.setValue('featureType', 'metered')
    form.setValue('metricId', metric.id)
    setIsCreatingNew(true)
  }

  const featureOptions = features
    .filter(f => !existingFeatureIds?.has(f.id))
    .map(f => ({ value: f.id, label: f.name, keywords: [f.name] }))

  const handleSubmit = async (e: FormEvent<HTMLFormElement>) => {
    e.stopPropagation()
    await form.handleSubmit(async data => {
      await onSubmit({
        ...data,
        featureId: lockedFeature?.id ?? data.featureId,
        selectedFeature: data.featureId ? seenFeatures.current.get(data.featureId) : undefined,
        featureType: effectiveFeatureType,
      })
    })(e)
  }

  const dialogTitle = title ?? (initialValues ? 'Edit entitlement' : 'Add entitlement')

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{dialogTitle}</DialogTitle>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={handleSubmit} className="flex flex-col gap-4">

            {/* ── Feature section ── */}
            {lockedFeature ? (
              <div className="text-sm">
                <span className="font-medium">{lockedFeature.name}</span>
                <span className="ml-2 text-xs text-muted-foreground">
                  {lockedFeature.featureType === 'metered'
                    ? 'Metered'
                    : lockedFeature.featureType === 'config'
                      ? 'Config'
                      : 'Boolean'}
                </span>
              </div>
            ) : !isCreatingNew ? (
              <div className="space-y-1">
                <ComboboxFormField
                  control={form.control}
                  name="featureId"
                  label="Feature"
                  hasSearch
                  // Trust the server's search results — disable cmdk's fuzzy filter so
                  // we don't double-filter and hide rows the server already matched.
                  shouldFilter={false}
                  onSearchChange={setFeatureSearch}
                  unit="feature"
                  description={moreFeatures ? `Showing the first ${features.length} features. Type to search the rest.` : undefined}
                  options={featureOptions}
                  action={
                    <button
                      type="button"
                      className="flex w-full items-center gap-2 px-2 py-1.5 text-sm hover:bg-accent"
                      onClick={() => {
                        form.setValue('featureId', undefined);
                        setIsCreatingNew(true)
                      }}
                    >
                      <PlusIcon size={14}/> Create new feature
                    </button>
                  }
                />
                <FormMessage>{form.formState.errors.featureId?.message}</FormMessage>
                {uncoveredMetrics.length > 0 && (
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1 pt-1 text-xs text-muted-foreground">
                    <span>No feature measures:</span>
                    {uncoveredMetrics.slice(0, MAX_METRIC_SHORTCUTS).map(m => (
                      <button
                        key={m.id}
                        type="button"
                        className="text-brand hover:underline"
                        onClick={() => startFeatureForMetric(m)}
                      >
                        Create a feature for {m.name}
                      </button>
                    ))}
                    {uncoveredMetrics.length > MAX_METRIC_SHORTCUTS && (
                      <span>and {uncoveredMetrics.length - MAX_METRIC_SHORTCUTS} more: search above</span>
                    )}
                  </div>
                )}
              </div>
            ) : (
              <>
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium">New feature</span>
                  <button
                    type="button"
                    className="text-xs text-muted-foreground hover:text-foreground"
                    onClick={() => {
                      form.setValue('featureName', undefined);
                      setIsCreatingNew(false)
                    }}
                  >
                    ← Select existing
                  </button>
                </div>
                <FormField
                  control={form.control}
                  name="featureName"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Feature name <span className="text-destructive">*</span></FormLabel>
                      <FormControl>
                        <Input
                          placeholder="e.g. Monthly API Calls"
                          {...field}
                          onChange={e => {
                            field.onChange(e)
                            // Auto-fill the code from the name until the user edits it.
                            if (!form.formState.dirtyFields.featureCode) {
                              form.setValue('featureCode', slugifyCode(e.target.value), {
                                shouldValidate: true,
                              })
                            }
                          }}
                        />
                      </FormControl>
                      <FormMessage/>
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="featureCode"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        Code <span className="text-destructive">*</span>{' '}
                        <span className="text-muted-foreground text-xs">(stable identifier)</span>
                      </FormLabel>
                      <FormControl>
                        <Input placeholder="e.g. monthly_api_calls" {...field} value={field.value ?? ''}/>
                      </FormControl>
                      <FormMessage/>
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="featureDescription"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        Description <span className="text-muted-foreground text-xs">(optional)</span>
                      </FormLabel>
                      <FormControl>
                        <Textarea placeholder="Optional description" rows={2} {...field} value={field.value ?? ''}/>
                      </FormControl>
                      <FormMessage/>
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="featureType"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Feature type <span className="text-destructive">*</span></FormLabel>
                      <FormControl>
                        <RadioGroup value={field.value} onValueChange={field.onChange} className="flex gap-4">
                          <div className="flex items-center gap-1.5">
                            <RadioGroupItem value="boolean" id="dlg-type-boolean"/>
                            <Label htmlFor="dlg-type-boolean" className="font-normal cursor-pointer">Boolean</Label>
                          </div>
                          <div className="flex items-center gap-1.5">
                            <RadioGroupItem value="metered" id="dlg-type-metered"/>
                            <Label htmlFor="dlg-type-metered" className="font-normal cursor-pointer">Metered</Label>
                          </div>
                          <div className="flex items-center gap-1.5">
                            <RadioGroupItem value="config" id="dlg-type-config"/>
                            <Label htmlFor="dlg-type-config" className="font-normal cursor-pointer">Config</Label>
                          </div>
                        </RadioGroup>
                      </FormControl>
                      <FormMessage/>
                    </FormItem>
                  )}
                />
                {featureType === 'metered' && (
                  <FormField
                    control={form.control}
                    name="metricId"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Metric <span className="text-destructive">*</span></FormLabel>
                        <Select value={field.value ?? ''} onValueChange={field.onChange}>
                          <FormControl>
                            <SelectTrigger><SelectValue placeholder="Select a metric"/></SelectTrigger>
                          </FormControl>
                          <SelectContent>
                            {metrics.map(m => (
                              <SelectItem key={m.id} value={m.id}>{m.name}</SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                        <FormMessage/>
                      </FormItem>
                    )}
                  />
                )}
              </>
            )}

            {/* ── Value fields — shown once feature is determined ── */}
            {showValueFields && (
              <>
                <Separator/>
                <EntitlementValueFields
                  featureType={effectiveFeatureType}
                  idPrefix="dlg"
                  canEditConfigOptions={isCreatingNew}
                  canPickConfigType={isCreatingNew}
                />
              </>
            )}

            <div className="flex justify-end gap-2 pt-2">
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={isSubmitting}>
                {submitLabel}
              </Button>
            </div>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
