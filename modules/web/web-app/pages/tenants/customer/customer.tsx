import { useMutation } from '@connectrpc/connect-query'
import { Button, Card, Flex, Separator, Skeleton } from '@md/ui'
import { ChevronDown, ExternalLink, Plus } from 'lucide-react'
import { Fragment, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { TenantPageLayout } from '@/components/layouts'
import { CustomerHeader, CustomersCreatePanel } from '@/features/customers'
import { InvoicesCard } from '@/features/customers/cards/InvoicesCard'
import { PaymentMethodsCard } from '@/features/customers/cards/PaymentMethodsCard'
import { SubscriptionsCard } from '@/features/customers/cards/SubscriptionsCard'
import { AddressLinesCompact } from '@/features/customers/cards/address/AddressCard'
import { EditCustomerModal } from '@/features/customers/cards/customer/EditCustomerModal'
import { CustomerInvoiceModal } from '@/features/customers/modals/CustomerInvoiceModal'
import { ManageConnectionsModal } from '@/features/customers/modals/ManageConnectionsModal'
import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import {
  generateCustomerPortalToken,
  getCustomerById,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { getInvoicingEntity } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { useTypedParams } from '@/utils/params'

export const Customer = () => {
  const { customerId } = useTypedParams<{ customerId: string }>()
  const navigate = useNavigate()
  const basePath = useBasePath()

  const [editPanelVisible, setEditPanelVisible] = useState(false)
  const [createInvoiceVisible, setCreateInvoiceVisible] = useState(false)
  const [editCustomerVisible, setEditCustomerVisible] = useState(false)
  const [manageConnectionsVisible, setManageConnectionsVisible] = useState(false)

  const customerQuery = useQuery(
    getCustomerById,
    {
      id: customerId ?? '',
    },
    { enabled: Boolean(customerId) }
  )

  const data = customerQuery.data?.customer

  const portalTokenMutation = useMutation(generateCustomerPortalToken, {
    onSuccess: data => {
      const portalUrl = `${window.location.origin}/portal/customer?token=${data.token}`
      window.open(portalUrl, '_blank')
    },
    onError: error => {
      console.error('Failed to generate customer portal token:', error)
    },
  })

  const handleOpenCustomerPortal = () =>
    portalTokenMutation.mutate({ customerId: customerId ?? '' })

  const invoicingEntityQuery = useQuery(
    getInvoicingEntity,
    {
      id: data?.invoicingEntityId ?? '',
    },
    { enabled: Boolean(data?.invoicingEntityId) }
  )

  const isLoading = customerQuery.isLoading || invoicingEntityQuery.isLoading

  return (
    <Fragment>
      <TenantPageLayout>
        <Flex direction="column" className="h-full">
          <CustomerHeader
            setEditPanelVisible={setEditPanelVisible}
            id={data?.id}
            name={data?.name || data?.alias}
            archivedAt={data?.archivedAt?.toDate()}
            // setShowIncoice={() => setCreateInvoiceVisible(true)}
            setShowIncoice={() => false}
            setShowEditCustomer={() => setEditCustomerVisible(true)}
          />
          {isLoading || !data ? (
            <>
              <Skeleton height={16} width={50} />
              <Skeleton height={44} />
            </>
          ) : (
            <Flex className="h-full">
              <Flex direction="column" className="gap-4 w-2/3 border-r border-border px-12 py-6">
                <div className="text-lg font-medium">Overview</div>
                <div className="grid grid-cols-2 gap-x-4">
                  <OverviewCard title="MRR" value={undefined} />
                  <OverviewCard
                    title="Balance"
                    value={data?.balanceValueCents ? Number(data.balanceValueCents) : undefined}
                  />
                </div>
                <Flex align="center" justify="between" className="mt-4">
                  <div className="text-lg font-medium">Subscriptions</div>
                  <Flex
                    align="center"
                    className="gap-1 text-sm cursor-pointer hover:text-primary"
                    onClick={() =>
                      navigate(`${basePath}/subscriptions/create?customerId=${customerId}`)
                    }
                  >
                    <Plus size={10} /> New subscription
                  </Flex>
                </Flex>
                <div className="flex-none">
                  <SubscriptionsCard customer={data} />
                </div>
                <Flex align="center" justify="between" className="mt-4">
                  <div className="text-lg font-medium">Invoices</div>
                  <Flex
                    align="center"
                    className="gap-1 text-sm cursor-pointer hover:text-primary"
                    onClick={() => navigate(`${basePath}/invoices/create?customerId=${customerId}`)}
                  >
                    <Plus size={10} /> New invoice
                  </Flex>
                </Flex>
                <div className="flex-none">
                  <InvoicesCard customer={data} />
                </div>
              </Flex>
              <Flex direction="column" className="gap-2 w-1/3">
                <Flex direction="column" className="gap-2 p-6">
                  <div className="flex justify-between">
                    <div className="text-lg font-medium">{data.name}</div>

                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setEditCustomerVisible(true)}
                    >
                      Edit
                    </Button>
                  </div>
                  <div className="text-muted-foreground text-[13px] mb-3">{data.alias}</div>
                  <FlexDetails title="Legal name" value={data.name} />
                  <FlexDetails title="Email" value={data.billingEmail} />
                  <FlexDetails title="Currency" value={data.currency} />
                  <FlexDetails
                    title="Invoicing Entity"
                    value={
                      invoicingEntityQuery.data?.entity ? (
                        <div className="flex items-center gap-1">
                          {invoicingEntityQuery.data.entity.country && (
                            <span>
                              {getCountryFlagEmoji(invoicingEntityQuery.data.entity.country)}
                            </span>
                          )}
                          <span>{invoicingEntityQuery.data.entity.legalName}</span>
                        </div>
                      ) : (
                        'N/A'
                      )
                    }
                  />
                  {data.billingAddress && (
                    <FlexDetails
                      title="Address"
                      value={
                        <AddressLinesCompact address={data.billingAddress} className="text-right" />
                      }
                    />
                  )}

                  <FlexDetails
                    title="Country"
                    value={
                      data.billingAddress?.country ? (
                        <span className="flex items-center gap-1 justify-end">
                          <span>{getCountryFlagEmoji(data.billingAddress.country)}</span>
                          <span>{getCountryName(data.billingAddress.country)}</span>
                        </span>
                      ) : (
                        ''
                      )
                    }
                  />

                  <FlexDetails
                    title="Custom taxes"
                    value={
                      data.customTaxes && data.customTaxes.length > 0 ? (
                        <div className="flex flex-col gap-0.5 items-end">
                          {data.customTaxes.map((tax, idx) => (
                            <div key={idx} className="text-[13px]">
                              {tax.name} ({tax.taxCode}): {Number(tax.rate) * 100}%
                            </div>
                          ))}
                        </div>
                      ) : (
                        'Default'
                      )
                    }
                  />
                  <FlexDetails
                    title="Tax ID"
                    value={data.vatNumber || (data.isTaxExempt ? 'Tax Exempt' : 'None')}
                  />
                </Flex>
                <Separator className="-my-3" />
                <Flex direction="column" className="gap-2 p-6">
                  <div className="text-[15px] font-medium">Invoicing</div>
                  <FlexDetails title="Invoicing emails" value={data.invoicingEmails?.join(', ')} />
                </Flex>
                <Separator className="-my-3" />
                <Flex direction="column" className="gap-2 p-6">
                  <div className="text-[15px] font-medium">Portal</div>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={handleOpenCustomerPortal}
                    disabled={portalTokenMutation.isPending}
                    className="w-full"
                  >
                    <ExternalLink size={14} className="mr-2" />
                    Open Customer Portal
                  </Button>
                </Flex>
                <Separator className="-my-3" />
                <Flex direction="column" className="gap-2 p-6">
                  <Flex align="center" justify="between" className="mb-2">
                    <div className="text-[15px] font-medium">Integrations</div>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => setManageConnectionsVisible(true)}
                      className="h-7 text-xs"
                    >
                      Manage
                    </Button>
                  </Flex>
                  <FlexDetails title="Alias (External ID)" value={data.alias} />
                  {data.customerConnections?.map(connection => {
                    const providerName = getProviderName(connection.connectorProvider)
                    const externalLink = getProviderLink(
                      connection.connectorProvider,
                      connection.externalCustomerId,
                      connection.externalCompanyId
                    )
                    return (
                      <FlexDetails
                        key={connection.id}
                        title={`${providerName} ID`}
                        value={connection.externalCustomerId}
                        externalLink={externalLink}
                      />
                    )
                  })}
                </Flex>
                <Separator className="-my-3" />
                <Flex direction="column" className="gap-2 p-6">
                  <div className="text-[15px] font-medium">Payment methods</div>
                  <PaymentMethodsCard
                    paymentMethods={data.paymentMethods ?? []}
                    currentPaymentMethodId={data.currentPaymentMethodId}
                  />
                </Flex>
              </Flex>
            </Flex>
          )}
        </Flex>
      </TenantPageLayout>
      <CustomersCreatePanel
        visible={editPanelVisible}
        closePanel={() => setEditPanelVisible(false)}
      />
      <CustomerInvoiceModal openState={[createInvoiceVisible, setCreateInvoiceVisible]} />
      {data && (
        <EditCustomerModal
          customer={data}
          visible={editCustomerVisible}
          onCancel={() => setEditCustomerVisible(false)}
        />
      )}
      <ManageConnectionsModal
        openState={[manageConnectionsVisible, setManageConnectionsVisible]}
        customer={data}
        onSuccess={() => {
          customerQuery.refetch()
        }}
      />
    </Fragment>
  )
}

