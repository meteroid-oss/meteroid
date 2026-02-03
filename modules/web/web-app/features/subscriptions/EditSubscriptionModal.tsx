import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  DialogDescription,
  DialogTitle,
  Form,
  InputFormField,
  Modal,
  SwitchFormField,
  TextareaFormField,
} from '@md/ui'
import { Settings } from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import {
  getSubscriptionDetails,
  updateSubscription,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'

const editSubscriptionSchema = z.object({
  chargeAutomatically: z.boolean(),
  autoAdvanceInvoices: z.boolean(),
  netTerms: z.coerce.number().int().min(0).max(365),
  invoiceMemo: z.string().max(500).optional(),
  purchaseOrder: z.string().max(100).optional(),
})

type EditSubscriptionFormValues = z.infer<typeof editSubscriptionSchema>

interface EditSubscriptionModalProps {
  subscription: Subscription
  onClose: () => void
  onSuccess?: () => void
}

export const EditSubscriptionModal = ({
  subscription,
  onClose,
  onSuccess,
}: EditSubscriptionModalProps) => {
  const queryClient = useQueryClient()

  const hasPaymentMethod = Boolean(
    subscription.cardConnectionId || subscription.directDebitConnectionId
  )

  const methods = useZodForm({
    schema: editSubscriptionSchema,
    defaultValues: {
      chargeAutomatically: subscription.chargeAutomatically,
      autoAdvanceInvoices: subscription.autoAdvanceInvoices,
      netTerms: subscription.netTerms,
      invoiceMemo: subscription.invoiceMemo ?? '',
      purchaseOrder: subscription.purchaseOrder ?? '',
    },
    mode: 'onSubmit',
  })

  const updateMutation = useMutation(updateSubscription, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getSubscriptionDetails, {
          subscriptionId: subscription.id,
        }),
      })
      toast.success('Subscription updated')
      onSuccess?.()
      onClose()
    },
    onError: error => {
      toast.error(
        `Failed to update subscription: ${error instanceof Error ? error.message : 'Unknown error'}`
      )
    },
  })

  const onSubmit = async (values: EditSubscriptionFormValues) => {
    if (values.chargeAutomatically && !hasPaymentMethod) {
      methods.setError('chargeAutomatically', {
        message: 'Cannot enable automatic charging without a payment method',
      })
      return
    }

    await updateMutation.mutateAsync({
      subscriptionId: subscription.id,
      chargeAutomatically: values.chargeAutomatically,
      autoAdvanceInvoices: values.autoAdvanceInvoices,
      netTerms: values.netTerms,
      invoiceMemo: values.invoiceMemo || undefined,
      purchaseOrder: values.purchaseOrder || undefined,
    })
  }

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <Settings className="w-5 h-5 text-muted-foreground" />
            <span>Edit Billing Settings</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Update billing configuration for this subscription.
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={false}
      onCancel={onClose}
      onConfirm={methods.handleSubmit(onSubmit)}
      confirmText={updateMutation.isPending ? 'Saving...' : 'Save Changes'}
      confirmDisabled={updateMutation.isPending}
    >
      <Modal.Content>
        <Form {...methods}>
          <div className="space-y-5 py-4">
            <div className="space-y-2">
              <SwitchFormField
                name="chargeAutomatically"
                control={methods.control}
                label="Charge automatically"
                description="Automatically charge invoices using the customer's payment method"
                disabled={!hasPaymentMethod && !methods.getValues('chargeAutomatically')}
              />
              {!hasPaymentMethod && (
                <p className="text-xs text-warning ml-10">
                  No payment method configured. Add a payment method to enable automatic charging.
                </p>
              )}

              <SwitchFormField
                name="autoAdvanceInvoices"
                control={methods.control}
                label="Auto-advance invoices"
                description="Automatically finalize draft invoices"
              />
            </div>

            <InputFormField
              name="netTerms"
              control={methods.control}
              label="Net Terms (days)"
              description="Number of days until invoice is due (0 = due on issue)"
              type="number"
              min={0}
              max={365}
              placeholder="0"
            />

            <TextareaFormField
              name="invoiceMemo"
              control={methods.control}
              label="Invoice Memo"
              placeholder="Optional memo to include on invoices..."
              rows={2}
              maxLength={500}
            />

            <InputFormField
              name="purchaseOrder"
              control={methods.control}
              label="Purchase Order"
              placeholder="PO-12345"
              maxLength={100}
            />
          </div>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
