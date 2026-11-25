import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@md/ui'

import { usePlanOverview } from '@/features/plans/hooks/usePlan'
import {
  buildExistingProductRef,
  buildFeeStructure,
  buildNewProductRef,
  buildPriceInputs,
} from '@/features/pricing/conversions'

import { NewChargeForm } from './ChargeForm'

import type { NewCharge } from './ChargeForm'
import type { ChargeMutations } from './useChargeMutations'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

type Props = {
  currency: string
  /** The cadence a recurring fee starts on. */
  term: BillingPeriod
  /** The plan has several options: a new charge's price goes into each. */
  everyOption: boolean
  mutations: ChargeMutations
  onClose: () => void
  /** Saved: the charge made, for the section to bring into view. */
  onAdded: (componentId: string | undefined) => void
}

/**
 * "Add a product", over the plan wherever it is scrolled: what it charges for (a new product or
 * one reused), then how. The form subscriptions and quotes add a fee with, in the same dialog.
 */
export const AddProductDialog = ({
  currency,
  term,
  everyOption,
  mutations,
  onClose,
  onAdded,
}: Props) => {
  const familyLocalId = usePlanOverview()?.productFamilyLocalId

  const add = async ({ name, product, charge }: NewCharge) => {
    const ref = product ? buildExistingProductRef(product.id) : buildNewProductRef(name)
    if (charge.included) {
      // Included with no price: neither the product nor the charge bills anything.
      const added = await mutations.create(name, ref, [])
      return onAdded(added.component?.id)
    }
    const inputs = buildPriceInputs(charge.pricingType, charge.formData, currency)
    const added = await mutations.create(
      name,
      ref,
      inputs,
      buildFeeStructure(charge.feeType, charge.formData)
    )
    onAdded(added.component?.id)
  }

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-h-[90vh] max-w-[min(66rem,calc(100vw-2rem))] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add a product</DialogTitle>
          <DialogDescription>
            Type a name: reuse one of the products it finds, or price a new one.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <NewChargeForm
            currency={currency}
            cadence={term}
            familyLocalId={familyLocalId}
            anyFamily
            context={everyOption ? `Every option · ${currency}` : currency}
            includable
            manyLevels
            submitLabel="Add product"
            pending={mutations.pending}
            onCancel={onClose}
            onSubmit={add}
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}
