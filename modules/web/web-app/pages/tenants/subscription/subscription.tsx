import {
  Alert,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Skeleton,
} from '@md/ui'
import { ChevronDown, ChevronLeftIcon, RefreshCw } from 'lucide-react'
import { ReactNode, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import {
  IntegrationType,
  SyncSubscriptionModal,
} from '@/features/settings/integrations/SyncSubscriptionModal'
import { CancelSubscriptionModal } from '@/features/subscriptions/CancelSubscriptionModal'
import { SubscriptionInvoicesCard } from '@/features/subscriptions/InvoicesCard'
import { formatSubscriptionFee } from '@/features/subscriptions/utils/fees'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { getLatestConnMeta } from '@/pages/tenants/utils'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import {
  SubscriptionFee,
  SubscriptionFeeBillingPeriod,
  SubscriptionStatus,
} from '@/rpc/api/subscriptions/v1/models_pb'
import { getSubscriptionDetails } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { useTypedParams } from '@/utils/params'

// Status Badge Component
const StatusBadge = ({ status }: { status: SubscriptionStatus }) => {
  const statusConfig = {
    [SubscriptionStatus.PENDING]: {
      bg: 'bg-warning/20',
      text: 'text-warning',
      render: 'pending',
    },
    [SubscriptionStatus.TRIALING]: {
      bg: 'bg-secondary/20',
      text: 'text-secondary',
      render: 'trialing',
    },
    [SubscriptionStatus.ACTIVE]: {
      bg: 'bg-success/20',
      text: 'text-success',
      render: 'active',
    },
    [SubscriptionStatus.CANCELED]: {
      bg: 'bg-destructive/20',
      text: 'text-destructive',
      render: 'cancelled',
    },
    [SubscriptionStatus.ENDED]: {
      bg: 'bg-muted/20',
      text: 'text-muted-foreground',
      render: 'ended',
    },
    [SubscriptionStatus.TRIAL_EXPIRED]: {
      bg: 'bg-warning/30',
      text: 'text-warning',
      render: 'trial expired',
    },
  }

  const config = statusConfig[status]

  return (
    <span
      className={`inline-flex items-center rounded-full px-2 py-1 text-xs font-medium ${config.bg} ${config.text}`}
    >
      {config.render}
    </span>
  )
}

// Format date
const formatDate = (dateString: string | undefined) => {
  if (!dateString) return 'N/A'
  return new Date(dateString).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

// Format currency
const formatCurrency = (amountCents: number, currency: string) => {
  if (amountCents === undefined) return 'N/A'
  const amount = amountCents / 100
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: currency || 'USD',
  }).format(amount)
}

// Section Title Component
const SectionTitle = ({ children }: { children: ReactNode }) => (
  <h3 className="text-lg font-medium text-foreground mb-3">{children}</h3>
)

// Detail Row Component
const DetailRow = ({
  label,
  value,
  link,
  externalLink,
}: {
  label: string
  value: ReactNode
  link?: string
  externalLink?: string
}) => (
  <div className="text-[13px] flex justify-between py-2 border-b border-border last:border-0">
    <div className=" text-muted-foreground">{label}</div>
    {externalLink && (
      <a href={externalLink} target="_blank" rel="noopener noreferrer">
        <div className="font-medium text-brand hover:underline">{value ?? 'N/A'}</div>
      </a>
    )}

    {link && (
      <Link to={link}>
        <div className="  font-medium text-brand hover:underline">{value}</div>
      </Link>
    )}

    {!link && !externalLink && <div className=" font-medium text-foreground">{value}</div>}
  </div>
)

// Detail Section Component
const DetailSection = ({ title, children }: { title: string; children: ReactNode }) => (
  <div className="mb-6">
    <SectionTitle>{title}</SectionTitle>
    <div className="space-y-1">{children}</div>
  </div>
)

// Map billing period to display format
const formatBillingPeriod = (period: SubscriptionFeeBillingPeriod) => {
  const periodMap = {
    [SubscriptionFeeBillingPeriod.ONE_TIME]: 'One Time',
    [SubscriptionFeeBillingPeriod.MONTHLY]: 'Monthly',
    [SubscriptionFeeBillingPeriod.QUARTERLY]: 'Quarterly',
    [SubscriptionFeeBillingPeriod.SEMIANNUAL]: 'Semiannual',
    [SubscriptionFeeBillingPeriod.YEARLY]: 'Yearly',
  }
  return periodMap[period]
}

