import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
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
import {
  BanIcon,
  CheckCircleIcon,
  ChevronDown,
  Download,
  FileCode,
  FolderSyncIcon,
  Trash2Icon,
} from 'lucide-react'
import { Fragment, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { CreditNoteStatusBadge } from '@/features/creditNotes/CreditNoteStatusBadge'
import { AddressLinesCompact } from '@/features/customers/cards/address/AddressCard'
import { CustomPropertyValue } from '@/features/customproperties/CustomPropertyValue'
import { useCustomPropertyRows } from '@/features/customproperties/useCustomPropertyRows'
import { NamedCreditUnitUsageNote } from '@/features/invoices/CreditUnitUsageNote'
import { RefundDialog } from '@/features/invoices/RefundDialog'
import { TransactionList } from '@/features/invoices/TransactionList'
import { packCreditText } from '@/features/invoices/packCredit'
import { creditNoteRefundLabel, latestSettledPayment } from '@/features/invoices/refunds'
import { useCreditUnitNames } from '@/features/invoices/useCreditUnitNames'
import { SyncCreditNoteModal } from '@/features/settings/integrations/SyncCreditNoteModal'
import { IntegrationType } from '@/features/settings/integrations/SyncInvoiceModal'
import { getCountryName } from '@/features/settings/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { formatCurrency, rateToPercent } from '@/lib/utils/numbers'
import { InvoiceConfirmationDialog } from '@/pages/tenants/invoice/InvoiceConfirmationDialog'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum, ConnectorStatusEnum } from '@/rpc/api/connectors/v1/models_pb'
import {
  deleteDraftCreditNote,
  finalizeCreditNote,
  getCreditNote,
  listCreditNoteRefunds,
  listCreditNotes,
  previewCreditNoteSvg,
  voidCreditNote,
} from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'
import {
  CreditNoteStatus,
  CreditType,
  DetailedCreditNote,
} from '@/rpc/api/creditnotes/v1/models_pb'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { getInvoice } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { LineItem, TaxBreakdownItem } from '@/rpc/api/invoices/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'
import { useTypedParams } from '@/utils/params'

import { resizeSvgContent } from '../invoice/utils'


export const CreditNote = () => {
  const { creditNoteId } = useTypedParams<{ creditNoteId: string }>()
  const creditNoteQuery = useQuery(
    getCreditNote,
    {
      id: creditNoteId ?? '',
    },
    { enabled: Boolean(creditNoteId) }
  )

  const data = creditNoteQuery.data?.creditNote
  const isLoading = creditNoteQuery.isLoading

  return (
    <Fragment>
      <Flex direction="column" className="h-full">
        {isLoading || !data ? (
          <>
            <Skeleton height={16} width={50} />
            <Skeleton height={44} />
          </>
        ) : (
          <CreditNoteView creditNote={data} creditNoteId={creditNoteId ?? ''} />
        )}
      </Flex>
    </Fragment>
  )
}

