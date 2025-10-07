import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  cn,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Flex,
  Separator,
  Skeleton,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import {
  BanIcon,
  CheckCircleIcon,
  ChevronDown,
  Download,
  FileX2Icon,
  FolderSyncIcon,
  RefreshCcw,
  Trash2,
} from 'lucide-react'
import { Fragment, useEffect, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { AddressLinesCompact } from '@/features/customers/cards/address/AddressCard'
import { PaymentStatusBadge } from '@/features/invoices/PaymentStatusBadge'
import { TransactionList } from '@/features/invoices/TransactionList'
import {
  IntegrationType,
  SyncInvoiceModal,
} from '@/features/settings/integrations/SyncInvoiceModal'
import { getCountryName } from '@/features/settings/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { getLatestConnMeta } from '@/pages/tenants/utils'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import {
  deleteInvoice,
  finalizeInvoice,
  getInvoice,
  listInvoices,
  markInvoiceAsUncollectible,
  previewInvoiceSvg,
  refreshInvoiceData,
  voidInvoice,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { DetailedInvoice, InvoiceStatus, LineItem } from '@/rpc/api/invoices/v1/models_pb'
import { parseAndFormatDate, parseAndFormatDateOptional } from '@/utils/date'
import { formatCurrency, formatCurrencyNoRounding, formatUsage } from '@/utils/numbers'
import { useTypedParams } from '@/utils/params'

import { InvoiceConfirmationDialog } from './InvoiceConfirmationDialog'

export const Invoice = () => {
  const { invoiceId } = useTypedParams<{ invoiceId: string }>()
  const invoiceQuery = useQuery(
    getInvoice,
    {
      id: invoiceId ?? '',
    },
    { enabled: Boolean(invoiceId) }
  )

  const data = invoiceQuery.data?.invoice
  const isLoading = invoiceQuery.isLoading

  return (
    <Fragment>
      <Flex direction="column" className="h-full">
        {isLoading || !data ? (
          <>
            <Skeleton height={16} width={50} />
            <Skeleton height={44} />
          </>
        ) : (
          <InvoiceView invoice={data} invoiceId={invoiceId ?? ''} />
        )}
      </Flex>
    </Fragment>
  )
}

interface Props {
  invoice: DetailedInvoice
}

// Function to resize SVG content by manipulating viewBox and dimensions
export const resizeSvgContent = (html: string, scaleFactor: number = 0.8): string => {
  // Create a temporary DOM parser to work with the HTML
  const parser = new DOMParser()
  const doc = parser.parseFromString(html, 'text/html')
  const svgElement = doc.querySelector('svg')

  if (!svgElement) {
    console.warn('No SVG element found in the provided HTML.')
    return html
  }

  // Get current dimensions
  const width = svgElement.getAttribute('width')
  const height = svgElement.getAttribute('height')

  // Scale dimensions if they exist, removing units like 'pt', 'px', etc.
  if (width && !width.includes('%')) {
    const numWidth = parseFloat(width)
    if (!isNaN(numWidth)) {
      // Remove units and set as unitless number (defaults to pixels)
      svgElement.setAttribute('width', (numWidth * scaleFactor).toString())
    }
  }

  if (height && !height.includes('%')) {
    const numHeight = parseFloat(height)
    if (!isNaN(numHeight)) {
      // Remove units and set as unitless number (defaults to pixels)
      svgElement.setAttribute('height', (numHeight * scaleFactor).toString())
    }
  }
  return doc.documentElement.outerHTML
}

// Component for inline invoice preview with direct SVG rendering
const InvoicePreviewFrame: React.FC<{ invoiceId: string; invoice: DetailedInvoice }> = ({
  invoiceId,
  invoice,
}) => {
  const previewQuery = useQuery(previewInvoiceSvg, { id: invoiceId }, { refetchOnMount: 'always' })

  if (previewQuery.isLoading) {
    return (
      <div className="h-full flex items-center justify-center bg-white">
        <div className="text-sm text-muted-foreground">Loading preview...</div>
      </div>
    )
  }

  if (previewQuery.error) {
    return (
      <div className="h-full flex items-center justify-center bg-white">
        <div className="text-sm text-muted-foreground">Failed to load preview</div>
      </div>
    )
  }

  // Extract and resize SVG content
  const svgContents =
    previewQuery.data?.svgs.map(svg => {
      const scaledHtml = svg ? resizeSvgContent(svg, 1) : ''

      // Extract just the SVG from the HTML
      const parser = new DOMParser()
      const doc = parser.parseFromString(scaledHtml, 'text/html')
      const svgElement = doc.querySelector('svg')
      return svgElement?.outerHTML || ''
    }) ?? []

  return (
    <div className="w-full h-full   flex flex-col">
      <div
        className="flex flex-col items-center justify-center gap-5 bg-gray-100 py-10 relative"
        style={{ minHeight: 'fit-content' }}
      >
        {svgContents.map((svgContent, i) => (
          <div
            className="  bg-white"
            key={`svg-${i}`}
            style={{
              boxShadow: '0px 4px 12px rgba(89, 85, 101, .2)',
            }}
            dangerouslySetInnerHTML={{ __html: svgContent }}
          />
        ))}

        {/* Floating Download Button */}
        {invoice.pdfDocumentId && (
          <div className="absolute top-16 right-16">
            <Button asChild variant="flat" size="icon" className="shadow-lg">
              <a
                href={
                  invoice.pdfDocumentId && invoice.documentSharingKey
                    ? `${env.meteroidRestApiUri}/files/v1/invoice/pdf/${invoice.localId}?token=${invoice.documentSharingKey}`
                    : '#'
                }
                download={`invoice_${invoice.invoiceNumber}.pdf`}
                target="_blank"
                rel="noreferrer"
                className="flex items-center gap-2"
              >
                <Download size="16" />
              </a>
            </Button>
          </div>
        )}
      </div>
    </div>
  )
}

export const InvoiceView: React.FC<Props & { invoiceId: string }> = ({ invoice, invoiceId }) => {
  const basePath = useBasePath()
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const refresh = useMutation(refreshInvoiceData, {
    onSuccess: async res => {
      await queryClient.setQueryData(
        createConnectQueryKey(getInvoice, { id: invoice?.id ?? '' }),
        res
      )
    },
  })

  const deleteInvoiceMutation = useMutation(deleteInvoice, {
    onSuccess: async () => {
      // Show success toast
      toast.success(`Invoice deleted`)
      // Invalidate the list invoices query to refresh the list
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listInvoices, {}),
      })
      // Navigate back to the invoices list after successful deletion
      navigate(`${basePath}/invoices`)
    },
    onError: error => {
      toast.error(`Failed to delete invoice: ${error.message}`)
    },
  })

  const finalizeInvoiceMutation = useMutation(finalizeInvoice, {
    onSuccess: async () => {
      toast.success('Invoice finalized')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getInvoice, { id: invoice?.id ?? '' }),
      })
    },
    onError: error => {
      toast.error(`Failed to finalize invoice: ${error.message}`)
    },
  })

  const voidInvoiceMutation = useMutation(voidInvoice, {
    onSuccess: async () => {
      toast.success('Invoice voided')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getInvoice, { id: invoice?.id ?? '' }),
      })
    },
    onError: error => {
      toast.error(`Failed to void invoice: ${error.message}`)
    },
  })

  const markInvoiceAsUncollectibleMutation = useMutation(markInvoiceAsUncollectible, {
    onSuccess: async () => {
      toast.success('Invoice marked as uncollectible')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getInvoice, { id: invoice?.id ?? '' }),
      })
    },
    onError: error => {
      toast.error(`Failed to mark invoice as uncollectible: ${error.message}`)
    },
  })

  const doRefresh = () => refresh.mutateAsync({ id: invoice?.id ?? '' })

  const handleDeleteConfirm = () => {
    setShowDeleteConfirmation(false)
    deleteInvoiceMutation.mutateAsync({ id: invoice?.id ?? '' })
  }

  const handleFinalizeConfirm = () => {
    setShowFinalizeConfirmation(false)
    finalizeInvoiceMutation.mutateAsync({ id: invoice?.id ?? '' })
  }

  const handleVoidConfirm = () => {
    setShowVoidConfirmation(false)
    voidInvoiceMutation.mutateAsync({ id: invoice?.id ?? '' })
  }

  const handleMarkAsUncollectibleConfirm = () => {
    setShowMarkAsUncollectibleConfirmation(false)
    markInvoiceAsUncollectibleMutation.mutateAsync({ id: invoice?.id ?? '' })
  }

  const isDraft = invoice && invoice.status === InvoiceStatus.DRAFT
  const isFinalized = invoice && invoice.status === InvoiceStatus.FINALIZED

  const pdf_url =
    invoice.documentSharingKey &&
    `${env.meteroidRestApiUri}/files/v1/invoice/pdf/${invoice.localId}?token=${invoice.documentSharingKey}`

  const connectorsQuery = useQuery(listConnectors, {})
  const connectorsData = connectorsQuery.data?.connectors ?? []

  const [showSyncPennylaneModal, setShowSyncPennylaneModal] = useState(false)
  const [showDeleteConfirmation, setShowDeleteConfirmation] = useState(false)
  const [showFinalizeConfirmation, setShowFinalizeConfirmation] = useState(false)
  const [showVoidConfirmation, setShowVoidConfirmation] = useState(false)
  const [showMarkAsUncollectibleConfirmation, setShowMarkAsUncollectibleConfirmation] =
    useState(false)
  const isPennylaneConnected = connectorsData.some(
    connector => connector.provider === ConnectorProviderEnum.PENNYLANE
  )

  const canRefresh = isDraft && !invoice.manual
  const canDelete = isDraft
  const canFinalize = isDraft
  const canSendToPennylane = isDraft && isPennylaneConnected

  const canVoid = isFinalized
  const canMarkAsUncollectible = isFinalized

  useEffect(() => {
    if (canRefresh) {
      doRefresh()
    }
  }, [])

  return (
    <Flex className="h-full">
      {showSyncPennylaneModal && (
        <SyncInvoiceModal
          invoiceNumber={invoice.invoiceNumber}
          id={invoice.id}
          integrationType={IntegrationType.Pennylane}
          onClose={() => setShowSyncPennylaneModal(false)}
        />
      )}

      <InvoiceConfirmationDialog
        open={showDeleteConfirmation}
        onOpenChange={setShowDeleteConfirmation}
        onConfirm={handleDeleteConfirm}
        icon={Trash2}
        title="Delete invoice"
        description="Are you sure you want to delete this draft invoice? This action cannot be undone."
      />

      <InvoiceConfirmationDialog
        open={showFinalizeConfirmation}
        onOpenChange={setShowFinalizeConfirmation}
        onConfirm={handleFinalizeConfirm}
        icon={CheckCircleIcon}
        title="Finalize & Send invoice"
        description="Finalize this invoice and send it to the customer. Once finalized, the invoice cannot be edited."
        invoiceNumber={invoice.invoiceNumber}
      />

      <InvoiceConfirmationDialog
        open={showVoidConfirmation}
        onOpenChange={setShowVoidConfirmation}
        onConfirm={handleVoidConfirm}
        icon={BanIcon}
        title="Void invoice"
        description="Are you sure you want to void this invoice? This action cannot be undone."
        invoiceNumber={invoice.invoiceNumber}
      />

      <InvoiceConfirmationDialog
        open={showMarkAsUncollectibleConfirmation}
        onOpenChange={setShowMarkAsUncollectibleConfirmation}
        onConfirm={handleMarkAsUncollectibleConfirm}
        icon={FileX2Icon}
        title="Mark invoice as Uncollectible"
        description="Are you sure you want to mark this invoice as uncollectible? This action cannot be undone."
        invoiceNumber={invoice.invoiceNumber}
      />

      {/* Left Panel - Invoice Details */}
      <Flex direction="column" className="w-1/3 border-r border-border">
        {/* Fixed Header - Always Visible */}
        <Flex direction="column" className="gap-2 p-6 border-b border-border">
          <div className="flex justify-between items-center">
            <div className="flex items-center gap-3">
              <InvoiceStatusBadge status={invoice.status} />
              <div className="text-lg font-medium">Invoice {invoice.invoiceNumber}</div>
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary" size="sm" hasIcon>
                  Actions
                  <ChevronDown className="w-4 h-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={doRefresh} disabled={!canRefresh || refresh.isPending}>
                  <RefreshCcw
                    size="16"
                    className={cn(refresh.isPending && 'animate-spin', 'mr-2')}
                  />
                  Refresh
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={!canFinalize}
                  onClick={() => setShowFinalizeConfirmation(true)}
                >
                  <CheckCircleIcon size="16" className="mr-2" />
                  Finalize & Send
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={!canSendToPennylane}
                  onClick={() => setShowSyncPennylaneModal(true)}
                >
                  <FolderSyncIcon size="16" className="mr-2" />
                  Sync to Pennylane
                </DropdownMenuItem>
                <DropdownMenuItem disabled={!invoice.pdfDocumentId}>
                  <a
                    href={invoice.pdfDocumentId ? pdf_url : '#'}
                    download={`invoice_${invoice.invoiceNumber}.pdf`}
                    target="_blank"
                    rel="noreferrer"
                    className="flex items-center gap-2"
                  >
                    <Download size="16" />
                    Download PDF
                  </a>
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={!canDelete}
                  onClick={() => setShowDeleteConfirmation(true)}
                  className="text-destructive focus:text-destructive"
                >
                  <Trash2 size="16" className="mr-2" />
                  Delete
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={!canVoid}
                  onClick={() => setShowVoidConfirmation(true)}
                  className="text-destructive focus:text-destructive"
                >
                  <BanIcon size="16" className="mr-2" />
                  Void
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={!canMarkAsUncollectible}
                  onClick={() => setShowMarkAsUncollectibleConfirmation(true)}
                  className="text-destructive focus:text-destructive"
                >
                  <FileX2Icon size="16" className="mr-2" />
                  Mark As Uncollectible
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>

          <div className="text-3xl font-bold">
            {formatCurrency(Number(invoice.total) || 0, invoice.currency)}
          </div>
        </Flex>

        {/* Scrollable Content */}
        <div className="flex-1 overflow-auto">
          <Flex direction="column" className="gap-2 p-6">
            <FlexDetails title="Invoice number" value={invoice.invoiceNumber} />

            <FlexDetails
              title="Plan"
              value={
                <Link
                  to={`${basePath}/plan-version/${invoice.planVersionId}`}
                  className="text-[13px] text-brand hover:underline"
                >
                  {invoice.planName}
                </Link>
              }
            />
            <FlexDetails title="Invoice date" value={parseAndFormatDate(invoice.invoiceDate)} />
            <FlexDetails title="Due date" value={parseAndFormatDateOptional(invoice.dueAt)} />
            {invoice.purchaseOrder && (
              <FlexDetails title="Purchase order" value={invoice.purchaseOrder} />
            )}
            <FlexDetails title="Currency" value={invoice.currency} />
          </Flex>

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Customer</div>
            <FlexDetails
              title="Customer"
              value={invoice.customerDetails?.name}
              link={`${basePath}/customers/${invoice.customerId}`}
            />
            <FlexDetails title="Email" value={invoice.customerDetails?.email} />
            {invoice.customerDetails?.billingAddress && (
              <>
                <FlexDetails
                  title="Address"
                  value={
                    <AddressLinesCompact
                      address={invoice.customerDetails.billingAddress}
                      className="text-right"
                    />
                  }
                />
                <FlexDetails
                  title="Country"
                  value={
                    invoice.customerDetails.billingAddress.country &&
                    getCountryName(invoice.customerDetails.billingAddress.country)
                  }
                />
              </>
            )}
            {invoice.customerDetails?.vatNumber && (
              <FlexDetails title="VAT Number" value={invoice.customerDetails.vatNumber} />
            )}
          </Flex>

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Line Items</div>
            <InvoiceLineItems items={invoice.lineItems} invoice={invoice} />
            <div className="mt-4 pt-4 border-t">
              <InvoiceSummaryLines invoice={invoice} />
            </div>
          </Flex>

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Payment Information</div>
            <FlexDetails
              title="Payment Status"
              value={<PaymentStatusBadge status={invoice.paymentStatus} />}
            />
            <FlexDetails
              title="Amount Due"
              value={formatCurrency(Number(invoice.amountDue) || 0, invoice.currency)}
            />
            {invoice.transactions && invoice.transactions.length > 0 && (
              <div className="mt-4">
                <TransactionList
                  transactions={invoice.transactions}
                  currency={invoice.currency}
                  isLoading={false}
                />
              </div>
            )}
          </Flex>

          {invoice.memo && (
            <>
              <Separator className="-my-3" />
              <Flex direction="column" className="gap-2 p-6">
                <div className="text-[15px] font-medium">Memo</div>
                <div className="text-[13px] text-muted-foreground whitespace-pre-line">
                  {invoice.memo}
                </div>
              </Flex>
            </>
          )}

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Timeline</div>
            <div className="space-y-2">
              <div className="flex items-start gap-2">
                <div className="w-1.5 h-1.5 rounded-full bg-muted-foreground mt-1.5 flex-shrink-0"></div>
                <div>
                  <div className="text-[13px] font-medium">Invoice Created</div>
                  <div className="text-[11px] text-muted-foreground">
                    {parseAndFormatDate(invoice.createdAt)}
                  </div>
                </div>
              </div>
              {invoice.finalizedAt && (
                <div className="flex items-start gap-2">
                  <div className="w-1.5 h-1.5 rounded-full bg-success mt-1.5 flex-shrink-0"></div>
                  <div>
                    <div className="text-[13px] font-medium">Invoice Finalized</div>
                    <div className="text-[11px] text-muted-foreground">
                      {parseAndFormatDate(invoice.finalizedAt)}
                    </div>
                  </div>
                </div>
              )}
              {invoice.voidedAt && (
                <div className="flex items-start gap-2">
                  <div className="w-1.5 h-1.5 rounded-full bg-red-500 mt-1.5 flex-shrink-0"></div>
                  <div>
                    <div className="text-[13px] font-medium">Invoice Voided</div>
                    <div className="text-[11px] text-muted-foreground">
                      {parseAndFormatDate(invoice.voidedAt)}
                    </div>
                  </div>
                </div>
              )}
              {invoice.markedAsUncollectibleAt && (
                <div className="flex items-start gap-2">
                  <div className="w-1.5 h-1.5 rounded-full bg-warning mt-1.5 flex-shrink-0"></div>
                  <div>
                    <div className="text-[13px] font-medium">Invoice marked as Uncollectible</div>
                    <div className="text-[11px] text-muted-foreground">
                      {parseAndFormatDate(invoice.markedAsUncollectibleAt)}
                    </div>
                  </div>
                </div>
              )}
            </div>
          </Flex>

          {getLatestConnMeta(invoice.connectionMetadata?.pennylane)?.externalId && (
            <>
              <Separator className="-my-3" />
              <Flex direction="column" className="gap-2 p-6">
                <div className="text-[15px] font-medium">Integrations</div>
                <FlexDetails
                  title="Pennylane ID"
                  value={getLatestConnMeta(invoice.connectionMetadata?.pennylane)?.externalId}
                  externalLink={`https://app.pennylane.com/companies/${getLatestConnMeta(invoice.connectionMetadata?.pennylane)?.externalCompanyId}/clients/customer_invoices?invoice_id=${getLatestConnMeta(invoice.connectionMetadata?.pennylane)?.externalId}`}
                />
              </Flex>
            </>
          )}
        </div>
      </Flex>

      {/* Right Panel - Invoice Preview */}
      <div className="w-2/3 flex flex-col">
        <div className="flex-1 overflow-auto p-6">
          <InvoicePreviewFrame invoiceId={invoiceId} invoice={invoice} />
        </div>
      </div>
    </Flex>
  )
}

