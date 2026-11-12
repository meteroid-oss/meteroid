import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query';
import {
  Badge,
  Button,
  Form,
  Select,
  SelectContent,
  SelectEmpty,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { InlineLoading } from '@/components/Loading'
import { CredentialsErrorBadge, railStatus } from '@/features/payments/connectors'
import { PaymentRail, paymentProvider } from '@/features/payments/providers'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { BankAccountsCard } from '@/features/settings/components/bankaccounts'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listBankAccounts } from '@/rpc/api/bankaccounts/v1/bankaccounts-BankAccountsService_connectquery'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import {
  Connector,
  ConnectorStatusEnum,
  ConnectorTypeEnum,
} from '@/rpc/api/connectors/v1/models_pb'
import {
  getInvoicingEntityProviders,
  updateInvoicingEntityProviders,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

const paymentMethodsSchema = z.object({
  cardProviderId: z.string().optional(),
  directDebitProviderId: z.string().optional(),
  bankAccountId: z.string().optional(),
})

interface ProviderOption {
  connector: Connector
  /** Why it can't be picked; absent when selectable. */
  unavailable?: string
}

/**
 * Selectable: active connections whose account has the rail enabled. Disabled connections and
 * rails the account hasn't enabled are listed greyed out with the reason; drafts and rails the
 * provider can't do at all are left out. The backend validates routing too.
 */
const providerOptions = (connectors: Connector[], rail: PaymentRail): ProviderOption[] =>
  connectors.flatMap<ProviderOption>(connector => {
    if (connector.status === ConnectorStatusEnum.DRAFT) return []
    const status = railStatus(connector, rail)
    if (!status.supported) return []
    if (connector.status === ConnectorStatusEnum.DISABLED) {
      return [{ connector, unavailable: 'Disabled' }]
    }
    return [{ connector, unavailable: status.enabled ? undefined : status.reason }]
  })

const RAIL_NOUNS: Record<PaymentRail, string> = {
  card: "cards aren't",
  directDebit: "direct debit isn't",
}

const ProviderIcon = ({ connector }: { connector: Connector }) => {
  const Logo = paymentProvider(connector.provider)?.Logo
  return Logo ? <Logo className="w-3 h-3" /> : <>P</>
}

interface PaymentMethodsFormProps {
  invoiceEntityId: string
  providersData: { cardProvider?: { id: string }; directDebitProvider?: { id: string }; bankAccount?: { id: string } } | undefined
  paymentProviders: Connector[]
  bankAccounts: { id: string; name: string; currency: string; displayName: string }[]
}

const PaymentMethodsForm = ({
  invoiceEntityId,
  providersData,
  paymentProviders,
  bankAccounts,
}: PaymentMethodsFormProps) => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const updateInvoicingEntityMut = useMutation(updateInvoicingEntityProviders, {
    onSuccess: async () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getInvoicingEntityProviders,
          cardinality: undefined
        })
      })
      toast.success('Payment methods updated')
    },
  })

  const methods = useZodForm({
    schema: paymentMethodsSchema,
    defaultValues: {
      cardProviderId: providersData?.cardProvider?.id || 'none',
      directDebitProviderId: providersData?.directDebitProvider?.id || 'none',
      bankAccountId: providersData?.bankAccount?.id || 'none',
    },
  })

  const cardOptions = providerOptions(paymentProviders, 'card')
  const directDebitOptions = providerOptions(paymentProviders, 'directDebit')

  const onSubmit = async (values: z.infer<typeof paymentMethodsSchema>) => {
    await updateInvoicingEntityMut.mutateAsync({
      id: invoiceEntityId,
      cardProviderId: values.cardProviderId === 'none' ? undefined : values.cardProviderId,
      directDebitProviderId:
        values.directDebitProviderId === 'none' ? undefined : values.directDebitProviderId,
      bankAccountId: values.bankAccountId === 'none' ? undefined : values.bankAccountId,
    })
    // Reset to the saved values so later changes mark the form dirty again.
    methods.reset(values)
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <SettingsSection
          divided={false}
          title="Payment method routing"
          description={
            <>
              Customers linked to this invoicing entity will inherit from this configuration.{' '}
              <Button
                type="button"
                variant="link"
                size="sm"
                className="h-fit px-0"
                onClick={() => navigate('../integrations', { replace: true })}
              >
                Configure your payment integrations →
              </Button>
            </>
          }
          action={
            <div className="flex items-center gap-2">
              <InvoicingEntitySelect />
              <Button
                size="sm"
                disabled={!methods.formState.isDirty || updateInvoicingEntityMut.isPending}
              >
                Save changes
              </Button>
            </div>
          }
        >
          <div className="overflow-hidden rounded-lg border border-border text-sm">
            <table className="w-full">
              <thead>
                <tr className="border-b">
                  <th className="text-left py-2 px-3 font-medium text-muted-foreground">Method</th>
                  <th className="text-left py-2 px-3 font-medium text-muted-foreground">
                    Provided by
                  </th>
                </tr>
              </thead>
              <tbody>
                <tr className="border-b">
                  <td className="py-4 px-3 flex items-center">
                    <span className="mr-2 inline-block">💳</span>
                    <span className="mr-2">Credit card</span>
                  </td>
                  <td className="py-4 px-3">
                    <ProviderSelect
                      rail="card"
                      options={cardOptions}
                      value={methods.watch('cardProviderId')}
                      onChange={value =>
                        methods.setValue('cardProviderId', value, { shouldDirty: true })
                      }
                    />
                  </td>
                </tr>
                <tr className="border-b">
                  <td className="py-4 px-3 flex items-center">
                    <span className="mr-2 inline-block">⬇️</span>
                    <span className="mr-2">Direct Debit (SEPA, BACS, ACH)</span>
                  </td>
                  <td className="py-4 px-3">
                    <ProviderSelect
                      rail="directDebit"
                      options={directDebitOptions}
                      value={methods.watch('directDebitProviderId')}
                      onChange={value =>
                        methods.setValue('directDebitProviderId', value, { shouldDirty: true })
                      }
                    />
                  </td>
                </tr>

                <tr className="border-b">
                  <td className="py-4 px-3 flex items-center">
                    <span className="mr-2 inline-block">🏦</span>
                    <span className="">Bank transfer</span>
                  </td>
                  <td className="py-4 px-3">
                    <Select
                      value={methods.watch('bankAccountId')}
                      onValueChange={value =>
                        methods.setValue('bankAccountId', value, { shouldDirty: true })
                      }
                    >
                      <SelectTrigger className="w-60">
                        <SelectValue placeholder="Select a bank account" />
                      </SelectTrigger>
                      <SelectContent>
                        {bankAccounts.length === 0 ? <SelectEmpty /> : null}
                        <SelectItem value="none">None</SelectItem>
                        {bankAccounts.map(account => (
                          <SelectItem key={account.id} value={account.id}>
                            {account.displayName}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </SettingsSection>
      </form>
    </Form>
  )
}

const ProviderSelect = ({
  rail,
  options,
  value,
  onChange,
}: {
  rail: PaymentRail
  options: ProviderOption[]
  value: string | undefined
  onChange: (value: string) => void
}) => {
  const selectable = options.filter(o => !o.unavailable)
  // A disabled connection stays routed until changed; it shows as the value, flagged.
  const selected = options.find(o => o.connector.id === value)?.connector
  const selectedDisabled = selected?.status === ConnectorStatusEnum.DISABLED
  return (
    <div className="space-y-1">
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger className="w-72">
          <SelectValue placeholder="Select a provider" />
        </SelectTrigger>
        <SelectContent>
          {selectable.length === 0 ? <SelectEmpty /> : null}
          <SelectItem value="none">None</SelectItem>
          {options.map(({ connector, unavailable }) => (
            <SelectItem
              key={connector.id}
              value={connector.id}
              // The routed value must stay a real item, or the trigger renders empty.
              disabled={!!unavailable && connector.id !== value}
            >
              <div className="flex items-center gap-2">
                <div className="w-5 h-5 rounded flex items-center justify-center">
                  <span className="text-xs">
                    <ProviderIcon connector={connector} />
                  </span>
                </div>
                {connector.alias}
                {connector.status === ConnectorStatusEnum.DISABLED ? (
                  <Badge variant="secondary" size="sm">
                    Disabled
                  </Badge>
                ) : unavailable ? (
                  <span className="text-xs text-muted-foreground">{unavailable}</span>
                ) : null}
                <CredentialsErrorBadge connector={connector} />
              </div>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {selectedDisabled && (
        <p className="text-xs text-destructive">
          Disabled: {RAIL_NOUNS[rail]} being collected for this entity.
        </p>
      )}
      {!selectedDisabled && selected?.credentialsError && (
        <p className="text-xs text-destructive">
          The provider rejected this connection&apos;s credentials. Update them in Integrations.
        </p>
      )}
    </div>
  )
}

export const PaymentMethodsTab = () => {
  const { selectedEntityId: invoiceEntityId } = useInvoicingEntity()

  const connectorsQuery = useQuery(listConnectors, {
    connectorType: ConnectorTypeEnum.PAYMENT_PROVIDER,
  })

  const bankAccountsQuery = useQuery(listBankAccounts)

  const providersQuery = useQuery(
    getInvoicingEntityProviders,
    {
      id: invoiceEntityId!,
    },
    { enabled: !!invoiceEntityId }
  )

  if (
    connectorsQuery.isLoading ||
    !invoiceEntityId ||
    providersQuery.isLoading ||
    bankAccountsQuery.isLoading
  ) {
    return <InlineLoading />
  }

  const paymentProviders =
    connectorsQuery.data?.connectors.filter(
      connector => connector.connectorType === ConnectorTypeEnum.PAYMENT_PROVIDER
    ) || []

  const bankAccounts =
    bankAccountsQuery.data?.accounts.map(account => ({
      id: account.id,
      name: account.data?.bankName || 'Unknown Bank',
      currency: account.data?.currency || '',
      displayName: `${account.data?.bankName || 'Unknown Bank'} (${account.data?.currency})`,
    })) || []

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <PaymentMethodsForm
        key={invoiceEntityId}
        invoiceEntityId={invoiceEntityId}
        providersData={providersQuery.data}
        paymentProviders={paymentProviders}
        bankAccounts={bankAccounts}
      />

      <BankAccountsCard />
    </div>
  )
}