const CreditNotePreviewFrame: React.FC<{
  creditNoteId: string
  creditNote: DetailedCreditNote
}> = ({ creditNoteId, creditNote }) => {
  const pdfUrl =
    creditNote.pdfDocumentId && creditNote.documentSharingKey
      ? `${env.meteroidRestApiUri}/files/v1/credit-note/pdf/${creditNote.id}?token=${creditNote.documentSharingKey}`
      : null
  // An issued credit note is a legal document: once its PDF is stored, that file is what is
  // shown, never a re-render with today's code and settings.
  const issuedPdf = creditNote.status !== CreditNoteStatus.DRAFT ? pdfUrl : null

  const previewQuery = useQuery(
    previewCreditNoteSvg,
    { id: creditNoteId },
    { refetchOnMount: 'always', enabled: issuedPdf === null }
  )

  if (issuedPdf) {
    return (
      <div className="w-full h-full flex flex-col bg-muted">
        <iframe
          src={issuedPdf}
          title={`Credit note ${creditNote.creditNoteNumber}`}
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

  const svgContents =
    previewQuery.data?.svgs.map(svg => {
      const scaledHtml = svg ? resizeSvgContent(svg, 1) : ''
      const parser = new DOMParser()
      const doc = parser.parseFromString(scaledHtml, 'text/html')
      const svgElement = doc.querySelector('svg')
      return svgElement?.outerHTML || ''
    }) ?? []

  return (
    <div className="w-full h-full flex flex-col">
      <div
        className="flex flex-col items-center justify-center gap-5 bg-gray-100 py-10 px-4 relative"
        style={{ minHeight: 'fit-content' }}
      >
        {svgContents.map((svgContent, i) => (
          <div
            className="bg-white max-w-full [&>svg]:h-auto [&>svg]:max-w-full"
            key={`svg-${i}`}
            style={{
              width: 'fit-content',
              boxShadow: '0 2px 8px rgba(0,0,0,0.1)',
            }}
            dangerouslySetInnerHTML={{ __html: svgContent }}
          />
        ))}

        {/* Floating Download Button */}
        {pdfUrl && (
          <div className="absolute top-4 right-4 sm:top-16 sm:right-16">
            <Button asChild variant="flat" size="icon" className="shadow-lg">
              <a
                href={pdfUrl}
                download={`credit_note_${creditNote.creditNoteNumber}.pdf`}
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

interface Props {
  creditNote: DetailedCreditNote
  creditNoteId: string
}

const CreditNoteView = ({ creditNote, creditNoteId }: Props) => {
  const basePath = useBasePath()
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const customPropertyRows = useCustomPropertyRows(
    CustomPropertyEntityType.CREDIT_NOTE,
    creditNote.customProperties
  )

  const [showFinalizeConfirmation, setShowFinalizeConfirmation] = useState(false)
  const [showVoidConfirmation, setShowVoidConfirmation] = useState(false)
  const [showDeleteConfirmation, setShowDeleteConfirmation] = useState(false)
  const [showSyncPennylaneModal, setShowSyncPennylaneModal] = useState(false)

  const connectorsQuery = useQuery(listConnectors, {})
  const isPennylaneConnected = (connectorsQuery.data?.connectors ?? []).some(
    connector => connector.provider === ConnectorProviderEnum.PENNYLANE
  )

  const finalizeMutation = useMutation(finalizeCreditNote, {
    onSuccess: async () => {
      toast.success('Credit note finalized')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getCreditNote,
          input: { id: creditNoteId },
          cardinality: 'finite'
        }),
      })
    },
    onError: error => {
      toast.error(`Failed to finalize credit note: ${error.message}`)
    },
  })

  const voidMutation = useMutation(voidCreditNote, {
    onSuccess: async () => {
      toast.success('Credit note voided')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getCreditNote,
          input: { id: creditNoteId },
          cardinality: 'finite'
        }),
      })
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listCreditNotes,
          input: {},
          cardinality: 'finite'
        }),
      })
    },
    onError: error => {
      toast.error(`Failed to void credit note: ${error.message}`)
    },
  })

  const deleteMutation = useMutation(deleteDraftCreditNote, {
    onSuccess: async () => {
      toast.success('Credit note deleted')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listCreditNotes,
          input: {},
          cardinality: 'finite'
        }),
      })
      navigate(`${basePath}/credit-notes`)
    },
    onError: error => {
      toast.error(`Failed to delete credit note: ${error.message}`)
    },
  })

  const handleFinalizeConfirm = () => {
    setShowFinalizeConfirmation(false)
    finalizeMutation.mutateAsync({ id: creditNoteId })
  }

  const handleVoidConfirm = () => {
    setShowVoidConfirmation(false)
    voidMutation.mutateAsync({ id: creditNoteId })
  }

  const handleDeleteConfirm = () => {
    setShowDeleteConfirmation(false)
    deleteMutation.mutateAsync({ id: creditNoteId })
  }

  const isDraft = creditNote.status === CreditNoteStatus.DRAFT
  const isFinalized = creditNote.status === CreditNoteStatus.FINALIZED
  const isRefundable = isFinalized && creditNote.creditType === CreditType.REFUND
  const canFinalize = isDraft
  const canVoid = isDraft || isFinalized
  const canDelete = isDraft

  const pdfUrl =
    creditNote.pdfDocumentId && creditNote.documentSharingKey
      ? `${env.meteroidRestApiUri}/files/v1/credit-note/pdf/${creditNote.id}?token=${creditNote.documentSharingKey}`
      : null
  // Only there when the credit note was issued as an e-invoice.
  const xmlUrl =
    creditNote.xmlDocumentId && creditNote.documentSharingKey
      ? `${env.meteroidRestApiUri}/files/v1/credit-note/xml/${creditNote.id}?token=${creditNote.documentSharingKey}`
      : null

  return (
    <Flex className="flex-col lg:flex-row lg:h-full">
      {showSyncPennylaneModal && (
        <SyncCreditNoteModal
          creditNoteNumber={creditNote.creditNoteNumber}
          id={creditNote.id}
          integrationType={IntegrationType.Pennylane}
          onClose={() => setShowSyncPennylaneModal(false)}
        />
      )}

      <InvoiceConfirmationDialog
        open={showFinalizeConfirmation}
        onOpenChange={setShowFinalizeConfirmation}
        onConfirm={handleFinalizeConfirm}
        icon={CheckCircleIcon}
        title="Finalize credit note"
        description="Finalize this credit note. Once finalized, it cannot be edited."
        invoiceNumber={creditNote.creditNoteNumber}
      />

      <InvoiceConfirmationDialog
        open={showVoidConfirmation}
        onOpenChange={setShowVoidConfirmation}
        onConfirm={handleVoidConfirm}
        icon={BanIcon}
        title="Void credit note"
        description="Are you sure you want to void this credit note? This action cannot be undone."
        invoiceNumber={creditNote.creditNoteNumber}
      />

      <InvoiceConfirmationDialog
        open={showDeleteConfirmation}
        onOpenChange={setShowDeleteConfirmation}
        onConfirm={handleDeleteConfirm}
        icon={Trash2Icon}
        title="Delete credit note"
        description="Are you sure you want to delete this draft credit note? This action cannot be undone."
        invoiceNumber={creditNote.creditNoteNumber}
      />

      {/* Left Panel - Credit Note Details */}
      <Flex
        direction="column"
        className="w-full lg:w-1/3 border-b lg:border-b-0 lg:border-r border-border"
      >
        {/* Fixed Header */}
        <Flex direction="column" className="gap-2 p-6 border-b border-border">
          <div className="flex justify-between items-start gap-2">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 min-w-0">
              <CreditNoteStatusBadge status={creditNote.status} />
              <div className="text-lg font-medium">Credit Note {creditNote.creditNoteNumber}</div>
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary" size="sm" hasIcon className="shrink-0">
                  Actions
                  <ChevronDown className="w-4 h-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <Tooltip>
                  <TooltipTrigger asChild>
                    <div>
                      <DropdownMenuItem disabled={!pdfUrl} asChild>
                        <a
                          href={pdfUrl ?? '#'}
                          download={`credit_note_${creditNote.creditNoteNumber}.pdf`}
                          target="_blank"
                          rel="noreferrer"
                          className="flex items-center"
                        >
                          <Download size="16" className="mr-2" />
                          Download PDF
                        </a>
                      </DropdownMenuItem>
                    </div>
                  </TooltipTrigger>
                  {!pdfUrl && <TooltipContent>PDF not yet generated</TooltipContent>}
                </Tooltip>
                {xmlUrl && (
                  <DropdownMenuItem asChild>
                    <a
                      href={xmlUrl}
                      download={`credit_note_${creditNote.creditNoteNumber}.xml`}
                      className="flex items-center"
                    >
                      <FileCode size="16" className="mr-2" />
                      Download e-invoice XML
                    </a>
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem
                  disabled={!canFinalize}
                  onClick={() => setShowFinalizeConfirmation(true)}
                >
                  <CheckCircleIcon size="16" className="mr-2" />
                  Finalize
                </DropdownMenuItem>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <div>
                      <DropdownMenuItem
                        disabled={!isPennylaneConnected}
                        onClick={() => setShowSyncPennylaneModal(true)}
                      >
                        <FolderSyncIcon size="16" className="mr-2" />
                        Sync to Pennylane
                      </DropdownMenuItem>
                    </div>
                  </TooltipTrigger>
                  {!isPennylaneConnected && (
                    <TooltipContent>Pennylane integration not connected</TooltipContent>
                  )}
                </Tooltip>
                <DropdownMenuItem
                  disabled={!canVoid}
                  onClick={() => setShowVoidConfirmation(true)}
                  className="text-destructive focus:text-destructive"
                >
                  <BanIcon size="16" className="mr-2" />
                  Void
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={!canDelete}
                  onClick={() => setShowDeleteConfirmation(true)}
                  className="text-destructive focus:text-destructive"
                >
                  <Trash2Icon size="16" className="mr-2" />
                  Delete
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>

          <div className="text-3xl font-bold">
            {formatCurrency(Math.abs(Number(creditNote.total)) || 0, creditNote.currency)}
          </div>
        </Flex>

        {/* Scrollable Content */}
        <div className="flex-1 overflow-auto">
          <Flex direction="column" className="gap-2 p-6">
            <FlexDetails title="Credit note number" value={creditNote.creditNoteNumber} />
            <FlexDetails title="Type" value={<CreditTypeLabel type={creditNote.creditType} />} />
            <FlexDetails
              title="Related invoice"
              value={
                <Link
                  to={`${basePath}/invoices/${creditNote.invoiceId}`}
                  className="text-[13px] text-brand hover:underline"
                >
                  View invoice
                </Link>
              }
            />
            <FlexDetails title="Created" value={parseAndFormatDate(creditNote.createdAt)} />
            <FlexDetails title="Currency" value={creditNote.currency} />
          </Flex>

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Customer</div>
            <FlexDetails
              title="Customer"
              value={creditNote.customerDetails?.name}
              link={`${basePath}/customers/${creditNote.customerId}`}
            />
            <FlexDetails title="Email" value={creditNote.customerDetails?.email} />
            {creditNote.customerDetails?.billingAddress && (
              <>
                <FlexDetails
                  title="Address"
                  value={
                    <AddressLinesCompact
                      address={creditNote.customerDetails.billingAddress}
                      className="text-right"
                    />
                  }
                />
                <FlexDetails
                  title="Country"
                  value={
                    creditNote.customerDetails.billingAddress.country &&
                    getCountryName(creditNote.customerDetails.billingAddress.country)
                  }
                />
              </>
            )}
            {creditNote.customerDetails?.vatNumber && (
              <FlexDetails title="VAT Number" value={creditNote.customerDetails.vatNumber} />
            )}
          </Flex>

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Line Items</div>
            <CreditNoteLineItems items={creditNote.lineItems} currency={creditNote.currency} />
            <div className="mt-4 pt-4 border-t">
              <CreditNoteSummaryLines creditNote={creditNote} />
            </div>
          </Flex>

          {isRefundable && (
            <>
              <Separator className="-my-3" />
              <CreditNoteRefunds creditNote={creditNote} />
            </>
          )}

          {(creditNote.reason || creditNote.memo) && (
            <>
              <Separator className="-my-3" />
              <Flex direction="column" className="gap-2 p-6">
                {creditNote.reason && (
                  <>
                    <div className="text-[15px] font-medium">Reason</div>
                    <div className="text-[13px] text-muted-foreground whitespace-pre-line">
                      {creditNote.reason}
                    </div>
                  </>
                )}
                {creditNote.memo && (
                  <>
                    <div className="text-[15px] font-medium mt-2">Memo</div>
                    <div className="text-[13px] text-muted-foreground whitespace-pre-line">
                      {creditNote.memo}
                    </div>
                  </>
                )}
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
            <div className="text-[15px] font-medium">Timeline</div>
            <div className="space-y-2">
              <div className="flex items-start gap-2">
                <div className="w-1.5 h-1.5 rounded-full bg-muted-foreground mt-1.5 shrink-0"></div>
                <div>
                  <div className="text-[13px] font-medium">Credit Note Created</div>
                  <div className="text-[11px] text-muted-foreground">
                    {parseAndFormatDate(creditNote.createdAt)}
                  </div>
                </div>
              </div>
              {creditNote.finalizedAt && (
                <div className="flex items-start gap-2">
                  <div className="w-1.5 h-1.5 rounded-full bg-success mt-1.5 shrink-0"></div>
                  <div>
                    <div className="text-[13px] font-medium">Credit Note Finalized</div>
                    <div className="text-[11px] text-muted-foreground">
                      {parseAndFormatDate(creditNote.finalizedAt)}
                    </div>
                  </div>
                </div>
              )}
              {creditNote.voidedAt && (
                <div className="flex items-start gap-2">
                  <div className="w-1.5 h-1.5 rounded-full bg-red-500 mt-1.5 shrink-0"></div>
                  <div>
                    <div className="text-[13px] font-medium">Credit Note Voided</div>
                    <div className="text-[11px] text-muted-foreground">
                      {parseAndFormatDate(creditNote.voidedAt)}
                    </div>
                  </div>
                </div>
              )}
            </div>
          </Flex>
        </div>
      </Flex>

      {/* Right Panel - Credit Note Preview */}
      <div className="w-full lg:w-2/3 flex flex-col">
        <div className="flex-1 overflow-auto p-6">
          <CreditNotePreviewFrame creditNoteId={creditNoteId} creditNote={creditNote} />
        </div>
      </div>
    </Flex>
  )
}


const CreditNoteRefunds: React.FC<{ creditNote: DetailedCreditNote }> = ({ creditNote }) => {
  const [showRefundDialog, setShowRefundDialog] = useState(false)

  const refundsQuery = useQuery(listCreditNoteRefunds, { creditNoteId: creditNote.id })
  const invoiceQuery = useQuery(getInvoice, { id: creditNote.invoiceId })
  const connectorsQuery = useQuery(listConnectors, {})

  const refunds = refundsQuery.data?.refunds ?? []
  const transactions = invoiceQuery.data?.invoice?.transactions ?? []
  // LIFO: the money goes back on the most recent settled payment.
  const parent = latestSettledPayment(transactions)

  const capabilities = (connectorsQuery.data?.connectors ?? [])
    .filter(connector => connector.status === ConnectorStatusEnum.ACTIVE)
    .map(connector => connector.paymentCapabilities)

  const canRefund = Boolean(parent) && creditNote.refundableAmountCents > 0n

  return (
    <Flex direction="column" className="gap-2 p-6">
      <div className="flex items-center justify-between">
        <div className="text-[15px] font-medium">Refunds</div>
        <Button
          variant="outline"
          size="sm"
          disabled={!canRefund}
          onClick={() => setShowRefundDialog(true)}
        >
          Refund
        </Button>
      </div>

      <FlexDetails
        title="Refundable"
        value={formatCurrency(creditNote.refundableAmountCents, creditNote.currency)}
      />
      <FlexDetails
        title="Refunded"
        value={formatCurrency(creditNote.refundedSettledCents, creditNote.currency)}
      />
      <FlexDetails
        title="In flight"
        value={formatCurrency(creditNote.refundedPendingCents, creditNote.currency)}
      />

      {refunds.length > 0 && (
        <div className="mt-2">
          <TransactionList
            transactions={refunds}
            currency={creditNote.currency}
            isLoading={refundsQuery.isLoading}
          />
        </div>
      )}

      {!canRefund && !parent && (
        <div className="text-[11px] text-muted-foreground">
          No settled payment on the invoice to refund.
        </div>
      )}

      {showRefundDialog && parent && (
        <RefundDialog
          open
          onOpenChange={() => setShowRefundDialog(false)}
          transaction={parent}
          transactions={transactions}
          currency={creditNote.currency}
          creditNoteOptions={[
            {
              id: creditNote.id,
              creditNoteNumber: creditNote.creditNoteNumber,
              refundableCents: creditNote.refundableAmountCents,
            },
          ]}
          lockedCreditNoteId={creditNote.id}
          supportsOnlineRefunds={capabilities.some(caps => caps?.supportsRefunds)}
          supportsPartialRefunds={capabilities.some(caps => caps?.supportsPartialRefunds)}
        />
      )}
    </Flex>
  )
}

const CreditTypeLabel: React.FC<{ type: CreditType }> = ({ type }) => {
  switch (type) {
    case CreditType.DEBT_CANCELLATION:
      return (
        <span className="text-[13px] px-2 py-0.5 rounded bg-muted text-muted-foreground">
          Debt cancellation
        </span>
      )
    case CreditType.REFUND:
      return (
        <span className="text-[13px] px-2 py-0.5 rounded bg-destructive/10 text-destructive">
          Refund
        </span>
      )
    case CreditType.CREDIT_TO_BALANCE:
    default:
      return (
        <span className="text-[13px] px-2 py-0.5 rounded bg-success/10 text-success">
          Credit to balance
        </span>
      )
  }
}

const CreditNoteSummaryLines: React.FC<{ creditNote: DetailedCreditNote }> = ({ creditNote }) => {
  const subtotal = Math.abs(Number(creditNote.subtotal)) || 0
  const taxAmount = Math.abs(Number(creditNote.taxAmount)) || 0
  const total = Math.abs(Number(creditNote.total)) || 0
  // Where the credited total went, as the PDF states it next to the totals.
  const effects = [
    {
      label: creditNoteRefundLabel(creditNote),
      amount:
        creditNote.status === CreditNoteStatus.VOIDED
          ? creditNote.refundedSettledCents
          : creditNote.refundedAmountCents,
    },
    { label: 'Credited to balance', amount: creditNote.creditedAmountCents },
    {
      label:
        creditNote.status === CreditNoteStatus.DRAFT
          ? 'Free credit to be restored'
          : 'Free credit restored',
      amount: creditNote.promotionalCreditRestoredCents,
    },
    { label: 'Debt cancelled', amount: creditNote.debtCancelledAmountCents },
  ].filter((effect): effect is { label: string; amount: bigint } =>
    Boolean(effect.label && effect.amount > 0n)
  )

  return (
    <div className="space-y-1">
      <FlexDetails title="Subtotal" value={formatCurrency(subtotal, creditNote.currency)} />

      {creditNote.discountCents > 0n && (
        <FlexDetails
          title="Discount"
          value={`-${formatCurrency(creditNote.discountCents, creditNote.currency)}`}
        />
      )}
      {creditNote.promotionalCreditCents > 0n && (
        <FlexDetails
          title="Free credit"
          value={`-${formatCurrency(creditNote.promotionalCreditCents, creditNote.currency)}`}
        />
      )}

      {creditNote.taxBreakdown && creditNote.taxBreakdown.length > 0
        ? creditNote.taxBreakdown.map((tax: TaxBreakdownItem) => {
            const taxRate = rateToPercent(tax.taxRate)
            const taxAmountValue = Math.abs(Number(tax.amount)) || 0
            if (taxRate > 0) {
              return (
                <FlexDetails
                  key={tax.name}
                  title={`${tax.name} (${taxRate}%)`}
                  value={formatCurrency(taxAmountValue, creditNote.currency)}
                />
              )
            }
            return null
          })
        : taxAmount > 0 && (
            <FlexDetails title="Tax" value={formatCurrency(taxAmount, creditNote.currency)} />
          )}

      <div className="pt-2 border-t">
        <FlexDetails
          title={<span className="font-semibold">Total Credit</span>}
          value={
            <span className="font-semibold text-[15px]">
              {formatCurrency(total, creditNote.currency)}
            </span>
          }
        />
      </div>

      {effects.length > 0 && (
        <div className="pt-2 space-y-1">
          {effects.map(effect => (
            <FlexDetails
              key={effect.label}
              title={effect.label}
              value={formatCurrency(effect.amount, creditNote.currency)}
            />
          ))}
        </div>
      )}
    </div>
  )
}

const CreditNoteLineItems: React.FC<{ items: LineItem[]; currency: string }> = ({
  items,
  currency,
}) => {
  const units = useCreditUnitNames(items.some(item => item.creditGrantUnit !== undefined))
  return (
    <div className="space-y-2">
      {items
        .sort((a, b) => a.name.localeCompare(b.name))
        .map(item => (
          <div key={item.id} className="py-2">
            <div className="flex justify-between items-start gap-2">
              <div className="flex-1 min-w-0">
                <div className="text-[13px] font-medium break-words">{item.name}</div>
                {item.startDate && item.endDate && (
                  <div className="text-[11px] text-muted-foreground mt-1">
                    {parseAndFormatDate(item.startDate)} → {parseAndFormatDate(item.endDate)}
                  </div>
                )}
                {packCreditText(item, currency, units, 'Reverses') && (
                  <div className="text-[11px] text-muted-foreground mt-1">
                    {packCreditText(item, currency, units, 'Reverses')}
                  </div>
                )}
                {item.creditUnit && <NamedCreditUnitUsageNote credit={item.creditUnit} className="mt-1" />}
              </div>
              <div className="text-right">
                <div className="text-[13px] font-medium">
                  {formatCurrency(Math.abs(Number(item.subtotal)), currency)}
                </div>
              </div>
            </div>
          </div>
        ))}
    </div>
  )
}

const FlexDetails = ({
  title,
  value,
  link,
}: {
  title: string | React.ReactNode
  value?: string | React.ReactNode
  link?: string
}) => (
  <Flex align="start" justify="between">
    <div className="text-[13px] text-muted-foreground">{title}</div>
    {link ? (
      <Link to={link}>
        <div className="text-[13px] text-brand hover:underline">{value ?? 'N/A'}</div>
      </Link>
    ) : (
      <div className="text-[13px]">{value ?? 'N/A'}</div>
    )}
  </Flex>
)