// Map subscription fee type to display format
const formatFeeType = (fee: SubscriptionFee | undefined) => {
  if (!fee) return 'N/A'

  if (fee.fee.case === 'rate') return 'Rate'
  if (fee.fee.case === 'oneTime') return 'One Time'
  if (fee.fee.case === 'recurring') return 'Recurring'
  if (fee.fee.case === 'capacity') return 'Capacity'
  if (fee.fee.case === 'slot') return 'Slot'
  if (fee.fee.case === 'usage') return 'Usage'

  return 'Unknown'
}

export const Subscription = () => {
  const navigate = useNavigate()
  const basePath = useBasePath()

  const [showSyncHubspotModal, setShowSyncHubspotModal] = useState(false)
  const [showCancelModal, setShowCancelModal] = useState(false)
  const [invoicesRefetch, setInvoicesRefetch] = useState<(() => void) | null>(null)
  const [invoicesIsFetching, setInvoicesIsFetching] = useState(false)

  const { subscriptionId } = useTypedParams()
  const subscriptionQuery = useQuery(
    getSubscriptionDetails,
    {
      subscriptionId: subscriptionId ?? '',
    },
    { enabled: Boolean(subscriptionId) }
  )

  const details = subscriptionQuery.data
  const data = details?.subscription

  const connectorsQuery = useQuery(listConnectors, {})
  const connectorsData = connectorsQuery.data?.connectors ?? []

  const isHubspotConnected = connectorsData.some(
    connector => connector.provider === ConnectorProviderEnum.HUBSPOT
  )

  const isLoading = subscriptionQuery.isLoading || connectorsQuery.isLoading

  const hubspotConnMeta = getLatestConnMeta(data?.connectionMetadata?.hubspot)

  // Subscription can be cancelled if it's not already in a terminal state
  const canCancelSubscription =
    data &&
    data.status !== SubscriptionStatus.CANCELED &&
    data.status !== SubscriptionStatus.ENDED

  if (isLoading || !data) {
    return (
      <div className="p-6">
        <Skeleton height={16} width={50} className="mb-4"/>
        <div className="flex gap-6">
          <div className="flex-1">
            <Skeleton height={100} className="mb-4"/>
            <Skeleton height={200} className="mb-4"/>
          </div>
          <div className="w-80">
            <Skeleton height={300} className="mb-4"/>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="flex min-h-screen bg-background gap-2">
      {showSyncHubspotModal && (
        <SyncSubscriptionModal
          customerName={data?.customerName ?? ''}
          id={data?.id ?? ''}
          integrationType={IntegrationType.Hubspot}
          onClose={() => setShowSyncHubspotModal(false)}
        />
      )}
      {showCancelModal && (
        <CancelSubscriptionModal
          subscriptionId={data?.id ?? ''}
          customerName={data?.customerName ?? ''}
          planName={data?.planName ?? ''}
          onClose={() => setShowCancelModal(false)}
          onSuccess={() => subscriptionQuery.refetch()}
        />
      )}
      {/* Main content area */}
      <div className="flex-1 p-6 pr-0">
        <div className="flex items-center mb-6 w-full justify-between">
          <div className="flex items-center">
            <ChevronLeftIcon
              className="cursor-pointer text-muted-foreground hover:text-foreground mr-2"
              onClick={() => navigate('..')}
              size={20}
            />
            <h2 className="text-xl font-semibold text-foreground">{data.planName}</h2>
            <div className="ml-2">
              <StatusBadge status={data.status}/>
            </div>
          </div>
          <div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="primary" className="gap-2  " size="sm" hasIcon>
                  Actions <ChevronDown className="w-4 h-4"/>
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {/* {secondaryActions.map((option, optionIndex) => (
                <DropdownMenuItem key={optionIndex} onClick={option.onClick}>
                  {option.label}
                </DropdownMenuItem>
              ))} */}
                <DropdownMenuItem
                  disabled={!isHubspotConnected}
                  onClick={() => setShowSyncHubspotModal(true)}
                >
                  Sync To Hubspot
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={!canCancelSubscription}
                  onClick={() => setShowCancelModal(true)}
                  className="text-destructive focus:text-destructive"
                >
                  Cancel Subscription
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>

        {data.checkoutUrl && data.status === SubscriptionStatus.PENDING && (
          <Alert variant="default" className="mb-6">
            <div className="flex gap-2 items-center content-between justify-between">
              <span>This subscription is pending checkout. </span>
              <CopyToClipboardButton
                text="Copy checkout link"
                textToCopy={data.checkoutUrl}
                className="whitespace-normal"
              />
            </div>
          </Alert>
        )}

        {/* Revenue summary card */}
        <div className="bg-card rounded-lg  shadow-sm p-6 mb-6">
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-medium text-foreground"></h3>
            <span className="text-2xl font-bold text-accent-foreground">
              {formatCurrency(Number(data.mrrCents), data.currency)}
              <span className="text-sm font-normal text-muted-foreground ml-1">MRR</span>
            </span>
          </div>
          <div className="grid grid-cols-3 gap-6">
            <div className="border-r border-border pr-4 last:border-0">
              <div className="text-sm text-muted-foreground">Plan</div>
              <Link
                to={`${basePath}/plans/${data.planId}/${data.version}`}
                className="text-md font-medium mt-1 text-brand hover:underline block"
              >
                {data.planName}
              </Link>
            </div>
            <div className="border-r border-border pr-4 last:border-0">
              <div className="text-sm text-muted-foreground">Started</div>
              <div className="text-md font-medium mt-1">{formatDate(data.startDate)}</div>
            </div>
            <div>
              <div className="text-sm text-muted-foreground">Terms</div>
              <div className="text-md font-medium mt-1">Net {data.netTerms} days</div>
            </div>
          </div>
        </div>

        {/* Price Components */}
        {details.priceComponents && details.priceComponents.length > 0 && (
          <div className="bg-card rounded-lg   shadow-sm mb-6">
            <div className="p-4 border-b border-border">
              <h3 className="text-md font-medium text-foreground">Pricing</h3>
            </div>
            <div className="overflow-hidden">
              <table className="w-full">
                <thead className="bg-muted/40">
                <tr>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Name
                  </th>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Billing Period
                  </th>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Fee Type
                  </th>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    price
                  </th>
                </tr>
                </thead>
                <tbody>
                {details.priceComponents.map((component, index) => (
                  <tr
                    key={index}
                    className={
                      index % 2 === 0 ? 'bg-card' : 'bg-muted/10 border-t border-b border-border'
                    }
                  >
                    <td className="px-4 py-3 text-sm font-medium text-foreground">
                      {component.name}
                    </td>
                    <td className="px-4 py-3 text-sm text-muted-foreground">
                      {formatBillingPeriod(component.period)}
                    </td>
                    <td className="px-4 py-3 text-sm text-muted-foreground">
                      {formatFeeType(component.fee)}
                    </td>
                    <td className="px-4 py-3 text-sm text-muted-foreground">
                      <SubscriptionFeeDetail fee={component.fee} currency={data.currency}/>
                    </td>
                  </tr>
                ))}
                </tbody>
              </table>
            </div>
          </div>
        )}

        {/* Add-ons */}
        {details.addOns && details.addOns.length > 0 && (
          <div className="bg-card rounded-lg  shadow-sm mb-6">
            <div className="p-4 border-b border-border">
              <h3 className="text-lg font-medium text-foreground">Add-ons</h3>
            </div>
            <div className="overflow-hidden">
              <table className="w-full">
                <thead className="bg-muted/40">
                <tr>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Name
                  </th>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Billing Period
                  </th>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Fee Type
                  </th>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    ID
                  </th>
                </tr>
                </thead>
                <tbody>
                {details.addOns.map((addon, index) => (
                  <tr
                    key={index}
                    className={
                      index % 2 === 0 ? 'bg-card' : 'bg-muted/10 border-t border-b border-border'
                    }
                  >
                    <td className="px-4 py-3 text-sm font-medium text-foreground">
                      {addon.name}
                    </td>
                    <td className="px-4 py-3 text-sm text-muted-foreground">
                      {formatBillingPeriod(addon.period)}
                    </td>
                    <td className="px-4 py-3 text-sm text-muted-foreground">
                      {formatFeeType(addon.fee)}
                    </td>
                    <td className="px-4 py-3 text-sm text-muted-foreground">{addon.addOnId}</td>
                  </tr>
                ))}
                </tbody>
              </table>
            </div>
          </div>
        )}

        {/* Billable Metrics */}
        {details.metrics && details.metrics.length > 0 && (
          <div className="bg-card rounded-lg   shadow-sm mb-6">
            <div className="p-4 border-b border-border">
              <h3 className="text-lg font-medium text-foreground">Billable Metrics</h3>
            </div>
            <div className="overflow-hidden">
              <table className="w-full">
                <thead className="bg-muted/40">
                <tr>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Name
                  </th>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Alias
                  </th>
                </tr>
                </thead>
                <tbody>
                {details.metrics.map((metric, index) => (
                  <tr
                    key={index}
                    className={
                      index % 2 === 0 ? 'bg-card' : 'bg-muted/10 border-t border-b border-border'
                    }
                  >
                    <td className="px-4 py-3 text-sm font-medium text-brand hover:underline">
                      <Link to={`${basePath}/metrics/${metric.id}`}>{metric.name}</Link>
                    </td>
                    <td className="px-4 py-3 text-sm text-muted-foreground">{metric.alias}</td>
                  </tr>
                ))}
                </tbody>
              </table>
            </div>
          </div>
        )}

        {/* Applied Coupons */}
        {details.appliedCoupons && details.appliedCoupons.length > 0 && (
          <div className="bg-card rounded-lg   shadow-sm mb-6">
            <div className="p-4 border-b border-border">
              <h3 className="text-lg font-medium text-foreground">Applied Coupons</h3>
            </div>
            <div className="overflow-hidden">
              <table className="w-full">
                <thead className="bg-muted/40">
                <tr>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Coupon
                  </th>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    Type
                  </th>
                  <th
                    className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    ID
                  </th>
                </tr>
                </thead>
                <tbody>
                {details.appliedCoupons.map((coupon, index) => (
                  <tr
                    key={index}
                    className={
                      index % 2 === 0 ? 'bg-card' : 'bg-muted/10 border-t border-b border-border'
                    }
                  >
                    <td className="px-4 py-3 text-sm font-medium text-foreground">
                      {coupon.coupon?.code || 'N/A'}
                    </td>
                    <td className="px-4 py-3 text-sm text-muted-foreground">
                      {coupon.appliedCoupon?.appliedAmount || 'N/A'}
                    </td>
                    <td className="px-4 py-3 text-sm text-muted-foreground">
                      {coupon.coupon?.id || 'N/A'}
                    </td>
                  </tr>
                ))}
                </tbody>
              </table>
            </div>
          </div>
        )}

        <div className="bg-card rounded-lg border border-border shadow-sm mb-6">
          <div className="p-4 border-b border-border flex items-center justify-between">
            <h3 className="text-md font-medium text-foreground">Invoices</h3>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => invoicesRefetch?.()}
              disabled={!invoicesRefetch || invoicesIsFetching}
              className="h-7 w-7 p-0"
            >
              <RefreshCw
                className={`h-3.5 w-3.5 ${invoicesIsFetching ? 'animate-spin' : ''}`}
              />
            </Button>
          </div>
          <div className="p-4 text-sm overflow-hidden text-muted-foreground">
            <SubscriptionInvoicesCard
              subscriptionId={data.localId}
              onRefetchChange={(refetch, isFetching) => {
                setInvoicesRefetch(() => refetch)
                setInvoicesIsFetching(isFetching)
              }}
            />
          </div>
        </div>
      </div>

      {/* Sidebar */}
      <div className="w-80 p-6 border-l border-border">
        <DetailSection title="Subscription Details">
          <DetailRow label="ID" value={data.localId}/>
          <DetailRow label="Version" value={data.version}/>
          <DetailRow label="Status" value={<StatusBadge status={data.status}/>}/>
          <DetailRow label="Currency" value={data.currency}/>
        </DetailSection>

        <DetailSection title="Customer">
          <DetailRow
            label="Customer"
            value={data.customerName}
            link={`${basePath}/customers/${data.customerId}`}
          />
          {data.customerAlias && <DetailRow label="Alias" value={data.customerAlias}/>}
        </DetailSection>

        <DetailSection title="Billing Information">
          <DetailRow label="Billing Day" value={data.billingDayAnchor}/>
          <DetailRow label="Net Terms" value={`${data.netTerms} days`}/>
          <DetailRow label="Auto-advance invoices" value={data.autoAdvanceInvoices ? 'Yes' : 'No'}/>
          <DetailRow label="Charge automatically" value={data.chargeAutomatically ? 'Yes' : 'No'}/>
          {data.invoiceMemo && <DetailRow label="Invoice Memo" value={data.invoiceMemo}/>}
          {data.invoiceThreshold && (
            <DetailRow label="Invoice Threshold" value={data.invoiceThreshold}/>
          )}
          {data.purchaseOrder && (
            <DetailRow label="Purchase Order" value={data.purchaseOrder}/>
          )}
        </DetailSection>

        <DetailSection title="Integrations">
          {hubspotConnMeta?.externalId ? (
            <DetailRow
              label="Hubspot ID"
              value={hubspotConnMeta?.externalId}
              externalLink={`https://app.hubspot.com/contacts/${hubspotConnMeta?.externalCompanyId}/deal/${hubspotConnMeta?.externalId}`}
            />
          ) : (
            <span className="text-xs font-medium text-muted-foreground">-</span>
          )}
        </DetailSection>

        <DetailSection title="Timeline">
          <DetailRow label="Created At" value={formatDate(data.createdAt)}/>
          <DetailRow label="Start Date" value={formatDate(data.startDate)}/>
          {data.billingStartDate && (
            <DetailRow label="Billing Start" value={formatDate(data.billingStartDate)}/>
          )}
          {data.activatedAt && (
            <DetailRow label="Activated At" value={formatDate(data.activatedAt)}/>
          )}
          {data.endDate && <DetailRow label="End Date" value={formatDate(data.endDate)}/>}
          {/* {data.canceledAt && <DetailRow label="Canceled At" value={formatDate(data.canceledAt)} />}
          {data.cancellationReason && <DetailRow label="Reason" value={data.cancellationReason} />} */}
        </DetailSection>

        {data.trialDuration && (
          <DetailSection title="Trial Information">
            <DetailRow label="Trial Duration" value={`${data.trialDuration} days`}/>
            {data.status === SubscriptionStatus.TRIALING && (
              <DetailRow label="Trial Status" value={<StatusBadge status={data.status}/>}/>
            )}
          </DetailSection>
        )}
      </div>
    </div>
  )
}

