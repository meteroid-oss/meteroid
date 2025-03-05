import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Badge,
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
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { Combobox } from '@/components/Combobox'
import { Loading } from '@/components/Loading'
import { BrandIcon } from '@/features/settings/tabs/IntegrationsTab'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum, ConnectorTypeEnum } from '@/rpc/api/connectors/v1/models_pb'
import {
  getInvoicingEntityProviders,
  listInvoicingEntities,
  updateInvoicingEntityProviders,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { siStripe } from 'simple-icons'

const paymentMethodsSchema = z.object({
  ccProviderId: z.string().optional(),
  bankAccountId: z.string().optional(),
})

export const PaymentMethodsTab = () => {
  const queryClient = useQueryClient()

  const listInvoicingEntitiesQuery = useQuery(listInvoicingEntities)

  const connectorsQuery = useQuery(listConnectors, {
    connectorType: ConnectorTypeEnum.PAYMENT_PROVIDER,
  })

  const defaultInvoicingEntity = listInvoicingEntitiesQuery.data?.entities?.find(
    entity => entity.isDefault
  )

  const [invoiceEntityId, setInvoiceEntityId] = useState<string | undefined>(
    defaultInvoicingEntity?.id
  )

  const providersQuery = useQuery(
    getInvoicingEntityProviders,
    {
      id: invoiceEntityId!!,
    },
    { enabled: !!invoiceEntityId }
  )

  const updateInvoicingEntityMut = useMutation(updateInvoicingEntityProviders, {
    onSuccess: async res => {
      queryClient.setQueryData(
        createConnectQueryKey(getInvoicingEntityProviders, { id: invoiceEntityId!! }),
        createProtobufSafeUpdater(getInvoicingEntityProviders, () => {
          return {
            ccProvider: res.ccProvider,
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
      ccProviderId: '',
      bankAccountId: '',
    },
  })

  useEffect(() => {
    if (defaultInvoicingEntity && !invoiceEntityId) {
      setInvoiceEntityId(defaultInvoicingEntity.id)
    }
  }, [defaultInvoicingEntity])

  useEffect(() => {
    if (providersQuery.data) {
      methods.setValue('ccProviderId', providersQuery.data.ccProvider?.id)
      methods.setValue('bankAccountId', providersQuery.data.bankAccount?.id)
    }
  }, [providersQuery.data])

  if (
    listInvoicingEntitiesQuery.isLoading ||
    connectorsQuery.isLoading ||
    (invoiceEntityId && providersQuery.isLoading)
  ) {
    return <Loading />
  }

  const onSubmit = async (values: z.infer<typeof paymentMethodsSchema>) => {
    await updateInvoicingEntityMut.mutateAsync({
      id: invoiceEntityId,
      ccProviderId: values.ccProviderId?.length ? values.ccProviderId : undefined,
      bankAccountId: values.bankAccountId?.length ? values.bankAccountId : undefined,
    })
  }

  const paymentProviders =
    connectorsQuery.data?.connectors.filter(
      connector => connector.connectorType === ConnectorTypeEnum.PAYMENT_PROVIDER
    ) || []

  const bankAccounts: { id: string; name: string }[] = [] // TODO

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
                <Combobox
                  placeholder="Select"
                  className="max-w-[300px]"
                  value={invoiceEntityId}
                  onChange={setInvoiceEntityId}
                  options={
                    listInvoicingEntitiesQuery.data?.entities.map(entity => ({
                      label: (
                        <div className="flex flex-row w-full">
                          <div className="pr-2">{getCountryFlagEmoji(entity.country)}</div>
                          <div>{entity.legalName}</div>
                          <div className="flex-grow" />
                          {entity.isDefault && (
                            <Badge variant="primary" size={'sm'}>
                              Default
                            </Badge>
                          )}
                        </div>
                      ),
                      value: entity.id,
                    })) ?? []
                  }
                />
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
                        value={methods.watch('ccProviderId')}
                        onValueChange={value =>
                          methods.setValue('ccProviderId', value, { shouldDirty: true })
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
                  {/* <tr className="border-b">
                    <td className="py-4 px-3 flex items-center">
                      <span className="mr-2">SEPA Direct Debit</span>
                      <span className="inline-block">⬇️</span>
                    </td>
                    <td className="py-4 px-3">
                      <div className="flex items-center text-gray-400">
                        <span>Unavailable</span>
                        <InfoIcon size={16} className="ml-2" />
                      </div>
                    </td>
                  </tr>
                  <tr className="border-b">
                    <td className="py-4 px-3 flex items-center">
                      <span className="mr-2">Bacs Direct Debit</span>
                      <span className="inline-block">⬇️</span>
                    </td>
                    <td className="py-4 px-3">
                      <div className="flex items-center text-gray-400">
                        <span>Unavailable</span>
                        <InfoIcon size={16} className="ml-2" />
                      </div>
                    </td>
                  </tr>
                  <tr className="border-b">
                    <td className="py-4 px-3 flex items-center">
                      <span className="mr-2">ACH Direct Debit</span>
                      <span className="inline-block">⬇️</span>
                    </td>
                    <td className="py-4 px-3">
                      <div className="flex items-center text-gray-400">
                        <span>Unavailable</span>
                        <InfoIcon size={16} className="ml-2" />
                      </div>
                    </td>
                  </tr> */}
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
                              {account.name || account.id}
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
    </div>
  )
}
