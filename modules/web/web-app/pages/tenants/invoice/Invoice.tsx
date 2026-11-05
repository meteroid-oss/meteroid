import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  cn,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Flex,
  Separator,
  Skeleton,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { formatDistanceToNowStrict } from 'date-fns'
import {
  BanIcon,
  CheckCircleIcon,
  CheckIcon,
  ChevronDown,
  CreditCard,
  Download,
  Edit,
  ExternalLink,
  FilePlus2,
  FileText,
  FileX2Icon,
  FolderSyncIcon,
  RefreshCcw,
  RotateCcw,
  TriangleAlertIcon,
  Trash2,
} from 'lucide-react'
import { Fragment, useEffect, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { EntityActivityTimeline } from '@/features/activity/EntityActivityTimeline'
import { CreditNoteStatusBadge } from '@/features/creditNotes'
import { AddressLinesCompact } from '@/features/customers/cards/address/AddressCard'
import { CustomPropertyValue } from '@/features/customproperties/CustomPropertyValue'
import { useCustomPropertyRows } from '@/features/customproperties/useCustomPropertyRows'
import { AddManualPaymentDialog } from '@/features/invoices/AddManualPaymentDialog'
import { CreateCreditNoteDialog } from '@/features/invoices/CreateCreditNoteDialog'
import { NamedCreditUnitUsageNote } from '@/features/invoices/CreditUnitUsageNote'
import { InvoiceStatusBadge } from '@/features/invoices/InvoiceStatusBadge'
import { MarkAsPaidDialog } from '@/features/invoices/MarkAsPaidDialog'
import { PaymentStatusBadge } from '@/features/invoices/PaymentStatusBadge'
import { TransactionList } from '@/features/invoices/TransactionList'
import {
  invoiceSummary,
  lineShownSubLines,
  lineShownSubtotal,
  lineShownUnitPrice,
} from '@/features/invoices/invoiceSummary'
import { packCreditText } from '@/features/invoices/packCredit'
import { useCreditUnitNames } from '@/features/invoices/useCreditUnitNames'
import { InvoiceRemindersCard } from '@/features/payment-reminders/InvoiceRemindersCard'
import {
  IntegrationType,
  SyncInvoiceModal,
} from '@/features/settings/integrations/SyncInvoiceModal'
import { getCountryName } from '@/features/settings/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import {
  formatCurrency,
  formatCurrencyNoRounding,
  formatUsage,
  rateToPercent,
} from '@/lib/utils/numbers'
import { resizeSvgContent } from '@/pages/tenants/invoice/utils'
import { getLatestConnMeta } from '@/pages/tenants/utils'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum, ConnectorStatusEnum } from '@/rpc/api/connectors/v1/models_pb'
import { listCreditNotesByInvoiceId } from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'
import { CreditNoteStatus, CreditType } from '@/rpc/api/creditnotes/v1/models_pb'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import {
  checkEinvoicingReadiness,
  createCorrectedInvoice,
  deleteInvoice,
  finalizeInvoice,
  generateInvoicePaymentToken,
  getInvoice,
  listInvoices,
  markInvoiceAsUncollectible,
  previewInvoiceSvg,
  refreshInvoiceData,
  regenerateInvoice,
  voidInvoice,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import {
  DetailedInvoice,
  EInvoicingStatus,
  InvoicePaymentStatus,
  InvoiceStatus,
  LineItem,
} from '@/rpc/api/invoices/v1/models_pb'
import {
  formatDateOnly,
  formatDateOnlyOptional,
  parseAndFormatDate,
  parseAndFormatDateTime,
} from '@/utils/date'
import { useTypedParams } from '@/utils/params'

import { InvoiceConfirmationDialog } from './InvoiceConfirmationDialog'
import { InvoiceEditForm } from './InvoiceEditForm'
import { SharePaymentLinkDialog } from './SharePaymentLinkDialog'

import type { CreditUnitNames } from '@/features/subscriptions/subscriptionPrice'

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

/** Client-side throttle on the preview's refresh button — the server has its own limits. */
const REFRESH_COOLDOWN_SECONDS = 10

/**
 * `dataUpdatedAt` is stamped every time the invoice's lines are recomputed — by the hourly
 * background sweep, by the refresh button, or at finalization. Absent means the draft still
 * carries the lines it was created with.
 */
const formatLastRefreshed = (dataUpdatedAt?: string) => {
  if (!dataUpdatedAt) return null
  const date = new Date(dataUpdatedAt)
  if (Number.isNaN(date.getTime())) return null
  return `${formatDistanceToNowStrict(date)} ago`
}

// Component for inline invoice preview with direct SVG rendering
const InvoicePreviewFrame: React.FC<{
  invoiceId: string
  invoice: DetailedInvoice
  canRefresh: boolean
  isRefreshing: boolean
  onRefresh: () => void
  /** Bumped once the refreshed preview has landed — drives the confirmation checkmark. */
  lastRefreshedAt?: number
  /** Server-side stamp, so the tooltip reads correctly on a page the operator just opened. */
  lastRefreshedLabel?: string | null
}> = ({
  invoiceId,
  invoice,
  canRefresh,
  isRefreshing,
  onRefresh,
  lastRefreshedAt,
  lastRefreshedLabel,
}) => {
  const pdfUrl =
    invoice.pdfDocumentId && invoice.documentSharingKey
      ? `${env.meteroidRestApiUri}/files/v1/invoice/pdf/${invoice.localId}?token=${invoice.documentSharingKey}`
      : null
  // An issued invoice is a legal document: once its PDF is stored, that file is what is shown,
  // never a re-render with today's code and settings.
  const issuedPdf = invoice.status !== InvoiceStatus.DRAFT ? pdfUrl : null

  const previewQuery = useQuery(
    previewInvoiceSvg,
    { id: invoiceId },
    { refetchOnMount: 'always', enabled: issuedPdf === null }
  )
  const [cooldown, setCooldown] = useState(0)
  const [justRefreshed, setJustRefreshed] = useState(false)

  useEffect(() => {
    if (cooldown <= 0) return
    const timer = setTimeout(() => setCooldown(seconds => seconds - 1), 1000)
    return () => clearTimeout(timer)
  }, [cooldown])

  useEffect(() => {
    if (!lastRefreshedAt) return
    setJustRefreshed(true)
    const timer = setTimeout(() => setJustRefreshed(false), 2000)
    return () => clearTimeout(timer)
  }, [lastRefreshedAt])

  const handleRefresh = () => {
    onRefresh()
    setCooldown(REFRESH_COOLDOWN_SECONDS)
  }

  if (issuedPdf) {
    return (
      <div className="w-full h-full flex flex-col bg-muted">
        <iframe
          src={issuedPdf}
          title={`Invoice ${invoice.invoiceNumber}`}
          className="w-full flex-1 min-h-[80vh] border-0"
        />
      </div>
    )
  }

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
        className="flex flex-col items-center justify-center gap-5 bg-gray-100 py-10 px-4 relative"
        style={{ minHeight: 'fit-content' }}
      >
        {svgContents.map((svgContent, i) => (
          <div
            className="bg-white max-w-full [&>svg]:h-auto [&>svg]:max-w-full"
            key={`svg-${i}`}
            style={{
              boxShadow: '0px 4px 12px rgba(89, 85, 101, .2)',
            }}
            dangerouslySetInnerHTML={{ __html: svgContent }}
          />
        ))}

        {/* Floating action: refresh while the invoice is still a draft, download once it has a PDF */}
        {(canRefresh || pdfUrl) && (
          <div className="absolute top-4 right-4 sm:top-16 sm:right-16">
            {canRefresh ? (
              <Tooltip>
                {/* span wrapper: a disabled button swallows the events the tooltip needs */}
                <TooltipTrigger asChild>
                  <span tabIndex={0}>
                    <Button
                      variant="flat"
                      size="icon"
                      className="shadow-lg"
                      onClick={handleRefresh}
                      disabled={isRefreshing || cooldown > 0}
                    >
                      {justRefreshed ? (
                        <CheckIcon size="16" className="text-success" />
                      ) : (
                        <RefreshCcw size="16" className={cn(isRefreshing && 'animate-spin')} />
                      )}
                    </Button>
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  {justRefreshed
                    ? 'Preview updated'
                    : cooldown > 0
                      ? `Refresh again in ${cooldown}s`
                      : lastRefreshedLabel
                        ? `Refresh invoice data — last refreshed ${lastRefreshedLabel}`
                        : 'Refresh invoice data'}
                </TooltipContent>
              </Tooltip>
            ) : (
              <Button asChild variant="flat" size="icon" className="shadow-lg">
                <a
                  href={pdfUrl ?? '#'}
                  download={`invoice_${invoice.invoiceNumber}.pdf`}
                  target="_blank"
                  rel="noreferrer"
                  className="flex items-center gap-2"
                >
                  <Download size="16" />
                </a>
              </Button>
            )}
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
  const isExpress = useIsExpressOrganization()
  const canManageInvoices = usePermissions().has('billing:invoices:manage')
  const [isEditMode, setIsEditMode] = useState(false)
  const [isAddPaymentDialogOpen, setIsAddPaymentDialogOpen] = useState(false)
  const [lastRefreshedAt, setLastRefreshedAt] = useState<number>()
  const [paymentLinkUrl, setPaymentLinkUrl] = useState<string | null>(null)
  const [isMarkAsPaidDialogOpen, setIsMarkAsPaidDialogOpen] = useState(false)

  const customPropertyRows = useCustomPropertyRows(
    CustomPropertyEntityType.INVOICE,
    invoice.customProperties
  )

  const refresh = useMutation(refreshInvoiceData, {
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: getInvoice,
            cardinality: undefined
          })
        }),
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: previewInvoiceSvg,
            cardinality: undefined
          })
        }),
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: checkEinvoicingReadiness,
            cardinality: undefined
          })
        }),
      ])
      // After the invalidations settle, so the checkmark means "the preview you see is new"
      setLastRefreshedAt(Date.now())
    },
    onError: error => {
      toast.error(`Failed to refresh invoice: ${error.message}`)
    },
  })

  const deleteInvoiceMutation = useMutation(deleteInvoice, {
    onSuccess: async () => {
      // Show success toast
      toast.success(`Invoice deleted`)
      // Invalidate the list invoices query to refresh the list
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listInvoices,
          input: {},
          cardinality: 'finite'
        }),
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
        queryKey: createConnectQueryKey({
          schema: getInvoice,
          input: { id: invoice?.id ?? '' },
          cardinality: 'finite'
        }),
      })
    },
    onError: error => {
      toast.error(`Failed to finalize invoice: ${error.message}`)
    },
  })

  const voidInvoiceMutation = useMutation(voidInvoice, {
    onSuccess: async () => {
      toast.success('Invoice voided')
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: getInvoice,
            input: { id: invoice?.id ?? '' },
            cardinality: 'finite'
          }),
        }),
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: listCreditNotesByInvoiceId,

            input: {
              invoiceId: invoice?.id ?? '',
            },

            cardinality: 'finite'
          }),
        }),
      ])
    },
    onError: error => {
      toast.error(`Failed to void invoice: ${error.message}`)
    },
  })

  const regenerateInvoiceMutation = useMutation(regenerateInvoice, {
    onSuccess: async data => {
      toast.success('Invoice regenerated')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listInvoices,
          cardinality: 'finite',
        }),
      })
      if (data.invoice?.id) {
        navigate(`${basePath}/invoices/${data.invoice.id}`)
      }
    },
    onError: error => {
      toast.error(`Failed to regenerate invoice: ${error.message}`)
    },
  })

  const markInvoiceAsUncollectibleMutation = useMutation(markInvoiceAsUncollectible, {
    onSuccess: async () => {
      toast.success('Invoice marked as uncollectible')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getInvoice,
          input: { id: invoice?.id ?? '' },
          cardinality: 'finite'
        }),
      })
    },
    onError: error => {
      toast.error(`Failed to mark invoice as uncollectible: ${error.message}`)
    },
  })

  const paymentTokenMutation = useMutation(generateInvoicePaymentToken, {
    onSuccess: data => {
      const paymentUrl = `${window.location.origin}/portal/invoice-payment?token=${data.token}`
      setPaymentLinkUrl(paymentUrl)
    },
    onError: error => {
      console.error('Failed to generate payment token:', error)
      toast.error('Failed to generate payment link')
    },
  })

  const doRefresh = () => refresh.mutate({ id: invoice?.id ?? '' })
  const handlePayOnline = () => paymentTokenMutation.mutate({ invoiceId: invoice?.id ?? '' })

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

  const handleRegenerateConfirm = () => {
    setShowRegenerateConfirmation(false)
    regenerateInvoiceMutation.mutate({ id: invoice?.id ?? '' })
  }

  const handleMarkAsUncollectibleConfirm = () => {
    setShowMarkAsUncollectibleConfirmation(false)
    markInvoiceAsUncollectibleMutation.mutateAsync({ id: invoice?.id ?? '' })
  }

  const isDraft = invoice && invoice.status === InvoiceStatus.DRAFT
  const isFinalized = invoice && invoice.status === InvoiceStatus.FINALIZED
  const canEdit = isDraft

  const pdf_url =
    invoice.documentSharingKey &&
    `${env.meteroidRestApiUri}/files/v1/invoice/pdf/${invoice.localId}?token=${invoice.documentSharingKey}`
  // The e-invoice XML, stored beside the PDF. Only there when the invoice was
  // actually issued as an e-invoice, so the action only appears then.
  const xml_url =
    invoice.xmlDocumentId &&
    invoice.documentSharingKey &&
    `${env.meteroidRestApiUri}/files/v1/invoice/xml/${invoice.localId}?token=${invoice.documentSharingKey}`

  const connectorsQuery = useQuery(listConnectors, {})
  const connectorsData = connectorsQuery.data?.connectors ?? []

  const [showSyncPennylaneModal, setShowSyncPennylaneModal] = useState(false)
  const [showDeleteConfirmation, setShowDeleteConfirmation] = useState(false)
  const [showFinalizeConfirmation, setShowFinalizeConfirmation] = useState(false)
  const [showVoidConfirmation, setShowVoidConfirmation] = useState(false)
  const [showRegenerateConfirmation, setShowRegenerateConfirmation] = useState(false)
  const [showMarkAsUncollectibleConfirmation, setShowMarkAsUncollectibleConfirmation] =
    useState(false)
  const [showCreateCreditNoteDialog, setShowCreateCreditNoteDialog] = useState(false)
  const isPennylaneConnected = connectorsData.some(
    connector => connector.provider === ConnectorProviderEnum.PENNYLANE
  )

  // Query for related credit notes
  const creditNotesQuery = useQuery(
    listCreditNotesByInvoiceId,
    { invoiceId: invoice.id },
    { enabled: Boolean(invoice.id) }
  )
  const creditNotes = creditNotesQuery.data?.creditNotes ?? []

  // A refund draws on a finalized refund credit note. The list model carries the accounting
  // figure, not what is left on it; the backend enforces the exact remaining bound.
  const refundCreditNoteOptions = creditNotes
    .filter(
      cn =>
        cn.status === CreditNoteStatus.FINALIZED &&
        cn.creditType === CreditType.REFUND &&
        cn.refundedAmountCents > 0n
    )
    .map(cn => ({
      id: cn.id,
      creditNoteNumber: cn.creditNoteNumber,
      refundableCents: cn.refundedAmountCents,
    }))

  const paymentCapabilities = connectorsData
    .filter(connector => connector.status === ConnectorStatusEnum.ACTIVE)
    .map(connector => connector.paymentCapabilities)
  const supportsRefunds = paymentCapabilities.some(caps => caps?.supportsRefunds)
  const supportsPartialRefunds = paymentCapabilities.some(caps => caps?.supportsPartialRefunds)

  const lastRefreshedLabel = formatLastRefreshed(invoice.dataUpdatedAt)

  const canRefresh = isDraft && !invoice.manual
  const canDelete = isDraft
  const canFinalize = isDraft
  const canCreateCreditNote = isFinalized
  const canSendToPennylane = isPennylaneConnected

  const canVoid = isFinalized
  // A voided subscription period is billed again only on request, and only while it is the
  // subscription's current period with no live invoice: the server decides.
  const canRegenerate = invoice.regenerable
  const canMarkAsUncollectible = isFinalized

  // "Create Corrected Invoice" is enabled when the parent is fully credited
  // (sum of finalized CN totals ≥ invoice total) and no child draft exists yet.
  // Matches the backend invariant in create_corrected_invoice_from_tx.
  const totalCredited = creditNotes
    .filter(cn => cn.status === CreditNoteStatus.FINALIZED)
    .reduce((sum, cn) => sum + Math.abs(Number(cn.total)), 0)
  const hasChildInvoice = Boolean(invoice.childInvoiceId)
  const canCreateCorrectedInvoice =
    totalCredited >= Number(invoice.total) && !hasChildInvoice

  const createCorrectedInvoiceMutation = useMutation(createCorrectedInvoice, {
    onSuccess: data => {
      toast.success('Corrected invoice created')
      if (data.invoice?.id) {
        navigate(`${basePath}/invoices/${data.invoice.id}`)
      }
    },
    onError: error => {
      toast.error(`Failed to create corrected invoice: ${error.message}`)
    },
  })

  const handleCreateCorrectedInvoice = () => {
    createCorrectedInvoiceMutation.mutate({ parentInvoiceId: invoice.id })
  }

  useEffect(() => {
    if (canRefresh) {
      // doRefresh() TODO uncomment
    }
  }, [])

  if (isEditMode) {
    return (
      <InvoiceEditForm
        invoice={invoice}
        invoiceId={invoiceId}
        onCancel={() => {
          setIsEditMode(false)
          queryClient.invalidateQueries({
            queryKey: createConnectQueryKey({
              schema: getInvoice,
              input: { id: invoiceId },
              cardinality: 'finite'
            }),
          })
        }}
        onSuccess={() => setIsEditMode(false)}
      />
    );
  }

  return (
    <Flex className="flex-col lg:flex-row lg:h-full">
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
        open={showRegenerateConfirmation}
        onOpenChange={setShowRegenerateConfirmation}
        onConfirm={handleRegenerateConfirm}
        icon={RotateCcw}
        title="Regenerate invoice"
        description="Create a new invoice for the same billing period, recalculated with the subscription's current pricing, usage and credit. This voided invoice stays as it is."
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

      <CreateCreditNoteDialog
        open={showCreateCreditNoteDialog}
        onOpenChange={setShowCreateCreditNoteDialog}
        invoice={invoice}
      />

      {/* Left Panel - Invoice Details */}
      <Flex
        direction="column"
        className="w-full lg:w-1/3 border-b lg:border-b-0 lg:border-r border-border"
      >
        {/* Fixed Header - Always Visible */}
        <Flex direction="column" className="gap-2 p-6 border-b border-border">
          <div className="flex justify-between items-start gap-2">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 min-w-0">
              <InvoiceStatusBadge
                status={invoice.status}
                consolidatedInto={invoice.consolidatedIntoInvoiceId}
              />
              {invoice.consolidatedIntoInvoiceId ? (
                <Link
                  to={`${basePath}/invoices/${invoice.consolidatedIntoInvoiceId}`}
                  className="flex items-center gap-1 text-lg font-medium hover:underline"
                >
                  Merged into {invoice.consolidatedIntoInvoiceNumber ?? 'consolidated invoice'}
                  <ExternalLink size={15} />
                </Link>
              ) : (
                <div className="text-lg font-medium">Invoice {invoice.invoiceNumber}</div>
              )}
              {invoice.parentInvoiceId && (
                <Link
                  to={`${basePath}/invoices/${invoice.parentInvoiceId}`}
                  className="text-xs text-muted-foreground hover:text-foreground underline"
                >
                  Replaces original
                </Link>
              )}
              {invoice.childInvoiceId && (
                <Link
                  to={`${basePath}/invoices/${invoice.childInvoiceId}`}
                  className="text-xs text-muted-foreground hover:text-foreground underline"
                >
                  Replaced by corrected
                </Link>
              )}
            </div>
            {!invoice.consolidatedIntoInvoiceId && canManageInvoices && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary" size="sm" hasIcon className="shrink-0">
                  Actions
                  <ChevronDown className="w-4 h-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {!isExpress && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem onClick={() => setIsEditMode(true)} disabled={!canEdit}>
                          <Edit size="16" className="mr-2" />
                          Edit Invoice
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!canEdit && <TooltipContent>Only draft invoices can be edited</TooltipContent>}
                  </Tooltip>
                )}
                {!isExpress && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          onClick={doRefresh}
                          disabled={!canRefresh || refresh.isPending}
                        >
                          <RefreshCcw
                            size="16"
                            className={cn(refresh.isPending && 'animate-spin', 'mr-2')}
                          />
                          Refresh
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!canRefresh && (
                      <TooltipContent>
                        {!isDraft
                          ? 'Only draft invoices can be refreshed'
                          : 'Manual invoices cannot be refreshed'}
                      </TooltipContent>
                    )}
                  </Tooltip>
                )}
                {invoice.paymentStatus !== InvoicePaymentStatus.PAID &&
                  Number(invoice.amountDue) > 0 && (
                    <DropdownMenuItem
                      onClick={handlePayOnline}
                      disabled={paymentTokenMutation.isPending}
                    >
                      <ExternalLink size="16" className="mr-2" />
                      Share Payment Link
                    </DropdownMenuItem>
                  )}
                {!isExpress &&
                  invoice.status === InvoiceStatus.FINALIZED &&
                  Number(invoice.amountDue) > 0 && (
                    <DropdownMenuItem onClick={() => setIsMarkAsPaidDialogOpen(true)}>
                      <CheckCircleIcon size="16" className="mr-2" />
                      Mark as Paid
                    </DropdownMenuItem>
                  )}
                {!isExpress && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          disabled={!canFinalize}
                          onClick={() => setShowFinalizeConfirmation(true)}
                        >
                          <CheckCircleIcon size="16" className="mr-2" />
                          Finalize & Send
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!canFinalize && (
                      <TooltipContent>Only draft invoices can be finalized</TooltipContent>
                    )}
                  </Tooltip>
                )}
                {!isExpress && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          disabled={!canSendToPennylane}
                          onClick={() => setShowSyncPennylaneModal(true)}
                        >
                          <FolderSyncIcon size="16" className="mr-2" />
                          Sync to Pennylane
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!canSendToPennylane && (
                      <TooltipContent>Pennylane integration not connected</TooltipContent>
                    )}
                  </Tooltip>
                )}
                {!isExpress && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          disabled={!canCreateCreditNote}
                          onClick={() => setShowCreateCreditNoteDialog(true)}
                        >
                          <FileText size="16" className="mr-2" />
                          Create Credit Note
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!canCreateCreditNote && (
                      <TooltipContent>Only finalized invoices can have credit notes</TooltipContent>
                    )}
                  </Tooltip>
                )}
                {!isExpress && totalCredited > 0 && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          disabled={
                            !canCreateCorrectedInvoice || createCorrectedInvoiceMutation.isPending
                          }
                          onClick={handleCreateCorrectedInvoice}
                        >
                          <FilePlus2 size="16" className="mr-2" />
                          Create Corrected Invoice
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {hasChildInvoice && (
                      <TooltipContent>
                        A corrected invoice already exists for this invoice
                      </TooltipContent>
                    )}
                  </Tooltip>
                )}
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span>
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
                    </span>
                  </TooltipTrigger>
                  {!invoice.pdfDocumentId && <TooltipContent>PDF not yet generated</TooltipContent>}
                </Tooltip>
                {!isExpress && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          disabled={!canDelete}
                          onClick={() => setShowDeleteConfirmation(true)}
                          className="text-destructive focus:text-destructive"
                        >
                          <Trash2 size="16" className="mr-2" />
                          Delete
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!canDelete && (
                      <TooltipContent>Only draft invoices can be deleted</TooltipContent>
                    )}
                  </Tooltip>
                )}
                {!isExpress && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          disabled={!canVoid}
                          onClick={() => setShowVoidConfirmation(true)}
                          className="text-destructive focus:text-destructive"
                        >
                          <BanIcon size="16" className="mr-2" />
                          Void
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!canVoid && (
                      <TooltipContent>Only finalized invoices can be voided</TooltipContent>
                    )}
                  </Tooltip>
                )}
                {!isExpress && canRegenerate && (
                  <DropdownMenuItem
                    disabled={regenerateInvoiceMutation.isPending}
                    onClick={() => setShowRegenerateConfirmation(true)}
                  >
                    <RotateCcw size="16" className="mr-2" />
                    Regenerate invoice
                  </DropdownMenuItem>
                )}
                {!isExpress && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          disabled={!canMarkAsUncollectible}
                          onClick={() => setShowMarkAsUncollectibleConfirmation(true)}
                          className="text-destructive focus:text-destructive"
                        >
                          <FileX2Icon size="16" className="mr-2" />
                          Mark As Uncollectible
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!canMarkAsUncollectible && (
                      <TooltipContent>
                        Only finalized invoices can be marked as uncollectible
                      </TooltipContent>
                    )}
                  </Tooltip>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
            )}
          </div>

          {invoice.consolidatedIntoInvoiceId && (
            <Link
              to={`${basePath}/invoices/${invoice.consolidatedIntoInvoiceId}`}
              className="flex items-center gap-2 rounded-md border border-border bg-muted/40 px-3 py-2 text-[13px] text-muted-foreground hover:text-foreground"
            >
              <ExternalLink size={14} />
              <span>
                This invoice was consolidated into{' '}
                <span className="font-medium">
                  {invoice.consolidatedIntoInvoiceNumber ?? 'a single invoice'}
                </span>
                . It is not charged or sent on its own — open the consolidated invoice to view or
                pay.
              </span>
            </Link>
          )}

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
            {invoice.subscriptionId && (
              <FlexDetails
                title="Subscription"
                value={
                  <Link
                    to={`${basePath}/subscriptions/${invoice.subscriptionId}`}
                    className="text-[13px] text-brand hover:underline"
                  >
                    View subscription
                  </Link>
                }
              />
            )}
            {invoice.billingPeriodStart && (
              <FlexDetails
                title="Invoice date"
                value={formatDateOnly(invoice.billingPeriodStart)}
              />
            )}
            <FlexDetails
              title={invoice.datesProjected ? 'Issue date (projected)' : 'Issue date'}
              value={formatDateOnly(invoice.invoiceDate)}
            />
            <FlexDetails
              title={invoice.datesProjected ? 'Due date (projected)' : 'Due date'}
              value={formatDateOnlyOptional(invoice.dueAt)}
            />
            {isDraft && (
              <FlexDetails
                title="Usage refreshed"
                value={
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span className="text-[13px]">{lastRefreshedLabel ?? 'Not yet'}</span>
                    </TooltipTrigger>
                    <TooltipContent>
                      {invoice.dataUpdatedAt
                        ? parseAndFormatDateTime(invoice.dataUpdatedAt)
                        : 'This draft still carries the lines it was created with. Usage refreshes hourly.'}
                    </TooltipContent>
                  </Tooltip>
                }
              />
            )}
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

          <EInvoicingSection
            invoice={invoice}
            isDraft={!!isDraft}
            pdfUrl={pdf_url || undefined}
            xmlUrl={xml_url || undefined}
          />

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
              value={
                <PaymentStatusBadge
                  status={invoice.paymentStatus}
                  transactions={invoice.transactions}
                  settledByCreditNotes={invoice.settledByCreditNotes}
                />
              }
            />
            {invoice.paymentDisputedAt && (
              <FlexDetails
                title="Disputed"
                value={
                  <Badge variant="warning">
                    Since {parseAndFormatDate(invoice.paymentDisputedAt)}
                  </Badge>
                }
              />
            )}
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
                  creditNoteOptions={refundCreditNoteOptions}
                  supportsRefunds={supportsRefunds}
                  supportsPartialRefunds={supportsPartialRefunds}
                  showActions={!isExpress && canManageInvoices}
                />
              </div>
            )}

            {!isExpress &&
              invoice.status === InvoiceStatus.FINALIZED &&
              Number(invoice.amountDue) > 0 && (
                <div className="mt-4">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setIsAddPaymentDialogOpen(true)}
                    className="w-full"
                  >
                    <CreditCard className="w-4 h-4 mr-2" />
                    Add Manual Payment
                  </Button>
                </div>
              )}
          </Flex>

          {!isDraft && <InvoiceRemindersCard invoiceId={invoiceId} />}

          <AddManualPaymentDialog
            open={isAddPaymentDialogOpen}
            onOpenChange={setIsAddPaymentDialogOpen}
            invoiceId={invoiceId}
            currency={invoice.currency}
            maxAmount={(Number(invoice.amountDue) / 100).toFixed(2)}
          />

          <MarkAsPaidDialog
            open={isMarkAsPaidDialogOpen}
            onOpenChange={setIsMarkAsPaidDialogOpen}
            invoiceId={invoiceId}
            invoiceNumber={invoice.invoiceNumber}
            currency={invoice.currency}
            totalAmount={(Number(invoice.amountDue) / 100).toFixed(2)}
          />

          <SharePaymentLinkDialog
            url={paymentLinkUrl}
            onClose={() => setPaymentLinkUrl(null)}
          />

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

          {creditNotes.length > 0 && (
            <>
              <Separator className="-my-3" />
              <Flex direction="column" className="gap-2 p-6">
                <div className="text-[15px] font-medium">Credit Notes</div>
                <div className="space-y-2">
                  {creditNotes.map(cn => (
                    <Link
                      key={cn.id}
                      to={`${basePath}/credit-notes/${cn.id}`}
                      className="flex items-center justify-between p-2 rounded-md hover:bg-muted/50 transition-colors"
                    >
                      <div className="flex items-center gap-3">
                        <FileText size={14} className="text-muted-foreground" />
                        <div>
                          <div className="text-[13px] font-medium">{cn.creditNoteNumber}</div>
                          <div className="text-[11px] text-muted-foreground">
                            {CREDIT_TYPE_LABELS[cn.creditType]} · {parseAndFormatDate(cn.createdAt)}
                          </div>
                        </div>
                      </div>
                      <div className="flex items-center gap-2">
                        <span className="text-[13px] font-medium">
                          {formatCurrency(Math.abs(Number(cn.total)), cn.currency)}
                        </span>
                        <CreditNoteStatusBadge status={cn.status} />
                      </div>
                    </Link>
                  ))}
                </div>
              </Flex>
            </>
          )}

          {invoice.consolidatedChildren.length > 0 && (
            <>
              <Separator className="-my-3" />
              <Flex direction="column" className="gap-2 p-6">
                <div className="text-[15px] font-medium">
                  Consolidated from {invoice.consolidatedChildren.length} subscriptions
                </div>
                <div className="text-[12px] text-muted-foreground">
                  This invoice merges the same-day subscription renewals below into a single charge.
                </div>
                <div className="space-y-2">
                  {invoice.consolidatedChildren.map(child => (
                    <div
                      key={child.id}
                      className="flex items-center justify-between gap-3 p-2 rounded-md hover:bg-muted/50 transition-colors"
                    >
                      <div className="flex min-w-0 items-center gap-3">
                        <FileText size={14} className="shrink-0 text-muted-foreground" />
                        {child.subscriptionId ? (
                          <Link
                            to={`${basePath}/subscriptions/${child.subscriptionId}`}
                            className="truncate text-[13px] font-medium text-brand hover:underline"
                          >
                            {child.planName ?? 'Subscription'}
                          </Link>
                        ) : (
                          <div className="truncate text-[13px] font-medium">
                            {child.planName ?? 'Subscription'}
                          </div>
                        )}
                      </div>
                      <div className="flex shrink-0 items-center gap-3">
                        <span className="text-[13px] font-medium">
                          {formatCurrency(Number(child.total), invoice.currency)}
                        </span>
                        <Link
                          to={`${basePath}/invoices/${child.id}`}
                          className="flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground"
                          title="Open the per-subscription document"
                        >
                          Document
                          <ExternalLink size={12} />
                        </Link>
                      </div>
                    </div>
                  ))}
                </div>
              </Flex>
            </>
          )}

          {customPropertyRows.length > 0 && (
            <>
              <Separator className="-my-3" />
              <Flex direction="column" className="gap-2 p-6">
                <div className="text-[15px] font-medium">Custom properties</div>
                {customPropertyRows.map(row => (
                  <FlexDetails
                    key={row.key}
                    title={row.label}
                    value={<CustomPropertyValue def={row.def} value={row.raw} />}
                  />
                ))}
              </Flex>
            </>
          )}

          <Separator className="-my-3" />
          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Activity</div>
            <EntityActivityTimeline
              entityType="invoice"
              entityId={invoiceId}
              emptyLabel="No activity yet for this invoice"
            />
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

      {/* Right Panel - Invoice Preview (a merged child has no standalone document) */}
      <div className="w-full lg:w-2/3 flex flex-col">
        <div className="flex-1 overflow-auto p-6">
          {invoice.consolidatedIntoInvoiceId ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 text-center text-muted-foreground">
              <FileText size={32} className="opacity-50" />
              <div className="text-sm font-medium text-foreground">
                No standalone document
              </div>
              <div className="max-w-sm text-[13px]">
                This is an internal per-subscription record that was merged into{' '}
                {invoice.consolidatedIntoInvoiceNumber ?? 'a consolidated invoice'}. The customer
                only ever receives the consolidated invoice.
              </div>
              <Link
                to={`${basePath}/invoices/${invoice.consolidatedIntoInvoiceId}`}
                className="flex items-center gap-1 text-[13px] font-medium text-foreground hover:underline"
              >
                Open the consolidated invoice
                <ExternalLink size={14} />
              </Link>
            </div>
          ) : (
            <InvoicePreviewFrame
              invoiceId={invoiceId}
              invoice={invoice}
              canRefresh={canRefresh}
              isRefreshing={refresh.isPending}
              onRefresh={doRefresh}
              lastRefreshedAt={lastRefreshedAt}
              lastRefreshedLabel={lastRefreshedLabel}
            />
          )}
        </div>
      </div>
    </Flex>
  )
}