/**
 * Enhanced display component for subscription fees (for tooltips or expanded views)
 *
 * @param fee The subscription fee to display
 * @returns JSX for a detailed view of the fee
 */
export const SubscriptionFeeDetail = ({
  fee,
  currency,
}: {
  fee: SubscriptionFee | undefined
  currency: string
}) => {
  if (!fee || !fee.fee.case) {
    return <span className="text-muted-foreground">No fee information</span>
  }

  const formatted = formatSubscriptionFee(fee, currency)

  return (
    <div className="space-y-1">
      <div className="flex justify-between">
        <span>{formatted.details}</span>
      </div>
      <div className="flex justify-between">
        <span className="font-medium text-foreground">{formatted.amount}</span>
      </div>

      {/* Conditionally render specific details based on fee type */}
      {fee.fee.case === 'slot' && (
        <div className="text-xs text-muted-foreground mt-1">
          <div>Upgrade: {getUpgradePolicyText(fee.fee.value.upgradePolicy)}</div>
          <div>Downgrade: {getDowngradePolicyText(fee.fee.value.downgradePolicy)}</div>
        </div>
      )}
    </div>
  )
}

// Helper to get human-readable upgrade policy text
const getUpgradePolicyText = (policy: number): string => {
  switch (policy) {
    case 0:
      return 'Prorated'
    case 1:
      return 'Full immediately'
    default:
      return 'Unknown'
  }
}

// Helper to get human-readable downgrade policy text
const getDowngradePolicyText = (policy: number): string => {
  switch (policy) {
    case 0:
      return 'At end of period'
    case 1:
      return 'Prorated refund'
    case 2:
      return 'Full refund immediately'
    default:
      return 'Unknown'
  }
}

export default Subscription
