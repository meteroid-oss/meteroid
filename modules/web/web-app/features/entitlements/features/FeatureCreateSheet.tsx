/**
 * FeatureCreateSheet — sheet for creating or editing a feature in the feature catalog.
 * On create, always configures a feature-level entitlement (lowest-priority baseline).
 * Feature-level entitlements apply when no higher-priority entitlement exists for an entity.
 */
import { create, type MessageInitShape } from '@bufbuild/protobuf';
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query';
import {
  Button,
  cn,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Separator,
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  Textarea,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { ValuesTagInput } from '@/components/form'
import { EntityEntitlementsSection } from '@/features/entitlements/EntityEntitlementsSection'
import { EntitlementValueFields } from '@/features/entitlements/creation/EntitlementValueFields'
import {
  buildConfigValue,
  buildMeteredPolicy,
  configFeatureTypeValue,
  configMapIssues,
  CONFIG_KINDS,
  groupBySchema,
  checkMeteredPolicy,
  meteredPolicySchemaShape,
} from '@/features/entitlements/creation/types'
import {
  FEATURE_CODE_CHARSET_MESSAGE,
  FEATURE_CODE_LENGTH_MESSAGE,
  FEATURE_CODE_MAX_LENGTH,
  FEATURE_CODE_REGEX,
  FeatureKind,
  slugifyCode,
} from '@/features/entitlements/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  createFeature,
  listFeatures,
  updateFeature,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { CalendarUnit, EntitlementValueSchema, ResetPeriodSchema } from '@/rpc/api/entitlements/v1/models_pb';
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

function isValidJson(s: string | undefined): boolean {
  if (!s || s.trim() === '') return false
  try {
    JSON.parse(s)
    return true
  } catch {
    return false
  }
}

const schema = z
  .object({
    name: z.string().min(1, 'Required'),
    code: z
      .string()
      .min(1, 'Required')
      .max(FEATURE_CODE_MAX_LENGTH, FEATURE_CODE_LENGTH_MESSAGE)
      .regex(FEATURE_CODE_REGEX, FEATURE_CODE_CHARSET_MESSAGE),
    description: z.string().optional(),
    productId: z.string().optional(),
    type: z.enum(['boolean', 'metered', 'config']),
    metricId: z.string().optional(),
    boolEnabled: z.boolean().optional(),
    limit: z.string().optional(),
    resetPeriodType: z
      .enum(['billingCycle', 'calendar', 'fixedWindow', 'slidingWindow', 'never'])
      .optional(),
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
  .refine(d => d.type !== 'metered' || !!d.metricId, {
    message: 'Metric is required for metered features',
    path: ['metricId'],
  })
  .superRefine((d, ctx) => {
    if (d.type === 'metered') checkMeteredPolicy(d, ctx)
    if (d.type === 'config' && d.configKind === 'map') {
      const issue = configMapIssues(d.configEntries)
      if (issue) ctx.addIssue({ code: z.ZodIssueCode.custom, message: issue, path: ['configEntries'] })
    }
  })
  .refine(d => d.type !== 'config' || d.configKind !== 'json' || isValidJson(d.configJson), {
    message: 'Must be valid JSON',
    path: ['configJson'],
  })
  .refine(d => d.type !== 'config' || d.configKind !== 'select' || (d.configOptions?.length ?? 0) > 0, {
    message: 'Add at least one option',
    path: ['configOptions'],
  })
  .refine(d => d.type !== 'config' || d.configKind !== 'select' || !!d.configText, {
    message: 'Choose a value',
    path: ['configText'],
  })

type FormData = z.infer<typeof schema>

const FEATURE_TYPE_OPTIONS: { value: FormData['type']; label: string; description: string }[] = [
  { value: 'boolean', label: 'Boolean', description: 'On/off access flag.' },
  { value: 'metered', label: 'Metered', description: 'Usage counted against a metric.' },
  { value: 'config', label: 'Config', description: 'A static value or key/value map.' },
]

interface Props {
  featureId?: string
  initialName?: string
  initialCode?: string
  initialDescription?: string
  initialProductId?: string
  initialKind?: FeatureKind
  onClose?: () => void
}

function buildEntitlementValue(
  type: FormData['type'],
  data: Pick<
    FormData,
    | 'boolEnabled'
    | 'limit'
    | 'resetPeriodType'
    | 'resetUnit'
    | 'resetInterval'
    | 'meteredEnabled'
    | 'groupBy'
    | 'overageType'
    | 'gracePct'
    | 'thresholds'
    | 'configKind'
    | 'configNumber'
    | 'configBoolean'
    | 'configText'
    | 'configJson'
    | 'configEntries'
  >
): MessageInitShape<typeof EntitlementValueSchema> {
  if (type === 'boolean') {
    return {
      value: {
        case: 'booleanValue' as const,
        value: { enabled: data.boolEnabled ?? true },
      },
    }
  }
  if (type === 'config') {
    return {
      value: {
        case: 'configValue' as const,
        value: buildConfigValue(data),
      },
    }
  }
  const resetPeriod: MessageInitShape<typeof ResetPeriodSchema> =
    data.resetPeriodType === 'billingCycle'
      ? { Inner: { case: 'billingCycle' as const, value: {} } }
      : data.resetPeriodType === 'calendar'
        ? {
            Inner: {
              case: 'calendar' as const,
              value: { unit: data.resetUnit!, interval: data.resetInterval! },
            },
          }
        : data.resetPeriodType === 'fixedWindow'
          ? {
              Inner: {
                case: 'fixedWindow' as const,
                value: { unit: data.resetUnit!, interval: data.resetInterval! },
              },
            }
          : data.resetPeriodType === 'slidingWindow'
            ? {
                Inner: {
                  case: 'slidingWindow' as const,
                  value: { unit: data.resetUnit!, interval: data.resetInterval! },
                },
              }
            : { Inner: { case: 'never' as const, value: {} } }

  return {
    value: {
      case: 'meteredValue' as const,
      value: {
        limit: data.limit || undefined,
        resetPeriod,
        enabled: data.meteredEnabled ?? true,
        groupBy: data.groupBy?.trim() || undefined,
        ...buildMeteredPolicy(data),
      },
    },
  }
}

export const FeatureCreateSheet = ({
  featureId,
  initialName = '',
  initialCode = '',
  initialDescription = '',
  initialProductId,
  initialKind = { type: 'boolean' },
  onClose,
}: Props) => {
  const navigate = useNavigate()
  const handleClose = onClose ?? (() => navigate('..'))
  const queryClient = useQueryClient()
  const isEdit = !!featureId
  // A "select" config feature carries a fixed option set — editable after creation.
  const isSelectConfig = initialKind.type === 'config' && !!initialKind.options?.length

  const metricsQuery = useQuery(listBillableMetrics, { pagination: { page: 0, perPage: 100 } })
  const metrics = (metricsQuery.data?.billableMetrics ?? []).slice().sort((a, b) => a.name.localeCompare(b.name))

  const productsQuery = useQuery(listProducts, { pagination: { page: 0, perPage: 200 } })
  const products = (productsQuery.data?.products ?? []).slice().sort((a, b) => a.name.localeCompare(b.name))

  const form = useZodForm({
    schema,
    defaultValues: {
      name: initialName,
      code: initialCode,
      description: initialDescription,
      productId: initialProductId ?? '',
      type: initialKind.type,
      metricId: initialKind.type === 'metered' ? initialKind.metricId : '',
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
      configOptions: initialKind.type === 'config' ? (initialKind.options ?? []) : [],
    },
  })

  const type = form.watch('type')
  const selectedProductId = form.watch('productId')
  const selectedProductName = products.find(p => p.id === selectedProductId)?.name

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
      schema: listFeatures.parent,
      cardinality: undefined
    }) })

  const createMutation = useMutation(createFeature)
  const updateMutation = useMutation(updateFeature)

  const onSubmit = form.handleSubmit(async data => {
    try {
      if (isEdit) {
        if (isSelectConfig && (data.configOptions?.length ?? 0) === 0) {
          form.setError('configOptions', { message: 'Add at least one option' })
          return
        }
        const detach = !data.productId && !!initialProductId
        await updateMutation.mutateAsync({
          id: featureId,
          name: data.name,
          description: data.description,
          productId: data.productId || undefined,
          clearProductId: detach,
          ...(isSelectConfig
            ? { configOptions: data.configOptions ?? [], updateConfigOptions: true }
            : {}),
        })
      } else {
        await createMutation.mutateAsync({
          name: data.name,
          code: data.code,
          description: data.description,
          productId: data.productId || undefined,
          featureType:
            data.type === 'boolean'
              ? { Inner: { case: 'boolean', value: {} } }
              : data.type === 'config'
                ? {
                    Inner: {
                      case: 'config',
                      value: configFeatureTypeValue(
                        data.configKind ?? 'number',
                        data.configOptions ?? []
                      ),
                    },
                  }
                : { Inner: { case: 'metered', value: { metricId: data.metricId! } } },
          entitlement: create(
            EntitlementValueSchema,
            buildEntitlementValue(data.type, data)
          ),
        })
      }
      invalidate()
      handleClose()
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err)
      toast.error(`Failed: ${message}`)
    }
  })

  const isPending = createMutation.isPending || updateMutation.isPending

  return (
    <Sheet open onOpenChange={() => handleClose()}>
      <SheetContent size="small">
        <SheetHeader className="pb-2">
          <SheetTitle>{isEdit ? 'Edit Feature' : 'New Feature'}</SheetTitle>
          <Separator />
        </SheetHeader>

        <Form {...form}>
          <form onSubmit={onSubmit} className="flex flex-col gap-4 py-4">
            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Name</FormLabel>
                  <FormControl>
                    <Input
                      placeholder="e.g. Monthly API Calls"
                      {...field}
                      onChange={e => {
                        field.onChange(e)
                        // Auto-fill the code from the name until the user edits it.
                        if (!isEdit && !form.formState.dirtyFields.code) {
                          form.setValue('code', slugifyCode(e.target.value), {
                            shouldValidate: true,
                          })
                        }
                      }}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="code"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>
                    Code{' '}
                    <span className="text-muted-foreground text-xs">
                      (stable identifier{isEdit ? ', immutable' : ''})
                    </span>
                  </FormLabel>
                  <FormControl>
                    <Input placeholder="e.g. monthly_api_calls" {...field} disabled={isEdit} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="description"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Description</FormLabel>
                  <FormControl>
                    <Textarea placeholder="Optional description" rows={2} {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="productId"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Product</FormLabel>
                  <Select
                    value={field.value || '__none__'}
                    onValueChange={v => field.onChange(v === '__none__' ? '' : v)}
                  >
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue placeholder="Global (no product)" />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      <SelectItem value="__none__">Global (no product)</SelectItem>
                      {products.map(p => (
                        <SelectItem key={p.id} value={p.id}>
                          {p.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="type"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>
                    Type {isEdit && <span className="text-muted-foreground text-xs">(immutable)</span>}
                  </FormLabel>
                  <FormControl>
                    <div className="grid grid-cols-3 gap-2">
                      {FEATURE_TYPE_OPTIONS.map(opt => {
                        const selected = field.value === opt.value
                        return (
                          <button
                            type="button"
                            key={opt.value}
                            disabled={isEdit}
                            onClick={() => field.onChange(opt.value)}
                            className={cn(
                              'flex flex-col gap-1 rounded-md border p-3 text-left transition-colors',
                              selected
                                ? 'border-primary ring-1 ring-primary bg-primary/5'
                                : 'border-border hover:border-muted-foreground/40',
                              isEdit && 'cursor-not-allowed opacity-60'
                            )}
                          >
                            <span className="text-sm font-medium">{opt.label}</span>
                            <span className="text-xs text-muted-foreground leading-snug">
                              {opt.description}
                            </span>
                          </button>
                        )
                      })}
                    </div>
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            {type === 'metered' && !isEdit && (
              <FormField
                control={form.control}
                name="metricId"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Metric</FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue placeholder="Select a metric" />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        {metrics.map(m => (
                          <SelectItem key={m.id} value={m.id}>
                            {m.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
            )}

            {!isEdit && (
              <>
                <Separator />
                <div>
                  <p className="text-sm font-medium">Default entitlement</p>
                  <p className="text-xs text-muted-foreground">
                    {selectedProductName
                      ? `Applies to customers subscribed to any plan that includes ${selectedProductName}. Can be overridden per plan or subscription.`
                      : 'Applies to all customers by default. Can be overridden per plan or subscription.'}
                  </p>
                </div>
                <EntitlementValueFields
                  featureType={type}
                  idPrefix="fcs"
                  canEditConfigOptions
                  canPickConfigType
                />
              </>
            )}

            {isEdit && isSelectConfig && (
              <>
                <Separator />
                <FormField
                  control={form.control}
                  name="configOptions"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Options</FormLabel>
                      <p className="text-xs text-muted-foreground">
                        Allowed values for this select feature. An option currently in use by an
                        entitlement can&apos;t be removed.
                      </p>
                      <FormControl>
                        <div className="min-h-9 rounded-md border border-input bg-transparent px-2 py-1.5">
                          <ValuesTagInput
                            value={field.value as string[] | undefined}
                            onChange={field.onChange}
                            placeholder="Type a value and press Enter"
                          />
                        </div>
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </>
            )}

            {isEdit && featureId && (
              <>
                <Separator />
                <EntityEntitlementsSection
                  entity={{ EntityId: { case: 'featureId', value: featureId } }}
                  hint="The default for every customer. Plans, add-ons, and subscriptions can override it. Disabling the feature blocks it everywhere."
                />
              </>
            )}

            <div className="flex justify-end gap-2 pt-2">
              <Button type="button" variant="outline" onClick={() => handleClose()}>
                Cancel
              </Button>
              <Button type="submit" disabled={isPending}>
                {isEdit ? 'Save' : 'Create Feature'}
              </Button>
            </div>
          </form>
        </Form>
      </SheetContent>
    </Sheet>
  )
}
