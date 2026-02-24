import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  DialogDescription,
  DialogTitle,
  Form,
  InputFormField,
  Modal,
  SelectFormField,
  SelectItem,
  SwitchFormField,
  TextareaFormField,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { InfoIcon, Settings } from 'lucide-react'
import { useEffect } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import {
  BankTransfer,
  External,
  OnlinePayment,
  PaymentMethodsConfig,
  Subscription,
} from '@/rpc/api/subscriptions/v1/models_pb'
import {
  getSubscriptionDetails,
  updateSubscription,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

type PaymentMethodsType = 'online' | 'bankTransfer' | 'external'

const editSubscriptionSchema = z.object({
  chargeAutomatically: z.boolean(),
  autoAdvanceInvoices: z.boolean(),
  netTerms: z.coerce.number().int().min(0).max(365),
  invoiceMemo: z.string().max(500).optional(),
  purchaseOrder: z.string().max(100).optional(),
  paymentMethodsType: z.enum(['online', 'bankTransfer', 'external']),
})

type EditSubscriptionFormValues = z.infer<typeof editSubscriptionSchema>

const getPaymentMethodsTypeFromProto = (config?: PaymentMethodsConfig): PaymentMethodsType => {
  if (!config) return 'online'
  switch (config.config.case) {
    case 'online':
      return 'online'
    case 'bankTransfer':
      return 'bankTransfer'
    case 'external':
      return 'external'
    default:
      return 'online'
  }
}

const buildProtoPaymentMethodsConfig = (type: PaymentMethodsType): PaymentMethodsConfig => {
  switch (type) {
    case 'online':
      return new PaymentMethodsConfig({ config: { case: 'online', value: new OnlinePayment() } })
    case 'bankTransfer':
      return new PaymentMethodsConfig({
        config: { case: 'bankTransfer', value: new BankTransfer() },
      })
    case 'external':
      return new PaymentMethodsConfig({ config: { case: 'external', value: new External() } })
  }
}

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

  const methods = useZodForm({
    schema: editSubscriptionSchema,
    defaultValues: {
      chargeAutomatically: subscription.chargeAutomatically,
      autoAdvanceInvoices: subscription.autoAdvanceInvoices,
      netTerms: subscription.netTerms,
      invoiceMemo: subscription.invoiceMemo ?? '',
      purchaseOrder: subscription.purchaseOrder ?? '',
      paymentMethodsType: getPaymentMethodsTypeFromProto(subscription.paymentMethodsConfig),
    },
    mode: 'onSubmit',
  })

  // Watch payment methods type and charge automatically for cross-validation
  const [paymentMethodsType, chargeAutomatically] = methods.watch([
    'paymentMethodsType',
    'chargeAutomatically',
  ])

  // Auto-disable chargeAutomatically when Bank or External payment methods selected
  useEffect(() => {
    if (
      (paymentMethodsType === 'bankTransfer' || paymentMethodsType === 'external') &&
      chargeAutomatically
    ) {
      methods.setValue('chargeAutomatically', false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paymentMethodsType, chargeAutomatically])

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
    await updateMutation.mutateAsync({
      subscriptionId: subscription.id,
      chargeAutomatically: values.chargeAutomatically,
      autoAdvanceInvoices: values.autoAdvanceInvoices,
      netTerms: values.netTerms,
      invoiceMemo: values.invoiceMemo || undefined,
      purchaseOrder: values.purchaseOrder || undefined,
      paymentMethodsConfig: buildProtoPaymentMethodsConfig(values.paymentMethodsType),
    })
  }

  // Can only enable charge automatically with online payment methods
  const canChargeAutomatically = paymentMethodsType === 'online'

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
            <SelectFormField
              name="paymentMethodsType"
              label="Payment Methods"
              control={methods.control}
              labelTooltip={
                <TooltipProvider delayDuration={100}>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                    </TooltipTrigger>
                    <TooltipContent className="max-w-80">
                      Online: Use card and/or direct debit payments.
                      <br />
                      Bank Transfer: Invoice with bank transfer instructions.
                      <br />
                      External: Manage payment collection outside the system.
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              }
            >
              <SelectItem value="online">Online (card / direct debit)</SelectItem>
              <SelectItem value="bankTransfer">Bank transfer</SelectItem>
              <SelectItem value="external">External</SelectItem>
            </SelectFormField>

            <div className="space-y-2 pt-2 border-t">
              <SwitchFormField
                name="chargeAutomatically"
                control={methods.control}
                label="Charge automatically"
                description={
                  canChargeAutomatically
                    ? 'Automatically charge invoices when the customer has a payment method configured'
                    : 'Only available with Online payment methods'
                }
                disabled={!canChargeAutomatically}
              />

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
