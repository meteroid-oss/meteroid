import { Button, Form } from '@md/ui'
import { XIcon } from 'lucide-react'
import { useMemo, useState, type ReactNode } from 'react'
import { useWatch } from 'react-hook-form'

import { termFormValue } from '@/features/plans/currencies/priceCells'
import { useComponentProduct } from '@/features/plans/pricecomponents/componentRefs'
import { feeTypeToHuman } from '@/features/plans/pricecomponents/utils'
import { feeTypeFromPrice } from '@/features/pricing/conversions'
import {
  buildDefaults,
  extractStructuralInfo,
  protoFeeTypeToComponentFeeType,
} from '@/features/pricing/formData'
import { ProductIcon } from '@/features/productCatalog/items/ProductIcon'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { listProductsWithPrices } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import {
  ChargeBody,
  ChargeLayout,
  ChargeNameInput,
  ContextChip,
  FormFooter,
  KindPicker,
} from './InlineChargeFields'
import { PreviewNote } from './InvoicePreview'
import { compileCharge, decompileCharge } from './compile'
import { kindOf, pickKind, valuesForKind } from './kinds'
import { useChargeForm } from './useChargeForm'

import type { ChargeBodyContext } from './InlineChargeFields'
import type { ChargeLock, ChargeShape, ChargeValues, CompiledCharge } from './compile'
import type { ChargeKind } from './kinds'
import type { ComponentFeeType } from '@/features/pricing/conversions'
import type { StructuralInfo } from '@/features/pricing/formData'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Product, ProductWithPrice } from '@/rpc/api/products/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

// The charge form without a plan around it: what it charges for, then its fields, beside the line
// it puts on an invoice. The plan editor, subscriptions, quotes and add-ons price their charges
// with it.

/** A charge as callers keep it: the fee type and form data conversions.ts builds requests from. */
export interface StoredCharge {
  feeType: ComponentFeeType
  formData: Record<string, unknown>
}

/** What the form saves: a priced charge, or (plans only) the product with no price. */
export type ChargeFormResult =
  | { included: true }
  | ({ included: false; kind: ChargeKind; shape: ChargeShape } & CompiledCharge)

export interface ChargeFormState {
  kind?: ChargeKind
  values: ChargeValues
}

export interface ChargeFormProps {
  currency: string
  /** The cadence a recurring charge bills on, until changed beside its amount. */
  cadence: BillingPeriod
  /** The charge's name, its invoice line's in the preview. */
  name?: string
  /** What the product is sold as: only the kinds compiling to it are offered. */
  lock?: ChargeLock
  /** The product's metric and seat unit, shown read-only. */
  product?: ChargeBodyContext
  /** A new product names its metric and seat unit in the form. Default: no `lock`. */
  newProduct?: boolean
  /** The charge it starts from; none: a new charge, priced once its kind is picked. */
  initial?: StoredCharge
  /** Where a new usage product's metric is picked from. */
  familyLocalId?: string
  /** "Included, no charge": offered, shown greyed out, or (default) not shown. */
  includable?: boolean | 'disabled'
  /** Opens on "Included, no charge" (a plan charge without a price). */
  startsIncluded?: boolean
  /**
   * "Usage levels" is offered and keeps several levels, the customer picking one (a plan's
   * charge). Default: a capacity charge prices its one level, and is not offered otherwise.
   */
  manyLevels?: boolean
  /** "Credit pack" is offered, as an add-on sells it. */
  creditPack?: boolean
  /** Kinds not offered here, e.g. seats on an add-on. */
  hiddenKinds?: ChargeKind[]
  /** Between the kind picker and the fields: notes and warnings. */
  notice?: ReactNode | ((state: ChargeFormState) => ReactNode)
  submitLabel: string
  pending?: boolean
  remove?: { label: string; onClick: () => void; disabled?: boolean }
  onCancel: () => void
  onSubmit: (result: ChargeFormResult) => void | Promise<unknown>
}

const START: ChargeShape = { source: 'fixed', model: 'flat', recurrence: 'recurring', billing: 'advance' }

const INCLUDED_NOTE = 'Included in every currency. Its features come with the plan, nothing is billed.'