const CREDIT_TYPE_LABELS: Record<CreditType, string> = {
  [CreditType.CREDIT_TO_BALANCE]: 'Credit to balance',
  [CreditType.REFUND]: 'Refund',
  [CreditType.DEBT_CANCELLATION]: 'Debt cancellation',
}

export const InvoiceSummaryLines: React.FC<{ invoice: DetailedInvoice }> = ({ invoice }) => {
  const taxAmount = Number(invoice.taxAmount) || 0
  const total = Number(invoice.total) || 0
  const appliedCredits = Number(invoice.appliedCredits) || 0
  const summary = invoiceSummary(invoice)
  const included = summary.taxIncluded ? ' included' : ''

  return (
    <div className="space-y-1">
      <FlexDetails title="Subtotal" value={formatCurrency(summary.subtotal, invoice.currency)} />

      {summary.reductions.map(row => (
        <FlexDetails
          key={row.label}
          title={row.label}
          value={`-${formatCurrency(row.amount, invoice.currency)}`}
        />
      ))}

      {summary.totalExclTax !== undefined && taxAmount > 0 && (
        <FlexDetails
          title="Total excluding tax"
          value={formatCurrency(summary.totalExclTax, invoice.currency)}
        />
      )}

      {invoice.taxBreakdown && invoice.taxBreakdown.length > 0
        ? invoice.taxBreakdown.map(tax => {
            const taxRate = rateToPercent(tax.taxRate)
            const taxAmountValue = Number(tax.amount) || 0
            // Only show tax breakdown if rate is greater than 0
            if (taxRate > 0) {
              return (
                <FlexDetails
                  key={tax.name}
                  title={
                    <span>
                      {tax.name} ({taxRate}%{included})
                      {tax.overridden && (
                        <span className="ml-1.5 text-[11px] text-warning">
                          overridden: EU VAT not applied
                        </span>
                      )}
                    </span>
                  }
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

      {appliedCredits > 0 && (
        <FlexDetails
          title="Credits applied"
          value={
            <span className="text-success">
              -{formatCurrency(appliedCredits, invoice.currency)}
            </span>
          }
        />
      )}
    </div>
  )
}

export const InvoiceLineItems: React.FC<{ items: LineItem[]; invoice: DetailedInvoice }> = ({
  items,
  invoice,
}) => {
  const units = useCreditUnitNames(items.some(item => item.creditGrantUnit !== undefined))
  return (
    <div className="space-y-2">
      {items
        .sort((a, b) => a.name.localeCompare(b.name))
        .map(item => {
          return (
            <InvoiceLineItemCard key={item.id} line_item={item} invoice={invoice} units={units} />
          )
        })}
    </div>
  )
}

const InvoiceLineItemCard: React.FC<{
  line_item: LineItem
  invoice: DetailedInvoice
  units: Record<string, CreditUnitNames>
}> = ({ line_item, invoice, units }) => {
  const [isExpanded, setIsExpanded] = useState(false)
  // A tax-inclusive line is shown as quoted, like the PDF and the summary.
  const subLines = lineShownSubLines(line_item)
  const hasSubItems = subLines.length > 0
  const unitPrice = lineShownUnitPrice(line_item)
  const subtotal = lineShownSubtotal(line_item)
  const packText = packCreditText(line_item, invoice.currency, units)

  return (
    <div className="py-2">
      <div
        className={cn('flex justify-between items-start gap-2', hasSubItems && 'cursor-pointer')}
        onClick={() => hasSubItems && setIsExpanded(!isExpanded)}
      >
        <div className="flex-1 min-w-0">
          <div className="flex items-start gap-2">
            {hasSubItems && (
              <ChevronDown
                size={12}
                className={cn(
                  'text-muted-foreground transition-transform shrink-0 mt-0.5',
                  isExpanded && 'rotate-180'
                )}
              />
            )}
            <div className="text-[13px] font-medium break-words">{line_item.name}</div>
          </div>
          {line_item.startDate && line_item.endDate && (
            <div className="text-[11px] text-muted-foreground mt-1 ml-4">
              {formatDateOnly(line_item.startDate)} → {formatDateOnly(line_item.endDate)}
            </div>
          )}
          {packText && (
            <div className="text-[11px] text-muted-foreground mt-1 ml-4">{packText}</div>
          )}
          {line_item.creditUnit && (
            <NamedCreditUnitUsageNote
              credit={line_item.creditUnit}
              estimated={invoice.status === InvoiceStatus.DRAFT}
              className="mt-1 ml-4"
            />
          )}
        </div>
        <div className="text-right">
          {!hasSubItems && line_item.quantity && unitPrice && (
            <div className="text-[11px] text-muted-foreground">
              {formatUsage(parseFloat(line_item.quantity))} ×{' '}
              {formatCurrencyNoRounding(unitPrice, invoice.currency)}
            </div>
          )}
          <div className="text-[13px] font-medium">
            {hasSubItems
              ? formatCurrency(
                  subLines.reduce((sum, sub) => sum + Number(sub.total), 0),
                  invoice.currency
                )
              : formatCurrency(subtotal, invoice.currency)}
          </div>
        </div>
      </div>

      {isExpanded && hasSubItems && (
        <div className="mt-2 ml-4 pt-2 border-t space-y-1">
          {subLines.map(subItem => (
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

/**
 * The e-invoicing section of the side panel: what was issued (or would be, for a draft)
 * and the files that make it up. Absent when there is nothing to say — the entity has
 * not opted in, or the invoice is a consolidated child, never issued on its own.
 *
 * A failure is not an error the reader must act on — the PDF is a valid invoice and was
 * sent — so it reads as a status with the findings a click away. On a draft the
 * findings are a dry run against the live customer and entity records, and still
 * fixable; after finalization the invoice is immutable and they only inform the next.
 */
const EInvoicingSection = ({
  invoice,
  isDraft,
  pdfUrl,
  xmlUrl,
}: {
  invoice: DetailedInvoice
  isDraft: boolean
  pdfUrl?: string
  xmlUrl?: string
}) => {
  const [open, setOpen] = useState(false)
  const checkDraft = isDraft && !invoice.consolidatedIntoInvoiceId
  // 'always': the operator comes back from the edit form or the customer page
  // expecting the verdict to reflect what they just filled in.
  const readinessQuery = useQuery(
    checkEinvoicingReadiness,
    { id: invoice.id },
    { enabled: checkDraft, refetchOnMount: 'always' }
  )

  const outcome = checkDraft
    ? readinessQuery.data?.readiness && {
        profile: readinessQuery.data.readiness.profile || 'e-invoice',
        findings: readinessQuery.data.readiness.findings,
        error: readinessQuery.data.readiness.error,
      }
    : invoice.einvoicingStatus !== EInvoicingStatus.EINVOICING_STATUS_UNSPECIFIED && {
        profile: invoice.einvoicingProfile || 'e-invoice',
        findings: invoice.einvoicingFindings,
        error: invoice.einvoicingError,
      }
  // Absent while a draft's check loads or fails, and when there is nothing to check.
  if (!outcome) return null

  const issues = outcome.findings.length + (outcome.error ? 1 : 0)
  const issued = !checkDraft && invoice.einvoicingStatus === EInvoicingStatus.EINVOICING_GENERATED

  const download = (href: string, filename: string) => (
    <a
      href={href}
      download={filename}
      className="flex items-center gap-1 text-[13px] text-brand hover:underline"
    >
      <Download size={13} />
      Download
    </a>
  )

  return (
    <>
      <Separator className="-my-3" />
      <Flex direction="column" className="gap-2 p-6">
        <div className="text-[15px] font-medium">E-invoicing</div>
        {issues === 0 ? (
          <FlexDetails
            title="Status"
            value={checkDraft ? `Ready to issue as ${outcome.profile}` : `Issued as ${outcome.profile}`}
          />
        ) : (
          <Collapsible open={open} onOpenChange={setOpen}>
            <Flex align="start" justify="between">
              <div className="text-[13px] text-muted-foreground">Status</div>
              <CollapsibleTrigger className="group flex items-center gap-1.5 text-[13px] text-amber-700 hover:underline dark:text-amber-500">
                <TriangleAlertIcon size={13} />
                <span>{checkDraft ? 'Would not be issued' : 'Not issued'}</span>
                <span className="text-muted-foreground">
                  ({issues} {issues === 1 ? 'issue' : 'issues'})
                </span>
                <ChevronDown
                  size={13}
                  className="text-muted-foreground transition-transform group-data-[state=open]:rotate-180"
                />
              </CollapsibleTrigger>
            </Flex>
            <CollapsibleContent className="mt-1.5 rounded-md bg-muted/40 px-3 py-2 text-[12px] text-muted-foreground">
              <p>
                {checkDraft
                  ? `Finalizing now would issue and send the PDF, but not a ${outcome.profile} document:`
                  : `The PDF is a valid invoice and was sent. It was not also issued as ${outcome.profile}:`}
              </p>
              <ul className="mt-1.5 space-y-0.5">
                {outcome.findings.map(finding => (
                  <li key={`${finding.rule}-${finding.term}`}>
                    <span className="font-mono text-[11px]">{finding.term}</span> —{' '}
                    {finding.message}
                    {finding.hint && <span className="italic"> ({finding.hint})</span>}
                  </li>
                ))}
                {outcome.error && <li>{outcome.error}</li>}
              </ul>
              <p className="mt-1.5">
                {checkDraft
                  ? 'This draft still reads the live customer and invoicing entity records, so filling these in before finalizing fixes this invoice.'
                  : 'This invoice is finalized and immutable, so filling these in applies to future invoices, not this one.'}
              </p>
            </CollapsibleContent>
          </Collapsible>
        )}
        {issued && xmlUrl && (
          <FlexDetails
            title="XML file"
            value={download(xmlUrl, `invoice_${invoice.invoiceNumber}.xml`)}
          />
        )}
        {/* Only for a hybrid format, where the PDF itself carries the XML and is the
            e-invoice a receiver imports — not merely the printable copy. */}
        {issued && invoice.einvoicingEmbeddedInPdf && pdfUrl && (
          <FlexDetails
            title="Hybrid PDF"
            value={download(pdfUrl, `invoice_${invoice.invoiceNumber}.pdf`)}
          />
        )}
      </Flex>
    </>
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
