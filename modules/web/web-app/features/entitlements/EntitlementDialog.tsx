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
import { type FormEvent, useEffect, useRef, useState } from 'react'
import { z } from 'zod'

import { EntitlementValueFields } from '@/features/entitlements/creation/EntitlementValueFields'
import {
  OVERAGE_BEHAVIOR_TYPES,
  RESET_PERIOD_TYPES,
  type OverageBehaviorType,
  type ResetPeriodType,
} from '@/features/entitlements/creation/types'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { listFeatures } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { CalendarUnit } from '@/rpc/api/entitlements/v1/models_pb'

// ── Shared form value type ─────────────────────────────────────────────────────

export type EntitlementFormValues = {
  featureId?: string
  featureName?: string
  featureDescription?: string
  featureType: 'boolean' | 'metered'
  metricId?: string
  boolEnabled?: boolean
  limit?: string
  resetPeriodType: ResetPeriodType
  resetUnit?: CalendarUnit
  resetInterval?: number
  overageBehaviorType: OverageBehaviorType
  gracePeriodPct?: number
  warningThresholdPct?: number
  meteredEnabled?: boolean
}

export interface EntitlementDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Caller handles saving (API call or state update) */
  onSubmit: (data: EntitlementFormValues) => void | Promise<void>
  /** Pre-fill values (for edit mode) */
  initialValues?: Partial<EntitlementFormValues>
  /** Lock the feature — shows name as display, skips selector */
  lockedFeature?: { id: string; name: string; isMetered: boolean }
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
    featureDescription: z.string().optional(),
    featureType: z.enum(['boolean', 'metered']),
    metricId: z.string().optional(),
    boolEnabled: z.boolean().optional(),
    limit: z.string().optional(),
    resetPeriodType: z.enum(RESET_PERIOD_TYPES),
    resetUnit: z.nativeEnum(CalendarUnit).optional(),
    resetInterval: z.coerce.number().int().min(1).optional(),
    overageBehaviorType: z.enum(OVERAGE_BEHAVIOR_TYPES),
    gracePeriodPct: z.coerce.number().int().min(0).optional(),
    warningThresholdPct: z.coerce.number().int().min(0).max(100).optional(),
    meteredEnabled: z.boolean().optional(),
  })
  .refine(d => !d.featureName || d.featureType !== 'metered' || !!d.metricId, {
    message: 'Metric is required for metered features',
    path: ['metricId'],
  })

const defaultValues: Partial<EntitlementFormValues> = {
  featureType: 'boolean',
  boolEnabled: true,
  resetPeriodType: 'billingCycle',
  resetUnit: CalendarUnit.MONTH,
  resetInterval: 1,
  overageBehaviorType: 'block',
  meteredEnabled: true,
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
    statuses: [],
    search: debouncedFeatureSearch || undefined,
  })
  const features = (featuresQuery.data?.features ?? []).slice().sort((a, b) => a.name.localeCompare(b.name))

  const metricsQuery = useQuery(listBillableMetrics, { pagination: { page: 0, perPage: 500 } })
  const metrics = (metricsQuery.data?.billableMetrics ?? []).slice().sort((a, b) => a.name.localeCompare(b.name))

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
  }, [open, form])

  useEffect(() => {
    if (selectedFeatureId && !lockedFeature) {
      const feature = features.find(f => f.id === selectedFeatureId)
      if (!feature) return
      form.setValue('featureType', feature.featureType?.Inner?.case === 'metered' ? 'metered' : 'boolean')
    }
  }, [selectedFeatureId, lockedFeature, features, form])

  const effectiveFeatureType: 'boolean' | 'metered' = lockedFeature
    ? (lockedFeature.isMetered ? 'metered' : 'boolean')
    : featureType

  const showValueFields = !!lockedFeature || !!selectedFeatureId || isCreatingNew

  const featureOptions = features
    .filter(f => !existingFeatureIds?.has(f.id))
    .map(f => ({ value: f.id, label: f.name, keywords: [f.name] }))

  const handleSubmit = async (e: FormEvent<HTMLFormElement>) => {
    e.stopPropagation()
    await form.handleSubmit(async data => {
      await onSubmit({
        ...data,
        featureId: lockedFeature?.id ?? data.featureId,
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
                  {lockedFeature.isMetered ? 'Metered' : 'Boolean'}
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
                        <Input placeholder="e.g. Monthly API Calls" {...field} />
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
