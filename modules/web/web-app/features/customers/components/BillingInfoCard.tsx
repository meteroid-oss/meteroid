import { timestampDate } from '@bufbuild/protobuf/wkt'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Badge, Button, Card, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, BadgeCheck, Edit2, Loader2, RefreshCw, XCircle } from 'lucide-react'
import { ReactNode } from 'react'
import { toast } from 'sonner'

import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  getCustomerById,
  refreshVatValidation,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import {
  CustomerSchema,
  CustomerTaxStatus,
  VatNumberValidationStatus,
} from '@/rpc/api/customers/v1/models_pb'
import { listInvoicingEntities } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { EInvoicingFormat } from '@/rpc/api/invoicingentities/v1/models_pb'

import type { MessageInitShape } from '@bufbuild/protobuf'
import type { Timestamp } from '@bufbuild/protobuf/wkt'
import type { LucideIcon } from 'lucide-react'

interface BillingInfoCardProps {
  customer: MessageInitShape<typeof CustomerSchema>
  onEdit: () => void
  title?: string
  actions?: ReactNode // Additional actions next to edit button
  cardVariant?: 'default' | 'accent' | 'accent2'
}

type VatBadgeConfig = {
  label: string
  variant: 'success' | 'destructive' | 'warning' | 'ghost'
  icon: LucideIcon
  description: string
}

const VAT_BADGES: Partial<Record<VatNumberValidationStatus, VatBadgeConfig>> = {
  [VatNumberValidationStatus.PENDING]: {
    label: 'Verifying…',
    variant: 'ghost',
    icon: Loader2,
    description: 'Verification against VIES is in progress.',
  },
  [VatNumberValidationStatus.VALID]: {
    label: 'VIES verified',
    variant: 'success',
    icon: BadgeCheck,
    description: 'This VAT number is registered in the EU VIES database.',
  },
  [VatNumberValidationStatus.INVALID]: {
    label: 'Not in VIES',
    variant: 'destructive',
    icon: XCircle,
    description: 'VIES has no active registration for this VAT number.',
  },
  [VatNumberValidationStatus.UNAVAILABLE]: {
    label: 'Unverified',
    variant: 'warning',
    icon: AlertTriangle,
    description: 'VIES could not be reached. The check retries automatically.',
  },
}

export const VatValidationBadge = ({
  customer,
}: {
  customer: BillingInfoCardProps['customer']
}) => {
  const queryClient = useQueryClient()
  const customerId = customer.id

  const refreshMutation = useMutation(refreshVatValidation, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getCustomerById,
          input: { id: customerId },
          cardinality: 'finite',
        }),
      })
    },
    onError: () => toast.error('Failed to start VAT number re-check'),
  })

  const status = customer.vatNumberValidationStatus
  const badge = status ? VAT_BADGES[status] : undefined
  if (!badge) return null

  const Icon = badge.icon
  const isPending = status === VatNumberValidationStatus.PENDING

  const checkedAt = customer.vatNumberCheckedAt
    ? timestampDate(customer.vatNumberCheckedAt as Timestamp).toLocaleDateString()
    : undefined

  return (
    <span className="inline-flex items-center gap-1 ml-1.5 align-middle">
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="inline-flex cursor-help">
            <Badge variant={badge.variant} size="sm">
              <Icon className={isPending ? 'animate-spin' : ''} />
              {badge.label}
            </Badge>
          </span>
        </TooltipTrigger>
        <TooltipContent className="max-w-xs space-y-1">
          <p>{badge.description}</p>
          {customer.vatNumberRegisteredName && (
            <p>
              <span className="opacity-60">Registered as </span>
              {customer.vatNumberRegisteredName}
            </p>
          )}
          {customer.vatNumberConsultationNumber && (
            <p>
              <span className="opacity-60">VIES ref </span>
              {customer.vatNumberConsultationNumber}
            </p>
          )}
          {checkedAt && <p className="opacity-60">Last checked {checkedAt}</p>}
        </TooltipContent>
      </Tooltip>
      {!isPending && customerId && (
        <Button
          variant="ghost"
          size="sm"
          className="p-0 h-auto text-muted-foreground"
          title="Re-check against VIES"
          disabled={refreshMutation.isPending}
          onClick={() => refreshMutation.mutate({ customerId })}
        >
          <RefreshCw size={12} className={refreshMutation.isPending ? 'animate-spin' : ''} />
        </Button>
      )}
    </span>
  )
}

