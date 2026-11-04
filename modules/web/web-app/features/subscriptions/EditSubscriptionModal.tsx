import { create } from '@bufbuild/protobuf';
import { ConnectError } from '@connectrpc/connect'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Alert,
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

import {
  CustomPropertiesFields,
  parseCustomProperties,
  serializeCustomProperties,
} from '@/features/customproperties/CustomPropertiesFields'
import {
  UsageFilterFields,
  formatUsageFilter,
  refineUsageFilter,
  USAGE_FILTER_CHANGE_WARNING,
  usageFilterFormFields,
  usageFilterFromForm,
  usageFilterToForm,
} from '@/features/subscriptions/UsageFilterFields'
import { useZodForm } from '@/hooks/useZodForm'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import {
  BankTransferSchema,
  ExternalSchema,
  OnlinePaymentSchema,
  PaymentMethodsConfigSchema,
  Subscription,
} from '@/rpc/api/subscriptions/v1/models_pb';
import {
  getSubscriptionDetails,
  listSubscriptions,
  listUnattributedUsage,
  updateSubscription,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { PaymentMethodsConfig } from '@/rpc/api/subscriptions/v1/models_pb';


type PaymentMethodsType = 'online' | 'bankTransfer' | 'external'

const editSubscriptionSchema = z
  .object({
    chargeAutomatically: z.boolean(),
    autoAdvanceInvoices: z.boolean(),
    netTerms: z.coerce.number().int().min(0).max(365),
    invoiceMemo: z.string().max(500).optional(),
    purchaseOrder: z.string().max(100).optional(),
    paymentMethodsType: z.enum(['online', 'bankTransfer', 'external']),
    customProperties: z.record(z.string(), z.unknown()).optional(),
    ...usageFilterFormFields,
  })
  .superRefine(refineUsageFilter)

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
      return create(
        PaymentMethodsConfigSchema,
        { config: { case: 'online', value: create(OnlinePaymentSchema) } }
      );
    case 'bankTransfer':
      return create(PaymentMethodsConfigSchema, {
        config: { case: 'bankTransfer', value: create(BankTransferSchema) },
      });
    case 'external':
      return create(
        PaymentMethodsConfigSchema,
        { config: { case: 'external', value: create(ExternalSchema) } }
      );
  }
}

interface EditSubscriptionModalProps {
  subscription: Subscription
  /** A usage filter means something only on a subscription that has not ended and bills usage. */
  usageFilterEditable: boolean
  onClose: () => void
  onSuccess?: () => void
}

// The server's refusals of a usage filter, shown on the field rather than as a toast.
const USAGE_FILTER_ERROR = /usage filter|already bills the events where/i

export const EditSubscriptionModal = ({
  subscription,
  usageFilterEditable,
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
      customProperties: parseCustomProperties(subscription.customProperties),
      ...usageFilterToForm(subscription.usageFilter),
    },
    mode: 'onSubmit',
  })

  // Watch payment methods type and charge automatically for cross-validation
  const [paymentMethodsType, chargeAutomatically, usageFilterProperty, usageFilterValue] =
    methods.watch([
      'paymentMethodsType',
      'chargeAutomatically',
      'usageFilterProperty',
      'usageFilterValue',
    ])
  const usageFilterChanged = (property?: string, value?: string) =>
    (property ?? '') !== (subscription.usageFilter?.property ?? '') ||
    (value ?? '') !== (subscription.usageFilter?.value ?? '')

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
        queryKey: createConnectQueryKey({
          schema: getSubscriptionDetails,

          input: {
            subscriptionId: subscription.id,
          },

          cardinality: 'finite'
        }),
      })
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listSubscriptions, cardinality: undefined }),
      })
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listUnattributedUsage, cardinality: undefined }),
      })
      toast.success('Subscription updated')
      onSuccess?.()
      onClose()
    },
    onError: error => {
      const message = ConnectError.from(error).rawMessage
      if (usageFilterEditable && USAGE_FILTER_ERROR.test(message)) {
        methods.setError('usageFilterValue', { type: 'server', message })
        return
      }
      toast.error(`Failed to update subscription: ${message || 'Unknown error'}`)
    },
  })

  const onSubmit = async (values: EditSubscriptionFormValues) => {
    // Sent only when changed: the server applies it from the first usage nobody has invoiced.
    const usageFilter =
      usageFilterEditable && usageFilterChanged(values.usageFilterProperty, values.usageFilterValue)
        ? { filter: usageFilterFromForm(values) }
        : undefined
    await updateMutation.mutateAsync({
      subscriptionId: subscription.id,
      chargeAutomatically: values.chargeAutomatically,
      autoAdvanceInvoices: values.autoAdvanceInvoices,
      netTerms: values.netTerms,
      invoiceMemo: values.invoiceMemo || undefined,
      purchaseOrder: values.purchaseOrder || undefined,
      paymentMethodsConfig: buildProtoPaymentMethodsConfig(values.paymentMethodsType),
      customProperties: serializeCustomProperties(values.customProperties),
      usageFilter,
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

            <CustomPropertiesFields
              control={methods.control}
              entityType={CustomPropertyEntityType.SUBSCRIPTION}
            />

            {usageFilterEditable && (
              <div className="space-y-3 pt-2 border-t">
                <h4 className="text-sm font-medium">Usage filter</h4>
                <UsageFilterFields control={methods.control} />
                {usageFilterChanged(usageFilterProperty, usageFilterValue) && (
                  <Alert variant="warning" className="text-xs">
                    {USAGE_FILTER_CHANGE_WARNING}
                  </Alert>
                )}
              </div>
            )}
            {!usageFilterEditable && subscription.usageFilter && (
              <div className="space-y-1 pt-2 border-t">
                <h4 className="text-sm font-medium">Usage filter</h4>
                <p className="text-sm text-foreground">
                  {formatUsageFilter(subscription.usageFilter)}
                </p>
                <p className="text-xs text-muted-foreground">
                  Applies only while the subscription bills usage, so it cannot be changed here.
                </p>
              </div>
            )}
          </div>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