export const InvoiceStatusBadge = ({ status }: { status: InvoiceStatus }) => {
  const getBadge = () => {
    switch (status) {
      case InvoiceStatus.DRAFT:
        return <Badge variant="primary">Draft</Badge>
      case InvoiceStatus.UNCOLLECTIBLE:
        return <Badge variant="warning">Uncollectible</Badge>
      case InvoiceStatus.FINALIZED:
        return <Badge variant="success">Finalized</Badge>
      case InvoiceStatus.VOID:
        return <Badge variant="outline">Void</Badge>
      default:
        return null
    }
  }

  return getBadge()
}

export const InvoiceSummaryLines: React.FC<{ invoice: DetailedInvoice }> = ({ invoice }) => {
  const subtotal = Number(invoice.subtotal) || 0
  const taxAmount = Number(invoice.taxAmount) || 0
  const total = Number(invoice.total) || 0

  return (
    <div className="space-y-1">
      <FlexDetails title="Subtotal" value={formatCurrency(subtotal, invoice.currency)} />

      {invoice.couponLineItems.map(c => {
        const couponTotal = Number(c.total) || 0
        return (
          <FlexDetails
            key={c.name}
            title={c.name}
            value={`-${formatCurrency(couponTotal, invoice.currency)}`}
          />
        )
      })}

      {invoice.taxBreakdown && invoice.taxBreakdown.length > 0
        ? invoice.taxBreakdown.map(tax => {
            const taxRate = parseFloat(tax.taxRate) * 100 || 0
            const taxAmountValue = Number(tax.amount) || 0
            // Only show tax breakdown if rate is greater than 0
            if (taxRate > 0) {
              return (
                <FlexDetails
                  key={tax.name}
                  title={`${tax.name} (${taxRate}%)`}
                  value={formatCurrency(taxAmountValue, invoice.currency)}
                />
              )
            }
            return null
          })
        : taxAmount > 0 && (
            <FlexDetails title="Tax" value={formatCurrency(taxAmount, invoice.currency)} />
          )}

      <div className="pt-2 border-t">
        <FlexDetails
          title={<span className="font-semibold">Total</span>}
          value={
            <span className="font-semibold text-[15px]">
              {formatCurrency(total, invoice.currency)}
            </span>
          }
        />
      </div>
    </div>
  )
}