/** One charge's kind and price, in one currency. */
export const ChargeForm = ({
  currency,
  cadence,
  name,
  lock,
  product = {},
  newProduct = !lock,
  initial,
  familyLocalId,
  includable = false,
  startsIncluded = false,
  manyLevels = false,
  creditPack = false,
  hiddenKinds = [],
  notice,
  submitLabel,
  pending,
  remove,
  onCancel,
  onSubmit,
}: ChargeFormProps) => {
  // Read once: the form keeps its own state from there.
  const [start] = useState(() =>
    initial
      ? decompileCharge(initial.feeType, {
          ...initial.formData,
          term: initial.formData.term ?? termFormValue(cadence),
        })
      : undefined
  )
  const [shape, setShape] = useState<ChargeShape>(start?.shape ?? START)
  const [included, setIncluded] = useState(startsIncluded)
  const [picked, setPicked] = useState<ChargeKind>()
  const methods = useChargeForm({
    shape,
    defaultValues: start?.values ?? {
      ...valuesForKind('fixed', START, {}),
      term: termFormValue(cadence),
    },
    currency,
    newProduct,
  })
  const values = (useWatch({ control: methods.control }) ?? {}) as ChargeValues
  // A stored charge reads as what it is; a new one waits for its kind.
  const kind: ChargeKind | undefined = included
    ? 'included'
    : (picked ?? (start ? kindOf(shape) : undefined))

  const pick = (next: ChargeKind) => {
    if (next === 'included') return setIncluded(true)
    const current = methods.getValues() as ChargeValues
    const picking = pickKind(next, lock, current, shape)
    if (!picking) return
    methods.reset({ ...picking.values, term: current.term ?? termFormValue(cadence) })
    setShape(picking.shape)
    setPicked(next)
    setIncluded(false)
  }

  const run = (result: ChargeFormResult) =>
    void Promise.resolve(onSubmit(result)).catch(() => undefined)
  const submit = () => {
    if (included) return run({ included: true })
    methods.handleSubmit(data => {
      if (!kind) return
      run({ included: false, kind, shape, ...compileCharge(shape, data as ChargeValues, lock) })
    })()
  }

  const actions = (
    <>
      <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
        Cancel
      </Button>
      {kind && (
        <Button type="button" variant="primary" size="sm" disabled={pending} onClick={submit}>
          {submitLabel}
        </Button>
      )}
    </>
  )

  const lead = (
    <>
      <KindPicker
        kind={kind}
        lock={lock}
        unavailable={includable === 'disabled' ? ['included'] : []}
        hidden={[
          ...(includable ? [] : (['included'] as ChargeKind[])),
          ...(manyLevels ? [] : (['levels'] as ChargeKind[])),
          ...hiddenKinds,
        ]}
        extra={creditPack ? ['credit'] : []}
        onPick={pick}
      />
      {typeof notice === 'function' ? notice({ kind, values }) : notice}
    </>
  )

  if (!kind || included) {
    return (
      <ChargeLayout
        preview={<PreviewNote name={name} included={included} />}
        footer={<FormFooter>{actions}</FormFooter>}
      >
        {lead}
        {included && (
          <p className="rounded-md border border-border bg-background px-3 py-2 text-sm text-muted-foreground">
            {INCLUDED_NOTE}
          </p>
        )}
      </ChargeLayout>
    )
  }
  return (
    <Form {...methods}>
      <ChargeBody
        shape={shape}
        onShapeChange={setShape}
        methods={methods}
        currency={currency}
        term={cadence}
        lock={lock}
        context={product}
        newProduct={newProduct}
        familyLocalId={familyLocalId}
        manyLevels={manyLevels}
        name={name}
        lead={lead}
        footer={{ remove, actions }}
      />
    </Form>
  )
}

/** What the product lets the charge become: its fee type and billing. */
export const useChargeLock = (component: PriceComponent) => {
  const product = useComponentProduct(component.productId)
  return useMemo(() => {
    const first = component.prices[0]
    const feeType: ComponentFeeType = first
      ? feeTypeFromPrice(first)
      : product?.feeType !== undefined
        ? protoFeeTypeToComponentFeeType(product.feeType)
        : 'rate'
    const structural: StructuralInfo = product
      ? extractStructuralInfo(feeType, product.feeStructure)
      : {}
    const lock: ChargeLock | undefined = product
      ? {
          feeType,
          billingType: structural.billingType as ChargeLock['billingType'],
        }
      : undefined
    return { feeType, structural, lock }
  }, [component.prices, product])
}

/** What an existing product lets a charge be, and the charge it starts from. */
export const productCharge = (
  item: Pick<ProductWithPrice, 'product' | 'latestPrice'>
): { lock: ChargeLock; product: ChargeBodyContext; initial: StoredCharge } | undefined => {
  const product = item.product
  if (!product || product.feeType === undefined) return undefined
  const feeType = protoFeeTypeToComponentFeeType(product.feeType)
  const structural = extractStructuralInfo(feeType, product.feeStructure)
  return {
    lock: { feeType, billingType: structural.billingType as ChargeLock['billingType'] },
    product: { metricId: structural.metricId, slotUnitName: structural.slotUnitName },
    initial: {
      feeType,
      formData: {
        ...buildDefaults(feeType, item.latestPrice ?? undefined, false, structural.usageModel),
        metricId: structural.metricId,
        slotUnitName: structural.slotUnitName,
        billingType: structural.billingType,
      },
    },
  }
}

/** A new charge: its name, and the existing product it reuses (none: one is made from it). */
export interface NewCharge {
  name: string
  product?: Product
  charge: ChargeFormResult
}

