import { skipToken } from '@connectrpc/connect-query'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@md/ui'
import { useState, type ReactNode } from 'react'

import { ChargeForm, NewChargeForm, useChargeLock } from '@/features/plans/charges/ChargeForm'
import { ChargeNameInput, ContextChip } from '@/features/plans/charges/InlineChargeFields'
import { hasTerm, termFormValue } from '@/features/plans/currencies/priceCells'
import { useComponentMetric } from '@/features/plans/pricecomponents/componentRefs'
import { buildDefaults } from '@/features/pricing/formData'
import { useQuery } from '@/lib/connectrpc'
import {
  getPlanOverview,
  getPlanWithVersionByVersionId,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import type { ChargeFormResult, StoredCharge } from '@/features/plans/charges/ChargeForm'
import type { ChargeBodyContext } from '@/features/plans/charges/InlineChargeFields'
import type { ChargeLock } from '@/features/plans/charges/compile'
import type { ComponentOverride } from '@/pages/tenants/subscription/create/state'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

// The plan editor's charge form for what a subscription, a quote or an amendment prices on its
// own: a custom price for one of its charges, a fee on top of the plan. Each form renders in
// place (a list row opened up) or in a dialog.

/** A fee added on top of the plan: the extra component of a subscription, quote or amendment. */
export interface FeeDraft extends StoredCharge {
  name: string
  /** The product reused; none: one is made by name (buildNewProductRef). The fee owns its shape. */
  productId?: string
}

/** The product family of a plan version: where a new fee's products and metrics come from. */
export const usePlanVersionFamily = (planVersionId: string | undefined) => {
  const plan = useQuery(
    getPlanWithVersionByVersionId,
    planVersionId ? { localId: planVersionId } : skipToken
  ).data?.plan?.plan
  return useQuery(getPlanOverview, plan?.localId ? { localId: plan.localId } : skipToken).data
    ?.planOverview?.productFamilyLocalId
}

/** The cadence a new fee starts on: the plan's charges' own, else monthly. */
export const salesCadence = (components: PriceComponent[]): BillingPeriod =>
  components.flatMap(c => c.prices).find(hasTerm)?.cadence ?? BillingPeriod.MONTHLY

const priced = (result: ChargeFormResult): StoredCharge | undefined =>
  result.included ? undefined : { feeType: result.feeType, formData: result.formData }

/** The name a custom price is for, fixed, and what it is priced in. */
const FixedName = ({ name, chip }: { name: string; chip: string }) => (
  <div className="flex items-end gap-3">
    <span className="min-w-0 flex-1 truncate pb-1 text-base font-semibold text-foreground">
      {name}
    </span>
    <ContextChip>{chip}</ContextChip>
  </div>
)

/** A form of this file in place, as the plan editor opens a charge in its list. */
export const InlinePanel = ({ children }: { children: ReactNode }) => (
  <div className="space-y-4 bg-muted/30 px-4 py-4">{children}</div>
)

const ChargeDialog = ({
  title,
  description,
  onClose,
  children,
}: {
  title: string
  description: string
  onClose: () => void
  children: ReactNode
}) => (
  <Dialog open onOpenChange={open => !open && onClose()}>
    <DialogContent className="max-h-[90vh] max-w-[min(66rem,calc(100vw-2rem))] overflow-y-auto">
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription>{description}</DialogDescription>
      </DialogHeader>
      <div className="space-y-4">{children}</div>
    </DialogContent>
  </Dialog>
)

// --- A custom price: the charge's kind stays its own ---

interface CustomPriceProps {
  name: string
  currency: string
  cadence: BillingPeriod
  lock: ChargeLock
  product: ChargeBodyContext
  initial: StoredCharge
  onClose: () => void
  onSave: (charge: StoredCharge) => void
}

export const CustomPriceForm = ({
  name,
  currency,
  cadence,
  lock,
  product,
  initial,
  onClose,
  onSave,
}: CustomPriceProps) => (
  <>
    <FixedName name={name} chip={`Custom price · ${currency}`} />
    <ChargeForm
      currency={currency}
      cadence={cadence}
      name={name}
      lock={lock}
      product={product}
      newProduct={false}
      initial={initial}
      submitLabel="Save price"
      onCancel={onClose}
      onSubmit={result => {
        const charge = priced(result)
        if (charge) onSave(charge)
      }}
    />
  </>
)

export const CustomPriceDialog = (props: CustomPriceProps) => (
  <ChargeDialog
    title="Custom price"
    description="Priced for this subscription only; the plan keeps its own price."
    onClose={props.onClose}
  >
    <CustomPriceForm {...props} />
  </ChargeDialog>
)

// --- "Override price" of a plan charge ---

interface PlanChargePriceProps {
  /** The charge as bought: its prices are the option's, in the currency. */
  component: PriceComponent
  /** The plan's price shown for it (the capacity level chosen). */
  price?: Price
  /** Its custom price so far: the form opens on it. */
  override?: ComponentOverride
  currency: string
  onClose: () => void
  onSave: (override: ComponentOverride) => void
}

export const PlanChargePriceForm = ({
  component,
  price,
  override,
  currency,
  onClose,
  onSave,
}: PlanChargePriceProps) => {
  const { feeType, structural, lock } = useChargeLock(component)
  const metric = useComponentMetric(structural.metricId)
  const name = override?.name || component.name
  const initial: StoredCharge = override
    ? { feeType: override.feeType, formData: override.formData }
    : {
        feeType,
        formData: {
          ...buildDefaults(feeType, price),
          metricId: structural.metricId,
          slotUnitName: structural.slotUnitName,
          quantityId: structural.quantityId,
          billingType: structural.billingType,
        },
      }
  return (
    <CustomPriceForm
      name={name}
      currency={currency}
      cadence={price && hasTerm(price) ? price.cadence : BillingPeriod.MONTHLY}
      lock={lock ?? { feeType }}
      product={{ metric, metricId: structural.metricId, slotUnitName: structural.slotUnitName }}
      initial={initial}
      onClose={onClose}
      onSave={charge =>
        onSave({ componentId: component.id, name, productId: component.productId, ...charge })
      }
    />
  )
}

export const PlanChargePriceDialog = (props: PlanChargePriceProps) => (
  <ChargeDialog
    title="Custom price"
    description="Priced for this customer only; the plan keeps its own price."
    onClose={props.onClose}
  >
    <PlanChargePriceForm {...props} />
  </ChargeDialog>
)

// --- A fee on top of the plan ---

interface FeeProps<F extends FeeDraft> {
  /** The fee edited; none: a new one. */
  fee?: F
  /** The edited fee's name may change; a quote keys its lines by it, so keeps it. */
  renamable?: boolean
  currency: string
  cadence: BillingPeriod
  familyLocalId?: string
  onClose: () => void
  onSave: (fee: FeeDraft | F) => void
}

/**
 * "Add a fee": an existing product of the family, reused, or a new charge by name; either way the
 * fee bills it any way. With `fee`, that fee's name and price.
 */
export const FeeForm = <F extends FeeDraft>({
  fee,
  renamable = true,
  currency,
  cadence,
  familyLocalId,
  onClose,
  onSave,
}: FeeProps<F>) =>
  fee ? (
    <EditFee
      fee={fee}
      renamable={renamable}
      currency={currency}
      cadence={cadence}
      familyLocalId={familyLocalId}
      onClose={onClose}
      onSave={onSave}
    />
  ) : (
    <NewChargeForm
      currency={currency}
      cadence={cadence}
      familyLocalId={familyLocalId}
      context={currency}
      submitLabel="Add fee"
      onCancel={onClose}
      onSubmit={({ name, product, charge }) => {
        const stored = priced(charge)
        if (stored && name) onSave({ name, productId: product?.id, ...stored })
      }}
    />
  )

export const FeeDialog = <F extends FeeDraft>(props: FeeProps<F>) => (
  <ChargeDialog
    title={props.fee ? 'Edit fee' : 'Add a fee'}
    description={
      props.fee
        ? 'A fee added on top of the plan.'
        : 'Type a name: reuse one of the products it finds, or price a new charge.'
    }
    onClose={props.onClose}
  >
    <FeeForm {...props} />
  </ChargeDialog>
)

const EditFee = <F extends FeeDraft>({
  fee,
  renamable,
  currency,
  cadence,
  familyLocalId,
  onClose,
  onSave,
}: {
  fee: F
  renamable: boolean
  currency: string
  cadence: BillingPeriod
  familyLocalId?: string
  onClose: () => void
  onSave: (fee: F) => void
}) => {
  const [name, setName] = useState(fee.name)
  const metricId = fee.formData.metricId as string | undefined
  const metric = useComponentMetric(metricId)

  return (
    <>
      {renamable ? (
        <div className="flex items-end gap-3">
          <ChargeNameInput value={name} onChange={setName} autoFocus />
          <ContextChip>{currency}</ContextChip>
        </div>
      ) : (
        <FixedName name={fee.name} chip={currency} />
      )}
      <ChargeForm
        currency={currency}
        cadence={cadence}
        name={name.trim() || fee.name}
        product={{
          metric,
          metricId,
          slotUnitName: fee.formData.slotUnitName as string | undefined,
        }}
        initial={{
          feeType: fee.feeType,
          formData: { term: termFormValue(cadence), ...fee.formData },
        }}
        familyLocalId={familyLocalId}
        submitLabel="Save fee"
        onCancel={onClose}
        onSubmit={result => {
          const charge = priced(result)
          if (charge) onSave({ ...fee, name: name.trim() || fee.name, ...charge })
        }}
      />
    </>
  )
}