export const InvoiceLineItems: React.FC<{ items: LineItem[]; invoice: DetailedInvoice }> = ({
  items,
  invoice,
}) => {
  return (
    <div className="space-y-2">
      {items
        .sort((a, b) => a.name.localeCompare(b.name))
        .map(item => {
          return <InvoiceLineItemCard key={item.id} line_item={item} invoice={invoice} />
        })}
    </div>
  )
}

const InvoiceLineItemCard: React.FC<{
  line_item: LineItem
  invoice: DetailedInvoice
}> = ({ line_item, invoice }) => {
  const [isExpanded, setIsExpanded] = useState(false)
  const hasSubItems = line_item.subLineItems && line_item.subLineItems.length > 0

  return (
    <div className="py-2">
      <div
        className={cn('flex justify-between items-start', hasSubItems && 'cursor-pointer')}
        onClick={() => hasSubItems && setIsExpanded(!isExpanded)}
      >
        <div className="flex-1">
          <div className="flex items-center gap-2">
            {hasSubItems && (
              <ChevronDown
                size={12}
                className={cn(
                  'text-muted-foreground transition-transform',
                  isExpanded && 'rotate-180'
                )}
              />
            )}
            <div className="text-[13px] font-medium">{line_item.name}</div>
          </div>
          {line_item.startDate && line_item.endDate && (
            <div className="text-[11px] text-muted-foreground mt-1 ml-4">
              {parseAndFormatDate(line_item.startDate)} → {parseAndFormatDate(line_item.endDate)}
            </div>
          )}
        </div>
        <div className="text-right">
          {line_item.quantity && line_item.unitPrice && (
            <div className="text-[11px] text-muted-foreground">
              {formatUsage(parseFloat(line_item.quantity))} ×{' '}
              {formatCurrencyNoRounding(line_item.unitPrice, invoice.currency)}
            </div>
          )}
          <div className="text-[13px] font-medium">
            {formatCurrency(line_item.subtotal, invoice.currency)}
          </div>
        </div>
      </div>

      {isExpanded && hasSubItems && (
        <div className="mt-2 ml-4 pt-2 border-t space-y-1">
          {line_item.subLineItems.map(subItem => (
            <div key={subItem.id} className="flex justify-between items-center py-1">
              <span className="text-[11px] text-muted-foreground">{subItem.name}</span>
              <span className="text-[11px]">{formatCurrency(subItem.total, invoice.currency)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// FlexDetails component matching the customer page pattern
const FlexDetails = ({
  title,
  value,
  externalLink,
  link,
}: {
  title: string | React.ReactNode
  value?: string | React.ReactNode
  externalLink?: string
  link?: string
}) => (
  <Flex align="start" justify="between">
    <div className="text-[13px] text-muted-foreground">{title}</div>
    {externalLink ? (
      <a href={externalLink} target="_blank" rel="noopener noreferrer">
        <div className="text-[13px] text-brand hover:underline">{value ?? 'N/A'}</div>
      </a>
    ) : link ? (
      <Link to={link}>
        <div className="text-[13px] text-brand hover:underline">{value ?? 'N/A'}</div>
      </Link>
    ) : (
      <div className="text-[13px]">{value ?? 'N/A'}</div>
    )}
  </Flex>
)
