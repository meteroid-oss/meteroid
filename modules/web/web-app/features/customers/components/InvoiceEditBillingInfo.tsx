import { PartialMessage } from '@bufbuild/protobuf'
import { Alert, Button } from '@md/ui'
import { InfoIcon, RefreshCw } from 'lucide-react'
import { UseFormReturn } from 'react-hook-form'

import { Customer } from '@/rpc/api/customers/v1/models_pb'
import { InlineCustomer } from '@/rpc/api/invoices/v1/models_pb'

import { BillingInfoCard } from './BillingInfoCard'
import { BillingInfoForm, BillingInfoFormValues } from './BillingInfoForm'

interface InvoiceEditBillingInfoProps {
  customerDetails: InlineCustomer
  isEditing: boolean
  setIsEditing: (isEditing: boolean) => void
  methods: UseFormReturn<BillingInfoFormValues>
  onSubmit: (values: BillingInfoFormValues) => void
  onRefreshFromCustomer: () => void
  isSubmitting?: boolean
}

export const InvoiceEditBillingInfo = ({
  customerDetails,
  isEditing,
  setIsEditing,
  methods,
  onSubmit,
  onRefreshFromCustomer,
  isSubmitting,
}: InvoiceEditBillingInfoProps) => {
  // Convert InlineCustomer to Customer format for reuse
  const customerForCard: PartialMessage<Customer> = {
    name: customerDetails.name,
    billingEmail: customerDetails.email || undefined,
    billingAddress: customerDetails.billingAddress,
    vatNumber: customerDetails.vatNumber || undefined,
  }

  // Helper to reset form to original customer details
  const resetToOriginal = () => {
    methods.reset({
      name: customerDetails.name || '',
      billingEmail: customerDetails.email || '',
      line1: customerDetails.billingAddress?.line1 || '',
      line2: customerDetails.billingAddress?.line2 || '',
      city: customerDetails.billingAddress?.city || '',
      zipCode: customerDetails.billingAddress?.zipCode || '',
      country: customerDetails.billingAddress?.country || undefined,
      vatNumber: customerDetails.vatNumber || '',
    })
  }

  if (!isEditing) {
    return (
      <BillingInfoCard
        customer={customerForCard}
        onEdit={() => setIsEditing(true)}
        title="Customer details"
        cardVariant="default"
        actions={
          <Button
            variant="ghost"
            size="sm"
            className="  p-0 h-auto"
            onClick={onRefreshFromCustomer}
            type="button"
            title="Refresh from customer record"
          >
            <RefreshCw size={16} />
          </Button>
        }
      />
    )
  }

  // In edit mode, show the form with hideActions=true
  // This hides Save/Cancel buttons since parent form handles submission
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between mb-2">
        <div className="text-sm font-medium">Customer details</div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => {
            resetToOriginal()
            setIsEditing(false)
          }}
        >
          Cancel
        </Button>
      </div>

      <Alert variant="default" className="text-xs text-muted-foreground">
        <span className="flex gap-1">
          <InfoIcon size={14} />
          Changes to the customer details only apply to this invoice.
        </span>
      </Alert>

      <BillingInfoForm
        customer={customerForCard}
        methods={methods}
        onSubmit={async data => onSubmit(data)}
        onBlur={async data => onSubmit(data)}
        onCancel={() => {
          resetToOriginal()
          setIsEditing(false)
        }}
        isSubmitting={isSubmitting}
        title="" // No title since we show it above
        hideActions={true} // Hide Save/Cancel buttons - parent form handles it
      />
    </div>
  )
}