const OverviewCard = ({ title, value }: { title: string; value?: number }) => (
  <Card className="bg-[#1A1A1A] bg-gradient-to-t from-[rgba(243,242,241,0.00)] to-[rgba(243,242,241,0.02)] rounded-md p-5">
    <Flex align="center" className="gap-1 text-muted-foreground">
      <div className="text-[13px]">{title}</div>
      <ChevronDown size={10} className="mt-0.5" />
    </Flex>
    <div className="mt-4 text-xl">€ {value}</div>
  </Card>
)

const FlexDetails = ({
  title,
  value,
  externalLink,
}: {
  title: string
  value?: string | React.ReactNode
  externalLink?: string
}) => (
  <Flex align="center" justify="between">
    <div className="text-[13px] text-muted-foreground">{title}</div>
    <div className="max-w-[250px] break-words text-right">
      {externalLink ? (
        <a href={externalLink} target="_blank" rel="noopener noreferrer">
          <div className="text-[13px] text-brand hover:underline">{value ?? 'N/A'}</div>
        </a>
      ) : (
        <div className="text-[13px]">{value ?? '-'}</div>
      )}
    </div>
  </Flex>
)

// Helper functions for connector providers
const getProviderName = (provider: ConnectorProviderEnum | undefined): string => {
  switch (provider) {
    case ConnectorProviderEnum.STRIPE:
      return 'Stripe'
    case ConnectorProviderEnum.HUBSPOT:
      return 'Hubspot'
    case ConnectorProviderEnum.PENNYLANE:
      return 'Pennylane'
    default:
      return 'Unknown'
  }
}

const getProviderLink = (
  provider: ConnectorProviderEnum | undefined,
  externalId: string,
  externalCompanyId?: string
): string | undefined => {
  // Return external links for providers that support it
  switch (provider) {
    case ConnectorProviderEnum.STRIPE:
      return `https://dashboard.stripe.com/customers/${externalId}`
    case ConnectorProviderEnum.HUBSPOT:
      return externalCompanyId
        ? `https://app.hubspot.com/contacts/${externalCompanyId}/company/${externalId}`
        : undefined
    case ConnectorProviderEnum.PENNYLANE:
      return externalCompanyId
        ? `https://app.pennylane.com/companies/${externalCompanyId}/thirdparties/customers?id=${externalId}`
        : undefined
    default:
      return undefined
  }
}
