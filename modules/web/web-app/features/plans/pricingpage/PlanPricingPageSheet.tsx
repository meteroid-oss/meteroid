import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from '@md/ui'

import { PlanPricingPageForm } from '@/features/plans/pricingpage/PlanPricingPageForm'

export const PlanPricingPageSheet = ({
  planId,
  planName,
  planVersionId,
  onClose,
}: {
  planId: string
  planName: string
  planVersionId?: string
  onClose: () => void
}) => (
  <Sheet open onOpenChange={open => !open && onClose()}>
    <SheetContent size="medium" className="flex flex-col">
      <SheetHeader>
        <SheetTitle>{planName} on the pricing page</SheetTitle>
        <SheetDescription>
          Marketing copy, not billing: editing it never makes a new plan version.
        </SheetDescription>
      </SheetHeader>
      <PlanPricingPageForm
        planId={planId}
        planVersionId={planVersionId}
        onCancel={onClose}
        onSaved={onClose}
        className="flex-1 overflow-y-auto py-4"
        actionsClassName="mt-auto border-t border-border pt-4"
      />
    </SheetContent>
  </Sheet>
)