export const BillingInfoCard = ({
  customer,
  onEdit,
  title = 'Billing information',
  actions,
  cardVariant = 'accent',
}: BillingInfoCardProps) => {
  // Whether the customer's invoicing entity issues e-invoices at all. Unknown while
  // loading, which reads as off: better no warning for a moment than a flash of one.
  const entitiesQuery = useQuery(listInvoicingEntities)
  const entity = entitiesQuery.data?.entities.find(e => e.id === customer.invoicingEntityId)
  const einvoicingOn =
    !!entity && entity.einvoicingFormat !== EInvoicingFormat.EINVOICING_FORMAT_UNSPECIFIED

  return (
    <>
      <div className="text-sm font-medium">{title}</div>
      <Card className="mb-8 px-6 py-4 mt-2 border-0" variant={cardVariant}>
        <div className="flex justify-between items-start mb-2">
          <div className="text-sm space-y-1">
            <div className="font-medium">{customer.name}</div>
            {customer.billingEmail && (
              <div className="text-muted-foreground">{customer.billingEmail}</div>
            )}
            {customer.billingAddress && (
              <div className="pt-0">
                {customer.billingAddress.line1}
                {customer.billingAddress.line2 && <span>, {customer.billingAddress.line2}</span>}
                {customer.billingAddress.line1 && <br />}
                {customer.billingAddress.city}
                {customer.billingAddress.state && (
                  <span>, {customer.billingAddress.state}</span>
                )}{' '}
                {customer.billingAddress.zipCode}
                <br />
                {customer.billingAddress.country && (
                  <span>
                    {getCountryFlagEmoji(customer.billingAddress.country)}{' '}
                    {getCountryName(customer.billingAddress.country)}
                  </span>
                )}
              </div>
            )}
            {customer.vatNumber && (
              <div className="pt-1">
                <span className="text-muted-foreground">Tax ID: </span>
                {customer.vatNumber}
                <VatValidationBadge customer={customer} />
              </div>
            )}
            {customer.legalNumber && (
              <div className="pt-1">
                <span className="text-muted-foreground">Reg. no: </span>
                {customer.legalNumber}
              </div>
            )}
            {/* On a reverse-charge invoice the buyer accounts for the tax, so EN 16931
                requires the buyer to be identifiable: BR-AE-02 accepts the tax ID *or*
                the registration number. With neither, the invoice is still valid and is
                still sent — it just cannot also be issued as an e-invoice, and by then
                it is finalized and immutable. Say so while it is still fixable — and only
                where it applies: an entity with e-invoicing off issues no e-invoice to miss. */}
            {einvoicingOn &&
              customer.taxStatus === CustomerTaxStatus.REVERSE_CHARGE &&
              !customer.vatNumber &&
              !customer.legalNumber && (
                <div className="mt-2 flex items-start gap-1.5 text-[13px] text-amber-700 dark:text-amber-500">
                  <AlertTriangle size={13} className="mt-0.5 shrink-0" />
                  <span>
                    Reverse-charge customers need a tax ID or a registration number, or their
                    invoices cannot be issued as e-invoices.
                  </span>
                </div>
              )}
          </div>
          <div className="flex items-center space-x-2">
            {actions}
            <Button variant="ghost" size="sm" className="p-0 h-auto" onClick={onEdit}>
              <Edit2 size={16} />
            </Button>
          </div>
        </div>
      </Card>
    </>
  )
}
