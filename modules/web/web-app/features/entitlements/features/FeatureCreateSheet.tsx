import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import {
  Alert,
  AlertDescription,
  Button,
  Form,
  GenericFormField,
  InputFormField,
  SelectFormField,
  SelectItem,
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SwitchFormField,
  TextareaFormField,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CopyIcon, InfoIcon } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { Link, useNavigate } from 'react-router'
import { toast } from 'sonner'
import { z } from 'zod'

import ConfirmationModal from '@/components/ConfirmationModal'
import { type ChoiceCardOption, ChoiceCardsFormField, ValuesTagInput } from '@/components/form'
import { PolicyFields } from '@/features/entitlements/PolicyFields'
import { KIND_ICON, KIND_NAME, type KindKey } from '@/features/entitlements/features/featureKind'
import { featureSnippet } from '@/features/entitlements/features/featureSnippet'
import { policyFromForm, policySchema, policyToForm } from '@/features/entitlements/model'
import {
  FEATURE_CODE_CHARSET_MESSAGE,
  FEATURE_CODE_LENGTH_MESSAGE,
  FEATURE_CODE_MAX_LENGTH,
  FEATURE_CODE_REGEX,
  slugifyCode,
} from '@/features/entitlements/utils'
import { TypeSelect } from '@/features/resources/TypeSelect'
import { lower, useResourceTypes } from '@/features/resources/hooks'
import { useBasePath } from '@/hooks/useBasePath'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { copyToClipboard } from '@/lib/helpers'
import {
  getBillableMetric,
  listBillableMetrics,
} from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  createFeature,
  listEntitlementsByFeature,
  updateFeature,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  type Feature,
  FeatureKind,
  FeatureValueType,
  LimitCountedBy,
} from '@/rpc/api/entitlements/v1/models_pb'
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

const KINDS = ['BOOLEAN', 'VALUE', 'LIMIT'] as const
const GLOBAL = '__global__'
const VALUE_TYPES = ['NUMBER', 'TEXT', 'SELECT', 'JSON'] as const
const VALUE_TYPE_LABEL: Record<(typeof VALUE_TYPES)[number], string> = {
  NUMBER: 'Number',
  TEXT: 'Text',
  SELECT: 'Select',
  JSON: 'JSON',
}

const KIND_HINT: Record<KindKey, string> = {
  BOOLEAN: 'Access granted or not',
  VALUE: 'A number, text or a choice',
  LIMIT: 'A cap on counted usage',
}

const KIND_OPTIONS: ChoiceCardOption<KindKey>[] = KINDS.map(kind => {
  const Icon = KIND_ICON[kind]
  return {
    value: kind,
    label: KIND_NAME[kind],
    hint: KIND_HINT[kind],
    icon: <Icon size={14} className="text-muted-foreground" />,
  }
})

const COUNTED_BY_OPTIONS: ChoiceCardOption<'EVENTS' | 'APP'>[] = [
  { value: 'EVENTS', label: 'From events', hint: 'A metric counts it' },
  { value: 'APP', label: 'By your app', hint: 'Your app reports the count' },
]

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
    kind: z.enum(KINDS),
    valueType: z.enum(VALUE_TYPES),
    higherIsBetter: z.boolean(),
    options: z.array(z.string()),
    ordered: z.boolean(),
    countedBy: z.enum(['EVENTS', 'APP']),
    metricId: z.string().optional(),
    unit: z.string().optional(),
    levelTypeId: z.string().optional(),
    policy: policySchema,
  })
  .refine(d => d.kind !== 'LIMIT' || d.countedBy !== 'EVENTS' || !!d.metricId, {
    message: 'Pick the metric that counts it',
    path: ['metricId'],
  })
  .refine(d => d.kind !== 'VALUE' || d.valueType !== 'SELECT' || d.options.length > 0, {
    message: 'Add at least one option',
    path: ['options'],
  })

type FormData = z.infer<typeof schema>

const toForm = (f?: Feature, productId?: string): FormData => ({
  name: f?.name ?? '',
  code: f?.code ?? '',
  description: f?.description ?? '',
  productId: f?.product?.id ?? productId ?? '',
  kind: f && f.kind !== FeatureKind.UNSPECIFIED ? (FeatureKind[f.kind] as FormData['kind']) : 'BOOLEAN',
  valueType:
    f?.value && f.value.type !== FeatureValueType.UNSPECIFIED
      ? (FeatureValueType[f.value.type] as FormData['valueType'])
      : 'NUMBER',
  higherIsBetter: f?.value?.higherIsBetter ?? true,
  options: f?.value?.options ?? [],
  ordered: f?.value?.ordered ?? false,
  countedBy: f?.limit?.countedBy === LimitCountedBy.APP ? 'APP' : 'EVENTS',
  metricId: f?.limit?.metricId ?? '',
  unit: f?.unit ?? '',
  levelTypeId: f?.levelResourceTypeId,
  policy: policyToForm(f?.policy, f?.limit?.countedBy === LimitCountedBy.APP),
})

