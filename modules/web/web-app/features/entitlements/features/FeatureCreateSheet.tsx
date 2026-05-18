/**
 * FeatureCreateSheet — sheet for creating or editing a feature in the feature catalog.
 * On create, optionally configures a feature-level entitlement (lowest-priority baseline).
 * Feature-level entitlements apply when no higher-priority entitlement exists for an entity.
 */
import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
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
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  Switch,
  Textarea,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { EntityEntitlementsSection } from '@/features/entitlements/EntityEntitlementsSection'
import { EntitlementValueFields } from '@/features/entitlements/creation/EntitlementValueFields'
import { FeatureKind } from '@/features/entitlements/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  createEntitlement,
  createFeature,
  listFeatures,
  updateFeature,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  CalendarUnit,
  EntitlementValue,
  OverageBehavior,
} from '@/rpc/api/entitlements/v1/models_pb'
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

const schema = z
  .object({
    name: z.string().min(1, 'Required'),
    description: z.string().optional(),
    productId: z.string().optional(),
    type: z.enum(['boolean', 'metered']),
    metricId: z.string().optional(),
    addEntitlement: z.boolean().optional(),
    boolEnabled: z.boolean().optional(),
    limit: z.string().optional(),
    resetPeriodType: z
      .enum(['billingCycle', 'calendar', 'fixedWindow', 'slidingWindow', 'never'])
      .optional(),
    resetUnit: z.nativeEnum(CalendarUnit).optional(),
    resetInterval: z.coerce.number().int().min(1).optional(),
    overageBehaviorType: z.enum(['allow', 'block', 'none']).optional(),
    gracePeriodPct: z.coerce.number().int().min(0).optional(),
    warningThresholdPct: z.coerce.number().int().min(0).max(100).optional(),
    meteredEnabled: z.boolean().optional(),
  })
  .refine(d => d.type !== 'metered' || !!d.metricId, {
    message: 'Metric is required for metered features',
    path: ['metricId'],
  })

type FormData = z.infer<typeof schema>

interface Props {
  featureId?: string
  initialName?: string
  initialDescription?: string
  initialProductId?: string
  initialKind?: FeatureKind
}

function buildEntitlementValue(
  isBoolean: boolean,
  data: Pick<
    FormData,
    | 'boolEnabled'
    | 'limit'
    | 'resetPeriodType'
    | 'resetUnit'
    | 'resetInterval'
    | 'overageBehaviorType'
    | 'gracePeriodPct'
    | 'warningThresholdPct'
    | 'meteredEnabled'
  >
) {
  if (isBoolean) {
    return {
      value: {
        case: 'booleanValue' as const,
        value: { enabled: data.boolEnabled ?? true },
      },
    }
  }
  const resetPeriod =
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

  const overageBehavior =
    data.overageBehaviorType === 'allow'
      ? new OverageBehavior({ Inner: { case: 'allow', value: {} } })
      : data.overageBehaviorType === 'block'
        ? new OverageBehavior({ Inner: { case: 'block', value: { gracePeriodPct: data.gracePeriodPct } } })
        : undefined

  return {
    value: {
      case: 'meteredValue' as const,
      value: {
        limit: data.limit || undefined,
        resetPeriod,
        overageBehavior,
        warningThresholdPct: data.warningThresholdPct,
        enabled: data.meteredEnabled ?? true,
      },
    },
  }
}

export const FeatureCreateSheet = ({
  featureId,
  initialName = '',
  initialDescription = '',
  initialProductId,
  initialKind = { type: 'boolean' },
}: Props) => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const isEdit = !!featureId

  const metricsQuery = useQuery(listBillableMetrics, { pagination: { page: 0, perPage: 100 } })
  const metrics = (metricsQuery.data?.billableMetrics ?? []).slice().sort((a, b) => a.name.localeCompare(b.name))

  const productsQuery = useQuery(listProducts, { pagination: { page: 0, perPage: 200 } })
  const products = (productsQuery.data?.products ?? []).slice().sort((a, b) => a.name.localeCompare(b.name))

  const form = useZodForm({
    schema,
    defaultValues: {
      name: initialName,
      description: initialDescription,
      productId: initialProductId ?? '',
      type: initialKind.type,
      metricId: initialKind.type === 'metered' ? initialKind.metricId : '',
      addEntitlement: false,
      boolEnabled: true,
      resetPeriodType: 'billingCycle',
      resetUnit: CalendarUnit.MONTH,
      resetInterval: 1,
      overageBehaviorType: 'block',
      meteredEnabled: true,
    },
  })

  const type = form.watch('type')
  const addEntitlement = form.watch('addEntitlement')

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: [listFeatures.service.typeName] })

  const createMutation = useMutation(createFeature)
  const createEntitlementMutation = useMutation(createEntitlement)
  const updateMutation = useMutation(updateFeature)

  const onSubmit = form.handleSubmit(async data => {
    try {
      if (isEdit) {
        const detach = !data.productId && !!initialProductId
        await updateMutation.mutateAsync({
          id: featureId,
          name: data.name,
          description: data.description,
          productId: data.productId || undefined,
          clearProductId: detach,
        })
      } else {
        const result = await createMutation.mutateAsync({
          name: data.name,
          description: data.description,
          productId: data.productId || undefined,
          featureType:
            data.type === 'boolean'
              ? { Inner: { case: 'boolean', value: {} } }
              : { Inner: { case: 'metered', value: { metricId: data.metricId! } } },
        })
        const newFeatureId = result.feature?.id
        if (data.addEntitlement && newFeatureId) {
          const value = new EntitlementValue(buildEntitlementValue(data.type === 'boolean', data))
          await createEntitlementMutation.mutateAsync({
            featureId: newFeatureId,
            entity: { EntityId: { case: 'featureId', value: newFeatureId } },
            value,
          })
        }
      }
      invalidate()
      navigate('..')
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err)
      toast.error(`Failed: ${message}`)
    }
  })

  const isPending =
    createMutation.isPending || updateMutation.isPending || createEntitlementMutation.isPending

  return (
    <Sheet open onOpenChange={() => navigate('..')}>
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
                    <Input placeholder="e.g. Monthly API Calls" {...field} />
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
                    <RadioGroup
                      value={field.value}
                      onValueChange={field.onChange}
                      disabled={isEdit}
                      className="flex gap-4"
                    >
                      <div className="flex items-center gap-1.5">
                        <RadioGroupItem value="boolean" id="type-boolean" />
                        <Label htmlFor="type-boolean" className="font-normal cursor-pointer">
                          Boolean
                        </Label>
                      </div>
                      <div className="flex items-center gap-1.5">
                        <RadioGroupItem value="metered" id="type-metered" />
                        <Label htmlFor="type-metered" className="font-normal cursor-pointer">
                          Metered
                        </Label>
                      </div>
                    </RadioGroup>
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
                <div className="flex items-center justify-between gap-4">
                  <div>
                    <p className="text-sm font-medium">Feature-level entitlement</p>
                    <p className="text-xs text-muted-foreground">
                      Baseline applied when no higher-priority entitlement exists
                    </p>
                  </div>
                  <FormField
                    control={form.control}
                    name="addEntitlement"
                    render={({ field }) => (
                      <Switch checked={field.value ?? false} onCheckedChange={field.onChange} />
                    )}
                  />
                </div>

                {addEntitlement && (
                  <EntitlementValueFields
                    featureType={type}
                    idPrefix="fcs"
                  />
                )}
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
              <Button type="button" variant="outline" onClick={() => navigate('..')}>
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
