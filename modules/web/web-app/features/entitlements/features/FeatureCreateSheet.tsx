/**
 * FeatureCreateSheet — sheet for creating or editing a feature in the feature catalog.
 * On create, always configures a feature-level entitlement (lowest-priority baseline).
 * Feature-level entitlements apply when no higher-priority entitlement exists for an entity.
 */
import { create, type MessageInitShape } from '@bufbuild/protobuf';
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query';
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
  Textarea,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { EntityEntitlementsSection } from '@/features/entitlements/EntityEntitlementsSection'
import { EntitlementValueFields } from '@/features/entitlements/creation/EntitlementValueFields'
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
    type: z.enum(['boolean', 'metered']),
    metricId: z.string().optional(),
    boolEnabled: z.boolean().optional(),
    limit: z.string().optional(),
    resetPeriodType: z
      .enum(['billingCycle', 'calendar', 'fixedWindow', 'slidingWindow', 'never'])
      .optional(),
    resetUnit: z.nativeEnum(CalendarUnit).optional(),
    resetInterval: z.coerce.number().int().min(1).optional(),
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
  initialCode?: string
  initialDescription?: string
  initialProductId?: string
  initialKind?: FeatureKind
  onClose?: () => void
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
    | 'meteredEnabled'
  >
): MessageInitShape<typeof EntitlementValueSchema> {
  if (isBoolean) {
    return {
      value: {
        case: 'booleanValue' as const,
        value: { enabled: data.boolEnabled ?? true },
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
        const detach = !data.productId && !!initialProductId
        await updateMutation.mutateAsync({
          id: featureId,
          name: data.name,
          description: data.description,
          productId: data.productId || undefined,
          clearProductId: detach,
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
              : { Inner: { case: 'metered', value: { metricId: data.metricId! } } },
          entitlement: create(
            EntitlementValueSchema,
            buildEntitlementValue(data.type === 'boolean', data)
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