export interface NewChargeFormProps {
  currency: string
  cadence: BillingPeriod
  /** Where reused products and a new usage product's metric are picked from. */
  familyLocalId?: string
  /** Suggests products of every family, not only `familyLocalId`'s (the plan editor's). */
  anyFamily?: boolean
  /** Beside the name: what the charge is priced for ("Every option · EUR"). */
  context: ReactNode
  /** "Included, no charge" is offered: a product with no price, reused or new (plans only). */
  includable?: boolean
  /** "Usage levels" is offered, with several levels (plans only); see ChargeFormProps. */
  manyLevels?: boolean
  /** "Credit pack" is offered, as an add-on sells it. */
  creditPack?: boolean
  /** Kinds not offered here, e.g. seats on an add-on. */
  hiddenKinds?: ChargeKind[]
  submitLabel: string
  pending?: boolean
  onCancel: () => void
  onSubmit: (charge: NewCharge) => void | Promise<unknown>
}

/** "Add a charge": a name, or an existing product of the family to reuse, then its price. */
export const NewChargeForm = ({
  currency,
  cadence,
  familyLocalId,
  anyFamily = false,
  context,
  includable = false,
  manyLevels = false,
  creditPack = false,
  hiddenKinds = [],
  submitLabel,
  pending,
  onCancel,
  onSubmit,
}: NewChargeFormProps) => {
  const [name, setName] = useState('')
  const [picked, setPicked] = useState<ProductWithPrice>()
  const query = useDebounceValue(name.trim(), 250)
  const suggestions = useQuery(
    listProductsWithPrices,
    {
      currency,
      query: query || undefined,
      familyLocalId: anyFamily ? undefined : familyLocalId,
      pagination: { perPage: 5, page: 0 },
    },
    { enabled: !picked && query.length > 1 }
  ).data?.products

  const product = picked?.product
  const shown = (suggestions ?? []).filter(item => item.product)
  // Keyed by product below: computed once per pick.
  const reused = useMemo(() => (picked ? productCharge(picked) : undefined), [picked])
  const chargeName = product?.name ?? name.trim()

  return (
    <>
      <div className="flex items-end gap-3">
        {product ? (
          <div className="min-w-0 flex-1">
            <span className="inline-flex h-9 items-center gap-1.5 rounded-md border border-border bg-muted/40 pl-1 pr-2.5 text-sm font-medium text-foreground">
              <ProductIcon icon={product.icon} color={product.color} size="sm" />
              {product.name}
              <button
                type="button"
                aria-label="Charge for another product"
                className="text-muted-foreground hover:text-foreground"
                onClick={() => setPicked(undefined)}
              >
                <XIcon size={12} />
              </button>
            </span>
          </div>
        ) : (
          <ChargeNameInput
            value={name}
            onChange={setName}
            placeholder="e.g. API calls, Seats"
            autoFocus
          />
        )}
        <ContextChip>{context}</ContextChip>
      </div>

      {!product && shown.length > 0 && (
        <ul aria-label="Reuse a product" className="flex flex-wrap gap-1.5">
          {shown.map(item => (
            <li key={item.product!.id}>
              <button
                type="button"
                onClick={() => setPicked(item)}
                className="inline-flex items-center gap-1.5 rounded-full border border-border py-0.5 pl-0.5 pr-2.5 text-xs text-muted-foreground transition-colors hover:border-muted-foreground/40 hover:text-foreground"
              >
                <ProductIcon
                  icon={item.product!.icon}
                  color={item.product!.color}
                  size="sm"
                  className="rounded-full"
                />
                Reuse “{item.product!.name}”
                {item.product!.feeType !== undefined && (
                  <span className="text-muted-foreground/70">
                    · {feeTypeToHuman(protoFeeTypeToComponentFeeType(item.product!.feeType))}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}

      {product || name.trim() ? (
        <ChargeForm
          key={product?.id ?? 'new'}
          currency={currency}
          cadence={cadence}
          name={chargeName}
          lock={reused?.lock}
          product={reused?.product}
          newProduct={!product}
          initial={
            reused && {
              ...reused.initial,
              formData: { ...reused.initial.formData, term: termFormValue(cadence) },
            }
          }
          familyLocalId={familyLocalId}
          includable={includable}
          manyLevels={manyLevels}
          creditPack={creditPack}
          hiddenKinds={hiddenKinds}
          notice={
            product && (
              <p className="text-xs text-muted-foreground">Reuses the product and its features.</p>
            )
          }
          submitLabel={submitLabel}
          pending={pending}
          onCancel={onCancel}
          onSubmit={charge => onSubmit({ name: chargeName, product, charge })}
        />
      ) : (
        // The form's frame before it is named, so naming it moves nothing.
        <ChargeLayout
          preview={<PreviewNote />}
          footer={
            <FormFooter>
              <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
                Cancel
              </Button>
            </FormFooter>
          }
        >
          <p className="text-xs text-muted-foreground">Name it, or reuse a product, to price it.</p>
        </ChargeLayout>
      )}
    </>
  )
}