const hasUnit = (d: Pick<FormData, 'kind' | 'valueType'>) =>
  d.kind === 'LIMIT' || (d.kind === 'VALUE' && d.valueType === 'NUMBER')

interface Props {
  feature?: Feature
  initialProductId?: string
  onClose?: (saved?: Feature) => void
}

export const FeatureCreateSheet = ({ feature, initialProductId, onClose }: Props) => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  const queryClient = useQueryClient()
  const isEdit = !!feature
  const { byId: typesById, hasTypes } = useResourceTypes()
  const [confirmingClose, setConfirmingClose] = useState(false)

  const metrics =
    useQuery(listBillableMetrics, { pagination: { page: 0, perPage: 200 } }).data?.billableMetrics ?? []
  const products = useQuery(listProducts, { pagination: { page: 0, perPage: 200 } }).data?.products ?? []

  const form = useZodForm({ schema, defaultValues: toForm(feature, initialProductId) })
  const { isDirty } = form.formState
  const d = useWatch({ control: form.control }) as FormData
  const appCounted = d.kind === 'LIMIT' && d.countedBy === 'APP'
  useEffect(() => {
    if (!isEdit && !form.formState.dirtyFields.code) form.setValue('code', slugifyCode(d.name ?? ''))
  }, [d.name]) // eslint-disable-line react-hooks/exhaustive-deps
  const type = d.levelTypeId ? typesById.get(d.levelTypeId) : undefined

  // A per-type limit from events needs a metric attributed to that type.
  const metricQuery = useQuery(
    getBillableMetric,
    d.kind === 'LIMIT' && d.countedBy === 'EVENTS' && d.metricId ? { id: d.metricId } : skipToken
  )
  const metric = metricQuery.data?.billableMetric
  const metricPending =
    !!d.levelTypeId &&
    d.kind === 'LIMIT' &&
    d.countedBy === 'EVENTS' &&
    !!d.metricId &&
    metricQuery.isPending
  const misattributed =
    !!type &&
    d.kind === 'LIMIT' &&
    d.countedBy === 'EVENTS' &&
    !!metric &&
    metric.attributedTo?.resourceTypeId !== type.id

  // A feature with entitlements keeps its level: they were written for it.
  const levelChanged = !!feature && (d.levelTypeId ?? '') !== (feature.levelResourceTypeId ?? '')
  const blockingQuery = useQuery(
    listEntitlementsByFeature,
    feature && levelChanged ? { featureId: feature.id } : skipToken
  )
  const blocking = blockingQuery.data?.entitlements
  const blockingPending = levelChanged && blockingQuery.isPending
  const blockingCount = levelChanged ? (blocking?.length ?? 0) : 0
  const blocked = blockingCount > 0
  const guarded = misattributed || metricPending || blocked || blockingPending

  const snippet = useMemo(
    () => featureSnippet({ code: d.code, kind: d.kind, countedBy: d.countedBy, type, baseUrl: env.meteroidRestApiUri }),
    [d.code, d.kind, d.countedBy, type]
  )

  const close = (saved?: Feature) => {
    if (onClose) onClose(saved)
    else navigate(saved && !isEdit ? `../${saved.id}` : '..')
  }
  const safeClose = () => (isDirty ? setConfirmingClose(true) : close())

  const done = (saved?: Feature) => {
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: createFeature.parent, cardinality: undefined }),
    })
    close(saved)
  }
  const createMutation = useMutation(createFeature, {
    onSuccess: r => done(r.feature),
    onError: e => toast.error(e.message),
  })
  const updateMutation = useMutation(updateFeature, {
    onSuccess: r => done(r.feature),
    onError: e => toast.error(e.message),
  })

  const onSubmit = form.handleSubmit(data => {
    if (guarded) return
    const policy = data.kind === 'LIMIT' ? policyFromForm(data.policy, appCounted) : undefined
    const unit = hasUnit(data) ? data.unit?.trim() || undefined : undefined
    const productId = data.productId === GLOBAL ? undefined : data.productId || undefined
    if (feature)
      return updateMutation.mutate({
        id: feature.id,
        name: data.name,
        description: data.description,
        productId,
        clearProductId: !productId && !!feature.product,
        unit,
        clearUnit: !unit && !!feature.unit,
        levelResourceTypeId: data.levelTypeId,
        clearLevelResourceTypeId: !data.levelTypeId && !!feature.levelResourceTypeId,
        policy,
        higherIsBetter: data.kind === 'VALUE' && data.valueType === 'NUMBER' ? data.higherIsBetter : undefined,
        selectOrdered: data.kind === 'VALUE' && data.valueType === 'SELECT' ? data.ordered : undefined,
        configOptions: data.options,
        updateConfigOptions: data.kind === 'VALUE' && data.valueType === 'SELECT',
      })
    createMutation.mutate({
      name: data.name,
      code: data.code,
      description: data.description,
      productId,
      kind: FeatureKind[data.kind],
      value:
        data.kind === 'VALUE'
          ? {
              type: FeatureValueType[data.valueType],
              higherIsBetter: data.valueType === 'NUMBER' ? data.higherIsBetter : undefined,
              options: data.valueType === 'SELECT' ? data.options : [],
              ordered: data.valueType === 'SELECT' && data.ordered,
            }
          : undefined,
      limit:
        data.kind === 'LIMIT'
          ? {
              countedBy: LimitCountedBy[data.countedBy],
              metricId: data.countedBy === 'EVENTS' ? data.metricId : undefined,
            }
          : undefined,
      unit,
      levelResourceTypeId: data.levelTypeId,
      policy,
    })
  })

  const saving = createMutation.isPending || updateMutation.isPending

  return (
    <>
      <Sheet open onOpenChange={safeClose}>
        <SheetContent size="xlarge" className="p-0 sm:max-w-[1040px]">
          <Form {...form}>
            <form onSubmit={onSubmit} className="flex h-full w-full flex-col">
              <SheetHeader className="ml-6 mt-4">
                <SheetTitle>{feature ? `Edit ${feature.name}` : 'New feature'}</SheetTitle>
              </SheetHeader>

              <div className="flex-1 overflow-y-auto">
                <div className="grid w-full gap-6 p-6 lg:grid-cols-[minmax(0,1fr)_320px]">
                  <div className="flex min-w-0 flex-col gap-4">
                    <section className="space-y-4 rounded-lg border border-border p-5">
                      <div className="grid gap-4 sm:grid-cols-2">
                        <InputFormField
                          name="name"
                          label="Name"
                          placeholder="e.g. Projects"
                          control={form.control}
                        />
                        <InputFormField
                          name="code"
                          label="Code"
                          className="font-mono"
                          disabled={isEdit}
                          control={form.control}
                          labelTooltip={<InfoTooltip>What your code checks. Can’t be changed later.</InfoTooltip>}
                        />
                      </div>
                      <TextareaFormField
                        name="description"
                        label="Description (optional)"
                        rows={2}
                        control={form.control}
                      />
                      <SelectFormField
                        name="productId"
                        label="Product"
                        control={form.control}
                        placeholder="All products"
                      >
                        <SelectItem value={GLOBAL}>All products</SelectItem>
                        {products.map(p => (
                          <SelectItem key={p.id} value={p.id}>
                            {p.name}
                          </SelectItem>
                        ))}
                      </SelectFormField>
                    </section>

                    <section className="space-y-4 rounded-lg border border-border p-5">
                      <ChoiceCardsFormField
                        name="kind"
                        label="Kind"
                        control={form.control}
                        columns={3}
                        disabled={isEdit}
                        options={KIND_OPTIONS}
                      />
                    </section>

                    {d.kind === 'VALUE' && (
                      <section className="space-y-4 rounded-lg border border-border p-5">
                        <h3 className="text-sm font-semibold">Value</h3>
                        <div className="grid gap-4 sm:grid-cols-2">
                          <SelectFormField
                            name="valueType"
                            label="Type"
                            control={form.control}
                            disabled={isEdit}
                          >
                            {VALUE_TYPES.map(v => (
                              <SelectItem key={v} value={v}>
                                {VALUE_TYPE_LABEL[v]}
                              </SelectItem>
                            ))}
                          </SelectFormField>
                          {d.valueType === 'NUMBER' && (
                            <InputFormField
                              name="unit"
                              label="Unit"
                              placeholder="e.g. days"
                              control={form.control}
                            />
                          )}
                        </div>
                        {d.valueType === 'NUMBER' && (
                          <SwitchFormField
                            name="higherIsBetter"
                            label="Higher is better"
                            control={form.control}
                            containerClassName="py-0"
                          />
                        )}
                        {d.valueType === 'SELECT' && (
                          <>
                            <GenericFormField
                              name="options"
                              label={d.ordered ? 'Options (lowest first)' : 'Options'}
                              control={form.control}
                              render={({ field }) => (
                                <ValuesTagInput value={field.value} onChange={field.onChange} />
                              )}
                            />
                            <SwitchFormField
                              name="ordered"
                              label="Ordered"
                              control={form.control}
                              containerClassName="py-0"
                            />
                          </>
                        )}
                      </section>
                    )}

                    {d.kind === 'LIMIT' && (
                      <section className="space-y-4 rounded-lg border border-border p-5">
                        <h3 className="text-sm font-semibold">Limit</h3>
                        <InputFormField
                          name="unit"
                          label="Unit"
                          placeholder="e.g. projects"
                          control={form.control}
                          containerClassName="sm:max-w-xs"
                        />
                        <ChoiceCardsFormField
                          name="countedBy"
                          label="Usage is counted"
                          control={form.control}
                          columns={2}
                          disabled={isEdit}
                          options={COUNTED_BY_OPTIONS}
                          onValueChange={v =>
                            form.setValue('policy', policyToForm(undefined, v === 'APP'), {
                              shouldDirty: true,
                            })
                          }
                        />
                        {d.countedBy === 'EVENTS' && (
                          <SelectFormField
                            name="metricId"
                            label="Metric"
                            disabled={isEdit}
                            control={form.control}
                            placeholder="Select a metric"
                            containerClassName="sm:max-w-xs"
                          >
                            {metrics.map(m => (
                              <SelectItem key={m.id} value={m.id}>
                                {m.name}
                              </SelectItem>
                            ))}
                          </SelectFormField>
                        )}
                        <PolicyFields name="policy" appCounted={appCounted} />
                      </section>
                    )}

                    {hasTypes && (
                      <section className="space-y-4 rounded-lg border border-border p-5">
                        <GenericFormField
                          name="levelTypeId"
                          label="Applies per"
                          control={form.control}
                          containerClassName="sm:max-w-xs"
                          render={({ field }) => (
                            <TypeSelect value={field.value} onChange={field.onChange} />
                          )}
                        />
                        {misattributed && type && metric && (
                          <Alert variant="warning">
                            <AlertDescription className="flex flex-col gap-2">
                              <span>
                                {metric.name} isn’t attributed to {lower(type.pluralName)}.
                              </span>
                              <span className="flex flex-wrap gap-2">
                                <Button asChild size="sm" variant="outline">
                                  <Link to={`${basePath}/metrics/edit/${metric.id}`}>
                                    Attribute to {lower(type.name)}
                                  </Link>
                                </Button>
                                <Button
                                  type="button"
                                  size="sm"
                                  variant="outline"
                                  onClick={() =>
                                    form.setValue('levelTypeId', undefined, { shouldDirty: true })
                                  }
                                >
                                  Apply per customer
                                </Button>
                              </span>
                            </AlertDescription>
                          </Alert>
                        )}
                        {blocked && (
                          <Alert variant="destructive">
                            <AlertDescription>
                              Used by {blockingCount} entitlement{blockingCount > 1 ? 's' : ''}:
                              remove them to change where it applies.
                            </AlertDescription>
                          </Alert>
                        )}
                      </section>
                    )}
                  </div>

                  <aside className="hidden lg:block">
                    <div className="sticky top-6 space-y-3 rounded-lg border border-border p-5">
                      <div className="flex items-center justify-between gap-2">
                        <h3 className="text-sm font-semibold">In your code</h3>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7"
                          aria-label="Copy"
                          onClick={() => copyToClipboard(snippet, () => toast.success('Copied'))}
                        >
                          <CopyIcon size={14} />
                        </Button>
                      </div>
                      <pre className="overflow-x-auto whitespace-pre-wrap break-words rounded-md bg-muted p-3 font-mono text-xs leading-relaxed text-foreground">
                        {snippet}
                      </pre>
                    </div>
                  </aside>
                </div>
              </div>

              <SheetFooter className="border-t border-border p-3">
                <Button type="button" variant="outline" onClick={safeClose}>
                  Cancel
                </Button>
                <Button type="submit" disabled={guarded || saving}>
                  {isEdit ? 'Save' : 'Create feature'}
                </Button>
              </SheetFooter>
            </form>
          </Form>
        </SheetContent>
      </Sheet>

      <ConfirmationModal
        visible={confirmingClose}
        danger
        header="Discard unsaved changes?"
        buttonLabel="Discard"
        onSelectCancel={() => setConfirmingClose(false)}
        onSelectConfirm={() => {
          setConfirmingClose(false)
          close()
        }}
      />
    </>
  )
}

const InfoTooltip = ({ children }: { children: React.ReactNode }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <InfoIcon className="h-3.5 w-3.5 cursor-help text-muted-foreground" />
    </TooltipTrigger>
    <TooltipContent multiline className="max-w-72">{children}</TooltipContent>
  </Tooltip>
)
