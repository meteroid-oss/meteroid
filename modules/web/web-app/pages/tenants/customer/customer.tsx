import { timestampDate } from '@bufbuild/protobuf/wkt'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Flex, Separator, Skeleton, Tabs, TabsContent, TabsList, TabsTrigger } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { cn } from '@ui/lib'
import { AlertTriangle } from 'lucide-react'
import { Fragment, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'

import { LocalId } from '@/components/LocalId'
import { TenantPageLayout } from '@/components/layouts'
import {
  DetailColumns,
  DetailMain,
  DetailPageFrame,
  DetailRow,
  DetailSidebar,
  DetailTitle,
} from '@/components/layouts/DetailLayout'
import { Slot, placeAfter, slotItems, useVisibleItems } from '@/edition'
import { CustomerWalletsPanel, useCustomerCreditBalances } from '@/features/credit'
import { CustomerHeader, CustomersCreatePanel } from '@/features/customers'
import { CustomerAvatar } from '@/features/customers/CustomerAvatar'
import { OwnerPicker } from '@/features/customers/OwnerPicker'
import { ActivityCard } from '@/features/customers/cards/ActivityCard'
import {
  ContainedTableSection,
  SectionAddButton,
} from '@/features/customers/cards/ContainedTableSection'
import { CustomerSubscriptions } from '@/features/customers/cards/CustomerSubscriptions'
import { CustomerSummaryStrip } from '@/features/customers/cards/CustomerSummary'
import { CustomerUsageCard } from '@/features/customers/cards/CustomerUsageCard'
import { InvoicesCard } from '@/features/customers/cards/InvoicesCard'
import { PaymentMethodsCard } from '@/features/customers/cards/PaymentMethodsCard'
import { QuotesCard } from '@/features/customers/cards/QuotesCard'
import { UnattributedUsageSection } from '@/features/customers/cards/UnattributedUsageSection'
import { AddressLinesCompact } from '@/features/customers/cards/address/AddressCard'
import { EditCustomerModal } from '@/features/customers/cards/customer/EditCustomerModal'
import { getPaymentRoutingWarnings } from '@/features/customers/cards/paymentRoutingWarnings'
import { VatValidationBadge } from '@/features/customers/components/BillingInfoCard'
import { CustomerInvoiceModal } from '@/features/customers/modals/CustomerInvoiceModal'
import { ManageConnectionsModal } from '@/features/customers/modals/ManageConnectionsModal'
import { CustomPropertyValue } from '@/features/customproperties/CustomPropertyValue'
import { useCustomPropertyRows } from '@/features/customproperties/useCustomPropertyRows'
import { CustomerEntitlementOverrides } from '@/features/entitlements/customer/CustomerEntitlementOverrides'
import { EffectiveEntitlementsCard } from '@/features/entitlements/customer/EffectiveEntitlementsCard'
import { connectorDisplayName, paymentProvider } from '@/features/payments/providers'
import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { useQueryState } from '@/hooks/useQueryState'
import { useScrollFade } from '@/hooks/useScrollFade'
import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { rateToPercent } from '@/lib/utils/numbers'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import {
  generateCustomerPortalToken,
  getCustomerById,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { CustomerTaxStatus, VatNumberValidationStatus } from '@/rpc/api/customers/v1/models_pb'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import {
  getInvoicingEntity,
  getInvoicingEntityProviders,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { useTypedParams } from '@/utils/params'

// Former ids of the module tabs, each to the tab it now names.
const tabAliases = (): Record<string, string> =>
  Object.fromEntries(
    slotItems('customerDetail.tabs').flatMap(tab =>
      (tab.aliases ?? []).map(alias => [alias, tab.id])
    )
  )

export const Customer = () => {
  const { customerId } = useTypedParams<{ customerId: string }>()
  const navigate = useNavigate()
  const basePath = useBasePath()
  const isExpress = useIsExpressOrganization()
  const { tenant } = useTenant()
  const isSandbox = tenant?.environment !== TenantEnvironmentEnum.PRODUCTION

  // Gate each section by the permission its RPCs require, so a viewer without a
  // given area never fires (and 403s) those requests. manage implies read, so
  // these are true for managers too (see backend `to_effective_codes`).
  const { has } = usePermissions()
  const canViewSubscriptions = has('billing:subscriptions:read')
  const canManageSubscriptions = has('billing:subscriptions:manage')
  const canViewInvoices = has('billing:invoices:read')
  const canManageInvoices = has('billing:invoices:manage')
  const canViewQuotes = has('billing:quotes:read')
  const canManageQuotes = has('billing:quotes:manage')
  const canViewAnalytics = has('analytics:read')
  const canViewCatalog = has('catalog:read')
  const canManageCustomers = has('customers:manage')
  const tabSerde = useMemo(() => {
    const aliases = tabAliases()
    return { serialize: (tab: string) => tab, deserialize: (tab: string) => aliases[tab] ?? tab }
  }, [])
  const showEntitlements = env.entitlementsEnabled && canViewCatalog
  const moduleTabs = useVisibleItems(slotItems('customerDetail.tabs'), {
    customerId: customerId ?? '',
  })

  // Module tabs follow the core content tabs unless they name the tab they follow.
  const contentTabs = [
    { id: 'overview', label: 'Overview' },
    { id: 'usage', label: 'Usage' },
    ...(showEntitlements ? [{ id: 'entitlements', label: 'Entitlements' }] : []),
  ]
  const tabList = placeAfter<{ id: string; label: string; after?: string }>(
    [
      ...contentTabs,
      ...(!isExpress && canViewInvoices ? [{ id: 'wallets', label: 'Credit' }] : []),
    ],
    moduleTabs,
    tab => tab.id,
    contentTabs.length
  )
  const tabs = tabList.map(t => t.id)
  const [requestedTab, setTab] = useQueryState('tab', 'overview', {
    ...tabSerde,
    replace: true,
  })
  const tab = tabs.includes(requestedTab) ? requestedTab : 'overview'
  const tabFade = useScrollFade<HTMLDivElement>()

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
  const creditBalances = useCustomerCreditBalances(
    customerId ?? '',
    Boolean(customerId) && canViewInvoices
  )

  const customPropertyRows = useCustomPropertyRows(
    CustomPropertyEntityType.CUSTOMER,
    data?.customProperties
  )

  // Connector aliases (the user-facing connector names) to label each external
  // customer connection by its actual connector rather than a hardcoded
  // provider string.
  const connectorsQuery = useQuery(listConnectors, {})
  const connectorAliasById = new Map(
    connectorsQuery.data?.connectors.map(c => [c.id, c.alias]) ?? []
  )

  // VIES validation resolves asynchronously after a VAT change; poll a few
  // times while it's pending so the badge settles without a manual refresh.
  const queryClient = useQueryClient()
  const vatStatus = data?.vatNumberValidationStatus
  useEffect(() => {
    if (!customerId || vatStatus !== VatNumberValidationStatus.PENDING) return
    let attempts = 0
    const interval = setInterval(() => {
      attempts += 1
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getCustomerById,
          input: { id: customerId },
          cardinality: 'finite',
        }),
      })
      if (attempts >= 3) clearInterval(interval)
    }, 3000)
    return () => clearInterval(interval)
  }, [vatStatus, customerId, queryClient])

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

  // Providers configured on the customer's invoicing entity, used to warn when
  // the customer's data would fail a provider's setup (e.g. GoCardless country).
  const invoicingEntityProvidersQuery = useQuery(
    getInvoicingEntityProviders,
    { id: data?.invoicingEntityId ?? '' },
    // LOCAL gated this on the removed 'billing:read'; the nearest fine-grained
    // codes are the invoices/subscriptions read permissions.
    {
      enabled: (canViewInvoices || canViewSubscriptions) && Boolean(data?.invoicingEntityId),
    }
  )

  const routingWarnings =
    data && invoicingEntityProvidersQuery.data
      ? getPaymentRoutingWarnings(data, invoicingEntityProvidersQuery.data)
      : []

  const isLoading = customerQuery.isLoading || invoicingEntityQuery.isLoading

  return (
    <Fragment>
      <TenantPageLayout>
        <DetailPageFrame
          header={
            <CustomerHeader
              setShowEditCustomer={() => setEditCustomerVisible(true)}
              id={data?.id}
              name={data?.name || data?.alias}
              archivedAt={data?.archivedAt ? timestampDate(data.archivedAt) : undefined}
              lifecycleState={data?.lifecycleState}
              onOpenPortal={!isExpress ? handleOpenCustomerPortal : undefined}
              portalPending={portalTokenMutation.isPending}
            />
          }
        >
          {isLoading || !data ? (
            <div className="flex flex-col gap-2 p-6">
              <Skeleton height={16} width={50} />
              <Skeleton height={44} />
            </div>
          ) : (
            <DetailColumns>
              <DetailMain>
                <DetailTitle
                  media={<CustomerAvatar name={data.name} size="lg" />}
                  title={data.name}
                  subtitle={data.alias}
                />
                <Tabs value={tab} onValueChange={setTab} className="w-full">
                  <TabsList
                    ref={tabFade.ref}
                    variant="underline"
                    className={cn(
                      'w-full overflow-x-auto',
                      tabFade.fadeLeft && 'scroll-fade-x-left',
                      tabFade.fadeRight && 'scroll-fade-x-right'
                    )}
                  >
                    {tabList.map(t => (
                      <TabsTrigger key={t.id} value={t.id}>
                        {t.label}
                      </TabsTrigger>
                    ))}
                  </TabsList>
                  <TabsContent
                    value="overview"
                    // 32px above and below each divider between the sections. Drawn as the top border of the
                    // section below it, so a section can leave its own out.
                    className="mt-6 flex flex-col [&>*+*]:border-t [&>*+*]:border-border [&>*]:py-8 [&>*:first-child]:pt-0 [&>*:last-child]:pb-0"
                  >
                    <CustomerSummaryStrip customerId={data.id} customer={data} />
                    <Slot name="customerDetail.attention" customerId={data.id} />
                    {!isExpress && canViewSubscriptions && (
                      // Follows the cards without a divider, 32px below them.
                      <section className="flex flex-col gap-4 !border-t-0 !pt-0">
                        <Flex align="center" justify="between" className="h-7">
                          <h3 className="text-base font-medium">Subscriptions</h3>
                          {canManageSubscriptions && (
                            <SectionAddButton
                              label="New subscription"
                              onClick={() =>
                                navigate(
                                  `${basePath}/subscriptions/create?customerId=${customerId}`
                                )
                              }
                            />
                          )}
                        </Flex>
                        <CustomerSubscriptions
                          customerId={data.id}
                          onCreate={
                            canManageSubscriptions
                              ? () =>
                                  navigate(
                                    `${basePath}/subscriptions/create?customerId=${customerId}`
                                  )
                              : undefined
                          }
                        />
                      </section>
                    )}
                    {!isExpress && canViewSubscriptions && (
                      <UnattributedUsageSection customerId={data.id} />
                    )}
                    {canViewInvoices && (
                      <ContainedTableSection
                        title="Invoices"
                        actionLabel="New invoice"
                        onAction={
                          !isExpress && canManageInvoices
                            ? () => navigate(`${basePath}/invoices/create?customerId=${customerId}`)
                            : undefined
                        }
                      >
                        <InvoicesCard
                          customer={data}
                          onCreate={
                            !isExpress && canManageInvoices
                              ? () =>
                                  navigate(`${basePath}/invoices/create?customerId=${customerId}`)
                              : undefined
                          }
                        />
                      </ContainedTableSection>
                    )}
                    {!isExpress && canViewQuotes && (
                      <ContainedTableSection
                        title="Quotes"
                        actionLabel="New quote"
                        onAction={
                          canManageQuotes
                            ? () => navigate(`${basePath}/quotes/create?customerId=${customerId}`)
                            : undefined
                        }
                      >
                        <QuotesCard
                          customer={data}
                          onCreate={
                            canManageQuotes
                              ? () => navigate(`${basePath}/quotes/create?customerId=${customerId}`)
                              : undefined
                          }
                        />
                      </ContainedTableSection>
                    )}
                  </TabsContent>
                  <TabsContent value="usage" className="mt-4">
                    <CustomerUsageCard customerId={data.id} />
                  </TabsContent>
                  {tabs.includes('entitlements') && (
                    <TabsContent value="entitlements" className="mt-4 flex flex-col gap-8">
                      <div>
                        <h4 className="text-sm font-medium mb-3">Effective</h4>
                        <EffectiveEntitlementsCard
                          customerId={data.id}
                          onViewCredit={
                            tabs.includes('wallets') ? () => setTab('wallets') : undefined
                          }
                        />
                      </div>
                      <CustomerEntitlementOverrides customerId={data.id} />
                    </TabsContent>
                  )}
                  {moduleTabs.map(moduleTab => (
                    <TabsContent key={moduleTab.id} value={moduleTab.id} className="mt-4">
                      <moduleTab.Content customerId={data.id} />
                    </TabsContent>
                  ))}
                  {tabs.includes('wallets') && (
                    <TabsContent value="wallets" className="mt-4">
                      <CustomerWalletsPanel
                        customerId={data.id}
                        currency={data.currency}
                        canManage={canManageInvoices}
                        paymentMethods={data.paymentMethods ?? []}
                        defaultPaymentMethodId={data.currentPaymentMethodId}
                      />
                    </TabsContent>
                  )}
                </Tabs>
              </DetailMain>
              <DetailSidebar>
                <Flex direction="column" className="gap-3 p-6">
                  <div className="mb-2 flex h-7 items-center justify-between gap-2">
                    <div className="text-[15px] font-medium">Details</div>

                    {!isExpress && canManageCustomers && (
                      <Button
                        size="sm"
                        variant="secondary"
                        className="shrink-0"
                        onClick={() => setEditCustomerVisible(true)}
                      >
                        Edit
                      </Button>
                    )}
                  </div>
                  <DetailRow
                    title="ID"
                    value={
                      // Quiet and compact: a reference to copy, not something to read. The full ID
                      // is in the hover title and on the clipboard.
                      <span title={data.localId} className="inline-flex max-w-[160px]">
                        <LocalId
                          localId={data.localId}
                          buttonClassName="h-6 max-w-full px-2 py-0 gap-1.5 bg-transparent text-muted-foreground hover:text-foreground dark:bg-transparent"
                          className="min-w-0 text-[11px] tracking-normal"
                        />
                      </span>
                    }
                  />
                  {!isExpress && (
                    <DetailRow
                      title="Owner"
                      value={
                        <OwnerPicker
                          customerId={data.id}
                          customerName={data.name}
                          ownerUserId={data.owner?.userId}
                          align="end"
                          className="-mr-2 h-7 text-[13px]"
                        />
                      }
                    />
                  )}
                  <DetailRow title="Legal name" value={data.name} />
                  <DetailRow title="Email" value={data.billingEmail} />
                  <DetailRow title="Currency" value={data.currency} />
                  {creditBalances.map(balance => (
                    <DetailRow key={balance.id} title={balance.label} value={balance.formatted} />
                  ))}
                  <DetailRow
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
                    <DetailRow
                      title="Address"
                      value={
                        <AddressLinesCompact address={data.billingAddress} className="text-right" />
                      }
                    />
                  )}

                  <DetailRow
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

                  <DetailRow
                    title="Custom taxes"
                    value={
                      data.customTaxes && data.customTaxes.length > 0 ? (
                        <div className="flex flex-col gap-0.5 items-end">
                          {data.customTaxes.map((tax, idx) => (
                            <div key={idx} className="text-[13px]">
                              {tax.name} ({tax.taxCode}): {rateToPercent(tax.rate)}%
                            </div>
                          ))}
                        </div>
                      ) : (
                        'Default'
                      )
                    }
                  />
                  <DetailRow
                    title="Tax ID"
                    value={
                      data.vatNumber ? (
                        <span className="flex items-center justify-end">
                          <span>{data.vatNumber}</span>
                          <VatValidationBadge customer={data} />
                        </span>
                      ) : data.taxStatus === CustomerTaxStatus.EXEMPT ? (
                        'Tax Exempt'
                      ) : data.taxStatus === CustomerTaxStatus.REVERSE_CHARGE ? (
                        'Reverse charge'
                      ) : (
                        'None'
                      )
                    }
                  />
                  {data.buyerReference && (
                    <DetailRow title="Buyer reference" value={data.buyerReference} />
                  )}
                </Flex>
                {customPropertyRows.length > 0 && (
                  <>
                    <Separator className="-my-3 h-[0.5px]" />
                    <Flex direction="column" className="gap-3 p-6">
                      <div className="text-[15px] font-medium">Custom properties</div>
                      {customPropertyRows.map(row => (
                        <DetailRow
                          key={row.key}
                          title={row.label}
                          value={<CustomPropertyValue def={row.def} value={row.raw} />}
                        />
                      ))}
                    </Flex>
                  </>
                )}
                <Separator className="-my-3 h-[0.5px]" />
                <Flex direction="column" className="gap-3 p-6">
                  <div className="text-[15px] font-medium">Invoicing</div>
                  <DetailRow title="Invoicing emails" value={data.invoicingEmails?.join(', ')} />
                  <Slot name="customerDetail.invoicingRows" customerId={data.id} />
                </Flex>
                <Separator className="-my-3 h-[0.5px]" />
                <Flex direction="column" className="gap-3 p-6">
                  <Flex align="center" justify="between" className="mb-2">
                    <div className="text-[15px] font-medium">Integrations</div>
                    {!isExpress && canManageCustomers && (
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => setManageConnectionsVisible(true)}
                        className="h-7 text-xs"
                      >
                        Manage
                      </Button>
                    )}
                  </Flex>
                  <DetailRow title="Alias (External ID)" value={data.alias} />
                  {data.customerConnections?.map(connection => {
                    // Prefer the connector's own name (alias); fall back to the
                    // provider display name if the connector isn't loaded yet.
                    const connectorName =
                      connectorAliasById.get(connection.connectorId) ||
                      connectorDisplayName(connection.connectorProvider)
                    const externalLink = getProviderLink(
                      connection.connectorProvider,
                      connection.externalCustomerId,
                      connection.externalCompanyId,
                      isSandbox
                    )
                    return (
                      <DetailRow
                        key={connection.id}
                        title={`${connectorName} ID`}
                        value={connection.externalCustomerId}
                        externalLink={externalLink}
                      />
                    )
                  })}
                </Flex>
                <Separator className="-my-3 h-[0.5px]" />
                <Flex direction="column" className="gap-2 p-6">
                  <div className="text-[15px] font-medium">Payment methods</div>
                  <PaymentMethodsCard
                    paymentMethods={data.paymentMethods ?? []}
                    currentPaymentMethodId={data.currentPaymentMethodId}
                  />
                  {routingWarnings.map((warning, i) => (
                    <div
                      key={i}
                      className="flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs"
                    >
                      <AlertTriangle size={14} className="mt-0.5 shrink-0 text-warning" />
                      <div>
                        <span className="font-medium">
                          {warning.rail} ({warning.providerAlias}):
                        </span>{' '}
                        <span className="text-muted-foreground">{warning.message}</span>
                      </div>
                    </div>
                  ))}
                </Flex>
                {canViewAnalytics && (
                  <>
                    <Separator className="-my-3 h-[0.5px]" />

                    <Flex direction="column" className="gap-2 p-6">
                      <div className="text-[15px] font-medium">Activity</div>
                      <ActivityCard customer={data} />
                    </Flex>
                  </>
                )}
              </DetailSidebar>
            </DetailColumns>
          )}
        </DetailPageFrame>
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

const getProviderLink = (
  provider: ConnectorProviderEnum | undefined,
  externalId: string,
  externalCompanyId?: string,
  isSandbox?: boolean
): string | undefined => {
  switch (provider) {
    case ConnectorProviderEnum.HUBSPOT:
      return externalCompanyId
        ? `https://app.hubspot.com/contacts/${externalCompanyId}/company/${externalId}`
        : undefined
    case ConnectorProviderEnum.PENNYLANE:
      return externalCompanyId
        ? `https://app.pennylane.com/companies/${externalCompanyId}/thirdparties/customers?id=${externalId}`
        : undefined
    default:
      return paymentProvider(provider)?.customerDashboardUrl?.(externalId, isSandbox ?? true)
  }
}
