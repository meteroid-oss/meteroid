import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Button,
  Card,
  Form,
  Select,
  SelectContent,
  SelectEmpty,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { siStripe } from 'simple-icons'
import { toast } from 'sonner'
import { z } from 'zod'

import { Loading } from '@/components/Loading'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import { BankAccountsCard } from '@/features/settings/components/bankaccounts'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { BrandIcon } from '@/features/settings/tabs/IntegrationsTab'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listBankAccounts } from '@/rpc/api/bankaccounts/v1/bankaccounts-BankAccountsService_connectquery'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum, ConnectorTypeEnum } from '@/rpc/api/connectors/v1/models_pb'
import {
  getInvoicingEntityProviders,
  updateInvoicingEntityProviders,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

const paymentMethodsSchema = z.object({
  cardProviderId: z.string().optional(),
  directDebitProviderId: z.string().optional(),
  bankAccountId: z.string().optional(),
})

export const PaymentMethodsTab = () => {
  const queryClient = useQueryClient()
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

  const updateInvoicingEntityMut = useMutation(updateInvoicingEntityProviders, {
    onSuccess: async res => {
      queryClient.setQueryData(
        createConnectQueryKey(getInvoicingEntityProviders, { id: invoiceEntityId! }),
        createProtobufSafeUpdater(getInvoicingEntityProviders, () => {
          return {
            cardProvider: res.cardProvider,
            directDebitProvider: res.directDebitProvider,
            bankAccount: res.bankAccount,
          }
        })
      )
      toast.success('Payment methods updated')
    },
  })

  const methods = useZodForm({
    schema: paymentMethodsSchema,
    defaultValues: {
      cardProviderId: '',
      directDebitProviderId: '',
      bankAccountId: '',
    },
  })

  useEffect(() => {
    if (providersQuery.data && !methods.formState.isDirty) {
      methods.setValue('cardProviderId', providersQuery.data.cardProvider?.id)
      methods.setValue('directDebitProviderId', providersQuery.data.directDebitProvider?.id)
      methods.setValue('bankAccountId', providersQuery.data.bankAccount?.id)
    }
  }, [providersQuery.data, methods.formState.isDirty, invoiceEntityId])

  if (
    connectorsQuery.isLoading ||
    (invoiceEntityId && providersQuery.isLoading) ||
    bankAccountsQuery.isLoading
  ) {
    return <Loading />
  }

  const onSubmit = async (values: z.infer<typeof paymentMethodsSchema>) => {
    await updateInvoicingEntityMut.mutateAsync({
      id: invoiceEntityId,
      cardProviderId: values.cardProviderId?.length ? values.cardProviderId : undefined,
      directDebitProviderId: values.directDebitProviderId?.length
        ? values.directDebitProviderId
        : undefined,
      bankAccountId: values.bankAccountId?.length ? values.bankAccountId : undefined,
    })
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
    <div className="flex flex-col gap-4">
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
          <Card className="px-8 py-6 max-w-[950px] space-y-4">
            <div className="grid grid-cols-6 gap-4">
              <div className="col-span-2">
                <h3 className="font-medium text-lg">Payment method routing</h3>
                <p className="text-sm text-muted-foreground mt-1">
                  Customers linked to this invoicing entity will inherit from this configuration.
                </p>
              </div>
              <div className="col-span-4 content-center flex flex-row">
                <div className="flex-grow"></div>
                <InvoicingEntitySelect />
              </div>
            </div>

            <div className="mt-6 text-sm">
              <table className="w-full">
                <thead>
                  <tr className="border-b">
                    <th className="text-left py-2 px-3 font-medium text-muted-foreground">
                      Method
                    </th>
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
                      <Select
                        value={methods.watch('cardProviderId')}
                        onValueChange={value =>
                          methods.setValue('cardProviderId', value, { shouldDirty: true })
                        }
                      >
                        <SelectTrigger className="w-60">
                          <SelectValue placeholder="Select a provider" />
                        </SelectTrigger>
                        <SelectContent>
                          {paymentProviders.length == 0 ? <SelectEmpty /> : null}

                          {paymentProviders.map(provider => (
                            <SelectItem key={provider.id} value={provider.id}>
                              <div className="flex items-center">
                                <div className="w-5 h-5  rounded flex items-center justify-center mr-2">
                                  <span className="  text-xs">
                                    {provider.provider === ConnectorProviderEnum.STRIPE ? (
                                      <BrandIcon
                                        path={siStripe.path}
                                        color="#635bff"
                                        className="w-3 h-3"
                                      />
                                    ) : (
                                      'P'
                                    )}
                                  </span>
                                </div>
                                {provider.alias}
                              </div>
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </td>
                  </tr>
                  <tr className="border-b">
                    <td className="py-4 px-3 flex items-center">
                      <span className="mr-2 inline-block">⬇️</span>
                      <span className="mr-2">Direct Debit (SEPA, BACS, ACH)</span>
                    </td>
                    <td className="py-4 px-3">
                      <Select
                        value={methods.watch('directDebitProviderId')}
                        onValueChange={value =>
                          methods.setValue('directDebitProviderId', value, { shouldDirty: true })
                        }
                      >
                        <SelectTrigger className="w-60">
                          <SelectValue placeholder="Select a provider" />
                        </SelectTrigger>
                        <SelectContent>
                          {paymentProviders.length == 0 ? <SelectEmpty /> : null}

                          {paymentProviders.map(provider => (
                            <SelectItem key={provider.id} value={provider.id}>
                              <div className="flex items-center">
                                <div className="w-5 h-5  rounded flex items-center justify-center mr-2">
                                  <span className="  text-xs">
                                    {provider.provider === ConnectorProviderEnum.STRIPE ? (
                                      <BrandIcon
                                        path={siStripe.path}
                                        color="#635bff"
                                        className="w-3 h-3"
                                      />
                                    ) : (
                                      'P'
                                    )}
                                  </span>
                                </div>
                                {provider.alias}
                              </div>
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
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
                          {bankAccounts.length == 0 ? <SelectEmpty /> : null}
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

            <div className="pt-10 flex justify-end items-center">
              <div>
                <Button
                  size="sm"
                  // disabled={!methods.formState.isValid || updateInvoicingEntityMut.isPending}
                >
                  Save changes
                </Button>
              </div>
            </div>
          </Card>
        </form>
      </Form>

      <BankAccountsCard />
    </div>
  )
}
