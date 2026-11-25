import {
  Badge,
  Button,
  Checkbox,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  cn,
} from '@md/ui'
import { MoreHorizontalIcon, PlusIcon } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import { useChargeContext } from '@/features/plans/canvas/ChargeRow'
import { billedWords, countsWords, priceWords } from '@/features/plans/canvas/billingWords'
import { type ComponentFeeType, feeTypeFromPrice, formDataToPrice } from '@/features/pricing'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import {
  FeeForm,
  InlinePanel,
  PlanChargePriceForm,
  salesCadence,
} from '@/features/subscriptions/pricecomponents/ChargeDialogs'
import {
  modelAmountRequest,
  quantityFigures,
  useModelAmounts,
  withModelAmount,
} from '@/features/subscriptions/pricecomponents/chargeFigures'
import { formatSubscriptionFee } from '@/features/subscriptions/utils/fees'
import {
  getPrice,
  getPriceBillingLabel,
  isPaidCreditPack,
  priceToSubscriptionFee,
} from '@/lib/mapping/priceToSubscriptionFee'
import { formatCurrencyNoRounding, formatUsage } from '@/lib/utils/numbers'
import {
  ComponentOverride,
  ComponentParameterization,
  ExtraComponent,
} from '@/pages/tenants/subscription/create/state'
import { PriceComponent as ProtoPriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import { Price } from '@/rpc/api/prices/v1/models_pb'

import type { ChargeContext } from '@/features/plans/canvas/billingWords'
import type { ChargeFigures } from '@/features/subscriptions/pricecomponents/chargeFigures'
import type { ReactNode } from 'react'

// --- State types ---

// Informative usage example for a usage-based component. Matched to the
// persisted quote component by `componentId` (plan price component) when
// available, otherwise by `name` (for extra components). Display-only.
export interface UsageExampleEntry {
  componentId?: string
  name?: string
  quantity: string
}

export interface PriceComponentsState {
  /** The plan option bought (its alias); unset: the default one sold in the currency. */
  option?: string
  components: {
    removed: string[]
    parameterized: ComponentParameterization[]
    overridden: ComponentOverride[]
    extra: ExtraComponent[]
    usageExamples?: UsageExampleEntry[]
  }
}

// --- Helpers ---

export function deriveFeeType(component: ProtoPriceComponent): ComponentFeeType {
  const price = component.prices[0]
  if (!price) return 'rate'
  return feeTypeFromPrice(price)
}

// --- State transitions (shared by every pricing editor) ---

export type UsageExampleKey = { componentId?: string; name?: string }

export function requiresConfiguration(
  component: ProtoPriceComponent,
  state: PriceComponentsState
): boolean {
  const feeType = deriveFeeType(component)
  // A custom seat price still bills a seat count; a custom capacity has its one level.
  if (feeType === 'slot') return true
  if (state.components.overridden.some(o => o.componentId === component.id)) return false
  return feeType === 'capacity' && component.prices.length > 1
}

export function isComponentConfigured(
  component: ProtoPriceComponent,
  state: PriceComponentsState
): boolean {
  if (!requiresConfiguration(component, state)) return true

  const config = state.components.parameterized.find(p => p.componentId === component.id)
  if (!config) return false

  const feeType = deriveFeeType(component)
  if (feeType === 'slot') return config.initialSlotCount !== undefined
  if (feeType === 'capacity' && component.prices.length > 1)
    return config.committedCapacity !== undefined
  return false
}

export function pricingConfigurationErrors(
  planComponents: ProtoPriceComponent[],
  state: PriceComponentsState
): string[] {
  return planComponents
    .filter(
      c =>
        !state.components.removed.includes(c.id) &&
        requiresConfiguration(c, state) &&
        !isComponentConfigured(c, state)
    )
    .map(c => `${c.name} requires configuration`)
}

export const pricingTransitions = {
  toggleRemoval: <S extends PriceComponentsState>(prev: S, componentId: string): S => ({
    ...prev,
    components: {
      ...prev.components,
      removed: prev.components.removed.includes(componentId)
        ? prev.components.removed.filter(id => id !== componentId)
        : [...prev.components.removed, componentId],
      parameterized: prev.components.removed.includes(componentId)
        ? prev.components.parameterized
        : prev.components.parameterized.filter(p => p.componentId !== componentId),
    },
  }),

  updateConfiguration: <S extends PriceComponentsState>(
    prev: S,
    componentId: string,
    config: Partial<ComponentParameterization>
  ): S => {
    const existing = prev.components.parameterized.find(p => p.componentId === componentId)
    return {
      ...prev,
      components: {
        ...prev.components,
        parameterized: existing
          ? prev.components.parameterized.map(p =>
              p.componentId === componentId ? { ...p, ...config } : p
            )
          : [...prev.components.parameterized, { componentId, ...config }],
      },
    }
  },

  // The seat count stays: the server applies it to the custom price. The capacity level chosen
  // was one of the plan's prices, which the custom one replaces.
  addOverride: <S extends PriceComponentsState>(
    prev: S,
    componentId: string,
    override: ComponentOverride
  ): S => ({
    ...prev,
    components: {
      ...prev.components,
      overridden: [
        ...prev.components.overridden.filter(o => o.componentId !== componentId),
        override,
      ],
      parameterized: prev.components.parameterized.flatMap(p =>
        p.componentId !== componentId
          ? [p]
          : p.initialSlotCount === undefined
            ? []
            : [{ componentId, initialSlotCount: p.initialSlotCount }]
      ),
    },
  }),

  removeOverride: <S extends PriceComponentsState>(prev: S, componentId: string): S => ({
    ...prev,
    components: {
      ...prev.components,
      overridden: prev.components.overridden.filter(o => o.componentId !== componentId),
    },
  }),

  // Capacity levels and custom prices were read off the old option's prices; seats still hold.
  changeOption: <S extends PriceComponentsState>(prev: S, option: string): S => ({
    ...prev,
    option,
    components: {
      ...prev.components,
      parameterized: prev.components.parameterized.flatMap(p =>
        p.initialSlotCount === undefined
          ? []
          : [{ componentId: p.componentId, initialSlotCount: p.initialSlotCount }]
      ),
      overridden: [],
    },
  }),

  addExtra: <S extends PriceComponentsState>(prev: S, component: ExtraComponent): S => ({
    ...prev,
    components: { ...prev.components, extra: [...prev.components.extra, component] },
  }),

  removeExtra: <S extends PriceComponentsState>(prev: S, index: number): S => ({
    ...prev,
    components: {
      ...prev.components,
      extra: prev.components.extra.filter((_, i) => i !== index),
    },
  }),

  updateExtra: <S extends PriceComponentsState>(
    prev: S,
    index: number,
    component: ExtraComponent
  ): S => ({
    ...prev,
    components: {
      ...prev.components,
      extra: prev.components.extra.map((c, i) => (i === index ? component : c)),
    },
  }),

  setExampleQuantity: <S extends PriceComponentsState>(
    prev: S,
    key: UsageExampleKey,
    quantity: string
  ): S => {
    const examples = prev.components.usageExamples ?? []
    const matches = (e: UsageExampleEntry) =>
      key.componentId ? e.componentId === key.componentId : e.name === key.name
    const without = examples.filter(e => !matches(e))
    const next =
      quantity.trim() === ''
        ? without
        : [...without, { componentId: key.componentId, name: key.name, quantity }]
    return {
      ...prev,
      components: { ...prev.components, usageExamples: next },
    }
  },
}

export function getExampleQuantity(state: PriceComponentsState, key: UsageExampleKey): string {
  const examples = state.components.usageExamples ?? []
  const match = examples.find(e =>
    key.componentId ? e.componentId === key.componentId : e.name === key.name
  )
  return match?.quantity ?? ''
}

export function getComponentBillingLabel(component: ProtoPriceComponent): string {
  const price = component.prices[0]
  if (price) return getPriceBillingLabel(price)
  return 'Monthly'
}

/** The price a plan component is shown at: its override, its committed capacity, else its own. */
export function planComponentDisplayPrice(
  component: ProtoPriceComponent,
  configuration: ComponentParameterization | undefined,
  override: ComponentOverride | undefined,
  currency: string
): Price | undefined {
  if (override) return formDataToPrice(override.feeType, override.formData, currency)
  const threshold =
    configuration?.committedCapacity !== undefined
      ? component.prices.find(
          p =>
            p.pricing.case === 'capacityPricing' &&
            p.pricing.value.included === configuration.committedCapacity
        )
      : undefined
  return threshold ?? getPrice(component)
}

/** The plan's own price of a component, the capacity level chosen: what a custom price starts from. */
const planPrice = (
  component: ProtoPriceComponent,
  configuration: ComponentParameterization | undefined,
  currency: string
) => planComponentDisplayPrice(component, configuration, undefined, currency)

/** The figures of a line's price, at its seat count. */
const lineFigures = (price: Price | undefined, seats: number | undefined, ctx?: ChargeContext) =>
  price ? quantityFigures(price, { seats, ctx }) : undefined

// --- Main component ---

interface PriceComponentsLogicProps {
  /** The plan's charges as bought in the chosen option (see `inOption`). */
  planComponents: ProtoPriceComponent[]
  currency: string
  state: PriceComponentsState
  onStateChange: (state: PriceComponentsState) => void
  onValidationChange?: (isValid: boolean, errors: string[]) => void
  /** Where a new fee's products and metrics come from. */
  familyLocalId?: string
  showUsageExamples?: boolean
  exampleAmountByKey?: Map<string, string | undefined>
}

type Editing =
  | { what: 'price'; componentId: string }
  | { what: 'fee'; index: number }
  | { what: 'newFee' }

export const PriceComponentsLogic = ({
  planComponents,
  currency,
  state,
  onStateChange,
  onValidationChange,
  familyLocalId,
  showUsageExamples = false,
  exampleAmountByKey,
}: PriceComponentsLogicProps) => {
  const [editing, setEditing] = useState<Editing>()

  const onValidationChangeRef = useRef(onValidationChange)
  onValidationChangeRef.current = onValidationChange

  const setState = (updater: (prev: PriceComponentsState) => PriceComponentsState) => {
    onStateChange(updater(state))
  }

  useEffect(() => {
    const errors = pricingConfigurationErrors(planComponents, state)
    onValidationChangeRef.current?.(errors.length === 0, errors)
  }, [state.components, planComponents])

  const getExampleAmount = (key: UsageExampleKey): string | undefined =>
    exampleAmountByKey?.get(key.componentId ?? key.name ?? '')
  const setExampleQuantity = (key: UsageExampleKey, quantity: string) =>
    setState(prev => pricingTransitions.setExampleQuantity(prev, key, quantity))

  const cadence = salesCadence(planComponents)
  const lines = planComponents.map(component => {
    const configuration = state.components.parameterized.find(p => p.componentId === component.id)
    const override = state.components.overridden.find(o => o.componentId === component.id)
    const price = planComponentDisplayPrice(component, configuration, override, currency)
    const seats = deriveFeeType(component) === 'slot' ? configuration?.initialSlotCount : undefined
    return { component, configuration, override, price, seats }
  })
  const extras = state.components.extra.map(extra => ({
    extra,
    price: formDataToPrice(extra.feeType, extra.formData, currency),
  }))
  // A count priced by a model: its amount is the server's.
  const amounts = useModelAmounts(
    [
      ...lines.flatMap(l =>
        modelAmountRequest(`c:${l.component.id}`, lineFigures(l.price, l.seats))
      ),
      ...extras.flatMap((e, i) => modelAmountRequest(`x:${i}`, lineFigures(e.price, undefined))),
    ],
    currency
  )
  const close = () => setEditing(undefined)

  return (
    <div className="overflow-hidden rounded-lg border border-border bg-card">
      <div className="divide-y divide-border">
        {lines.map(({ component, configuration, override, price, seats }) => {
          if (editing?.what === 'price' && editing.componentId === component.id) {
            return (
              <InlinePanel key={component.id}>
                <PlanChargePriceForm
                  component={component}
                  price={planPrice(component, configuration, currency)}
                  override={override}
                  currency={currency}
                  onClose={close}
                  onSave={next => {
                    setState(prev => pricingTransitions.addOverride(prev, component.id, next))
                    close()
                  }}
                />
              </InlinePanel>
            )
          }
          const excluded = state.components.removed.includes(component.id)
          return (
            <PlanChargeLine
              key={component.id}
              component={component}
              price={price}
              configuration={configuration}
              override={override}
              seats={seats}
              amounts={amounts}
              currency={currency}
              excluded={excluded}
              needsConfiguration={
                !excluded &&
                requiresConfiguration(component, state) &&
                !isComponentConfigured(component, state)
              }
              onOverride={() => setEditing({ what: 'price', componentId: component.id })}
              onResetOverride={() =>
                setState(prev => pricingTransitions.removeOverride(prev, component.id))
              }
              onToggleExclude={() =>
                setState(prev => pricingTransitions.toggleRemoval(prev, component.id))
              }
              onConfigure={config =>
                setState(prev => pricingTransitions.updateConfiguration(prev, component.id, config))
              }
              example={
                showUsageExamples
                  ? {
                      quantity: getExampleQuantity(state, { componentId: component.id }),
                      amount: getExampleAmount({ componentId: component.id }),
                      onChange: v => setExampleQuantity({ componentId: component.id }, v),
                    }
                  : undefined
              }
            />
          )
        })}

        {extras.map(({ extra, price }, index) =>
          editing?.what === 'fee' && editing.index === index ? (
            <InlinePanel key={`extra-${index}`}>
              <FeeForm
                fee={extra}
                currency={currency}
                cadence={cadence}
                familyLocalId={familyLocalId}
                onClose={close}
                onSave={fee => {
                  setState(prev => pricingTransitions.updateExtra(prev, index, fee))
                  close()
                }}
              />
            </InlinePanel>
          ) : (
            <ExtraChargeLine
              key={`extra-${index}`}
              extra={extra}
              price={price}
              figures={withModelAmount(lineFigures(price, undefined), amounts, `x:${index}`)}
              currency={currency}
              onEdit={() => setEditing({ what: 'fee', index })}
              onRemove={() => setState(prev => pricingTransitions.removeExtra(prev, index))}
              example={
                showUsageExamples
                  ? {
                      quantity: getExampleQuantity(state, { name: extra.name }),
                      amount: getExampleAmount({ name: extra.name }),
                      onChange: v => setExampleQuantity({ name: extra.name }, v),
                    }
                  : undefined
              }
            />
          )
        )}

        {!planComponents.length && !state.components.extra.length && (
          <p className="px-4 py-3 text-sm text-muted-foreground">No charges on this plan.</p>
        )}

        {editing?.what === 'newFee' ? (
          <InlinePanel>
            <FeeForm
              currency={currency}
              cadence={cadence}
              familyLocalId={familyLocalId}
              onClose={close}
              onSave={fee => {
                setState(prev => pricingTransitions.addExtra(prev, fee))
                close()
              }}
            />
          </InlinePanel>
        ) : (
          <button
            type="button"
            onClick={() => setEditing({ what: 'newFee' })}
            className="flex w-full items-center gap-1.5 px-4 py-2.5 text-sm text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground"
          >
            <PlusIcon size={14} /> Add a fee
          </button>
        )}
      </div>
    </div>
  )
}

// --- Lines: a charge's name and what it is, its figures, its actions ---

interface Example {
  quantity: string
  amount?: string
  onChange: (value: string) => void
}

/** "Per seat · billed monthly, upfront": what a price is and when it bills, in plan words. */
const describe = (price: Price | undefined, ctx: ChargeContext) =>
  price ? [countsWords(price, ctx), billedWords(price, ctx)].filter(Boolean).join(' · ') : 'Included without a price'

const LineShell = ({
  name,
  description,
  badge,
  muted,
  figures,
  actions,
  children,
}: {
  name: string
  description: string
  badge?: string
  muted?: boolean
  figures: ReactNode
  actions: ReactNode
  children?: ReactNode
}) => (
  <div className={cn('group px-4 py-3', muted && 'text-muted-foreground')}>
    <div className="flex min-h-7 items-start gap-3">
      <div className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2">
        <span
          className={cn(
            'truncate text-sm font-semibold',
            muted ? 'line-through' : 'text-foreground'
          )}
        >
          {name}
        </span>
        {!muted && <span className="text-xs text-muted-foreground">{description}</span>}
        {badge && !muted && (
          <Badge variant="outline" size="sm" className="font-normal text-muted-foreground">
            {badge}
          </Badge>
        )}
      </div>
      {!muted && <div className="max-w-[50%] text-right text-sm">{figures}</div>}
      <div className="flex shrink-0 items-center gap-0.5">{actions}</div>
    </div>
    {children}
  </div>
)

/** "10 × $12.00 per seat  $120.00", or the price in words and the server's amount. */
const Figures = ({ figures, fallback }: { figures?: ChargeFigures; fallback?: ReactNode }) => {
  if (!figures) return <>{fallback}</>
  const { quantity, unitPrice, inWords, amount } = figures
  // One unit at its own price reads as the amount alone; seats always read as a count.
  const counted = quantity !== undefined && (quantity > 1 || unitPrice !== amount)
  return (
    <div>
      {inWords ? (
        <div className="text-xs text-muted-foreground">
          {quantity !== undefined && `${formatUsage(quantity)} · `}
          {unitPrice}
        </div>
      ) : counted ? (
        <span className="text-xs text-muted-foreground">
          {formatUsage(quantity!)} × {unitPrice}
          {amount && ' = '}
        </span>
      ) : null}
      {amount ? (
        <span className="font-medium tabular-nums text-foreground">{amount}</span>
      ) : (
        !inWords && !counted && <span className="tabular-nums text-foreground">{unitPrice}</span>
      )}
    </div>
  )
}

/** Usage, a capacity or a credit pack: their price in words. */
const SummaryFigures = ({ price, ctx, currency }: { price: Price; ctx: ChargeContext; currency: string }) => {
  const creditUnits = useCreditUnits()
  if (price.pricing.case === 'creditPricing') {
    const granted = formatSubscriptionFee(
      priceToSubscriptionFee(price, { creditUnits }),
      currency,
      creditUnits
    ).details
    const { unitPrice, quantity } = price.pricing.value
    return (
      <div>
        <span className="font-medium tabular-nums text-foreground">
          {isPaidCreditPack(price)
            ? formatCurrencyNoRounding(Number(unitPrice || 0) * Math.max(quantity, 1), currency)
            : 'Included'}
        </span>
        {granted && <div className="text-xs text-muted-foreground">{granted}</div>}
      </div>
    )
  }
  return <span className="text-xs text-muted-foreground">{priceWords(price, ctx)}</span>
}

const lineMenuButton = (label: string) => (
  <DropdownMenuTrigger asChild>
    <Button
      type="button"
      variant="ghost"
      size="icon"
      className="size-7 text-muted-foreground"
      aria-label={`${label} actions`}
    >
      <MoreHorizontalIcon size={14} />
    </Button>
  </DropdownMenuTrigger>
)

const PlanChargeLine = ({
  component,
  price,
  configuration,
  override,
  seats,
  amounts,
  currency,
  excluded,
  needsConfiguration,
  onOverride,
  onResetOverride,
  onToggleExclude,
  onConfigure,
  example,
}: {
  component: ProtoPriceComponent
  price?: Price
  configuration?: ComponentParameterization
  override?: ComponentOverride
  seats?: number
  amounts: Map<string, string | undefined>
  currency: string
  excluded: boolean
  needsConfiguration: boolean
  onOverride: () => void
  onResetOverride: () => void
  onToggleExclude: () => void
  onConfigure: (config: Partial<ComponentParameterization>) => void
  example?: Example
}) => {
  const ctx = useChargeContext(component)
  const feeType = deriveFeeType(component)
  const name = override?.name || component.name
  const figures = withModelAmount(lineFigures(price, seats, ctx), amounts, `c:${component.id}`)
  const levels = !override && feeType === 'capacity' && component.prices.length > 1

  const actions = excluded ? (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      className="h-7 px-2 text-xs"
      onClick={onToggleExclude}
    >
      Include
    </Button>
  ) : (
    <>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
        onClick={onOverride}
      >
        {override ? 'Edit custom price' : 'Override price'}
      </Button>
      <DropdownMenu modal={false}>
        {lineMenuButton(name)}
        <DropdownMenuContent align="end">
          {override && (
            <>
              <DropdownMenuItem onSelect={onResetOverride}>Reset to plan price</DropdownMenuItem>
              <DropdownMenuSeparator />
            </>
          )}
          <DropdownMenuItem onSelect={onToggleExclude} className="text-destructive">
            Exclude from this subscription
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  )

  return (
    <LineShell
      name={name}
      description={describe(price, ctx)}
      badge={override ? 'Custom price' : undefined}
      muted={excluded}
      figures={
        needsConfiguration && !levels ? (
          <span className="text-xs text-warning">Set the seat count</span>
        ) : (
          <Figures
            figures={figures}
            fallback={price && <SummaryFigures price={price} ctx={ctx} currency={currency} />}
          />
        )
      }
      actions={actions}
    >
      {!excluded && feeType === 'slot' && (
        <SeatCount
          name={name}
          value={configuration?.initialSlotCount}
          invalid={needsConfiguration}
          onChange={count => onConfigure({ initialSlotCount: count })}
        />
      )}
      {!excluded && levels && (
        <div className="mt-2 w-60">
          <ComponentConfigurationFields
            component={component}
            configuration={configuration}
            currency={currency}
            onUpdateConfiguration={onConfigure}
          />
        </div>
      )}
      {!excluded && example && isExampleUsage(price) && <UsageExample {...example} currency={currency} />}
    </LineShell>
  )
}

const ExtraChargeLine = ({
  extra,
  price,
  figures,
  currency,
  onEdit,
  onRemove,
  example,
}: {
  extra: ExtraComponent
  price: Price
  figures?: ChargeFigures
  currency: string
  onEdit: () => void
  onRemove: () => void
  example?: Example
}) => {
  const ctx: ChargeContext = {}
  return (
    <LineShell
      name={extra.name}
      description={describe(price, ctx)}
      badge="Added"
      figures={
        <Figures
          figures={figures}
          fallback={<SummaryFigures price={price} ctx={ctx} currency={currency} />}
        />
      }
      actions={
        <>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
            onClick={onEdit}
          >
            Edit fee
          </Button>
          <DropdownMenu modal={false}>
            {lineMenuButton(extra.name)}
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={onRemove} className="text-destructive">
                Remove fee
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </>
      }
    >
      {example && isExampleUsage(price) && <UsageExample {...example} currency={currency} />}
    </LineShell>
  )
}

// Usage priced by dimension has no one quantity to show an example at.
const isExampleUsage = (price: Price | undefined) =>
  price?.pricing.case === 'usagePricing' &&
  price.pricing.value.model.case !== 'dimensionRates' &&
  price.pricing.value.model.case !== 'rateCard'

/** The seats a seat charge starts with: its price, custom or not, bills them. */
const SeatCount = ({
  name,
  value,
  invalid,
  onChange,
}: {
  name: string
  value?: number
  invalid: boolean
  onChange: (count: number | undefined) => void
}) => (
  <label className="mt-2 flex items-center gap-2 text-xs text-muted-foreground">
    Starts with
    <Input
      type="number"
      min="0"
      value={value ?? ''}
      onChange={e => onChange(e.target.value ? parseInt(e.target.value) : undefined)}
      aria-label={`${name} seats`}
      aria-invalid={invalid}
      className={cn('h-7 w-20 px-2 text-sm', invalid && 'border-warning')}
    />
    {value === 1 ? 'seat' : 'seats'}
  </label>
)

export const ComponentConfigurationFields = ({
  component,
  configuration,
  currency,
  onUpdateConfiguration,
}: {
  component: ProtoPriceComponent
  configuration?: ComponentParameterization
  currency: string
  onUpdateConfiguration: (config: Partial<ComponentParameterization>) => void
}) => {
  const feeType = deriveFeeType(component)

  if (feeType === 'slot') {
    return (
      <SeatCount
        name={component.name}
        value={configuration?.initialSlotCount}
        invalid={configuration?.initialSlotCount === undefined}
        onChange={count => onUpdateConfiguration({ initialSlotCount: count })}
      />
    )
  }

  if (feeType === 'capacity' && component.prices.length > 1) {
    return (
      <Select
        value={configuration?.committedCapacity?.toString() || ''}
        onValueChange={value => onUpdateConfiguration({ committedCapacity: BigInt(value) })}
      >
        <SelectTrigger className="h-8 text-xs" aria-label={`${component.name} level`}>
          <SelectValue placeholder="Choose the included amount" />
        </SelectTrigger>
        <SelectContent>
          {component.prices
            .filter(p => p.pricing.case === 'capacityPricing')
            .map(p => {
              const cap = p.pricing.value as { included: bigint; rate: string }
              return (
                <SelectItem key={cap.included.toString()} value={cap.included.toString()}>
                  <div className="flex w-full items-center justify-between">
                    <span>{formatUsage(Number(cap.included))} included</span>
                    <span className="ml-2 text-muted-foreground">
                      {formatCurrencyNoRounding(cap.rate, currency)}
                    </span>
                  </div>
                </SelectItem>
              )
            })}
        </SelectContent>
      </Select>
    )
  }

  return null
}

/** An example quantity of usage, priced by the server, shown on the quote. */
const UsageExample = ({
  quantity,
  amount,
  currency,
  onChange,
}: Example & { currency: string }) => {
  const [enabled, setEnabled] = useState(Boolean(quantity))

  const toggle = (checked: boolean) => {
    setEnabled(checked)
    if (!checked) onChange('')
  }

  const qty = Number(quantity)
  const showAmount = quantity.trim() && Number.isFinite(qty) && amount

  return (
    <div className="mt-2 space-y-2">
      <label className="flex w-fit cursor-pointer items-center gap-2 text-xs text-muted-foreground">
        <Checkbox checked={enabled} onCheckedChange={v => toggle(v === true)} />
        Add example pricing
      </label>
      {enabled && (
        <div className="flex items-center gap-2">
          <Input
            type="number"
            min="0"
            className="h-7 max-w-[140px] text-xs"
            placeholder="e.g. 10000"
            value={quantity}
            onChange={e => onChange(e.target.value)}
          />
          {showAmount && (
            <span className="whitespace-nowrap text-xs text-muted-foreground">
              {qty.toLocaleString()} units = {formatCurrencyNoRounding(amount, currency || 'USD')}
            </span>
          )}
        </div>
      )}
    </div>
  )
}
