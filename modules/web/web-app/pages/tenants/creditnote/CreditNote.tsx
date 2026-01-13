import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Flex,
  Separator,
  Skeleton,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { BanIcon, CheckCircleIcon, ChevronDown, Trash2Icon } from 'lucide-react'
import { Fragment, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { AddressLinesCompact } from '@/features/customers/cards/address/AddressCard'
import { getCountryName } from '@/features/settings/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrency, rateToPercent } from '@/lib/utils/numbers'
import { InvoiceConfirmationDialog } from '@/pages/tenants/invoice/InvoiceConfirmationDialog'
import {
  deleteDraftCreditNote,
  finalizeCreditNote,
  getCreditNote,
  listCreditNotes,
  previewCreditNoteSvg,
  voidCreditNote,
} from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'
import { CreditNoteStatus, DetailedCreditNote } from '@/rpc/api/creditnotes/v1/models_pb'
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

const CreditNotePreviewFrame: React.FC<{ creditNoteId: string }> = ({ creditNoteId }) => {
  const previewQuery = useQuery(
    previewCreditNoteSvg,
    { id: creditNoteId },
    { refetchOnMount: 'always' }
  )

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
        className="flex flex-col items-center justify-center gap-5 bg-gray-100 py-10 relative"
        style={{ minHeight: 'fit-content' }}
      >
        {svgContents.map((svgContent, i) => (
          <div
            className="bg-white"
            key={`svg-${i}`}
            style={{
              width: 'fit-content',
              boxShadow: '0 2px 8px rgba(0,0,0,0.1)',
            }}
            dangerouslySetInnerHTML={{ __html: svgContent }}
          />
        ))}
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

  const [showFinalizeConfirmation, setShowFinalizeConfirmation] = useState(false)
  const [showVoidConfirmation, setShowVoidConfirmation] = useState(false)
  const [showDeleteConfirmation, setShowDeleteConfirmation] = useState(false)

  const finalizeMutation = useMutation(finalizeCreditNote, {
    onSuccess: async () => {
      toast.success('Credit note finalized')
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getCreditNote, { id: creditNoteId }),
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
        queryKey: createConnectQueryKey(getCreditNote, { id: creditNoteId }),
      })
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listCreditNotes, {}),
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
        queryKey: createConnectQueryKey(listCreditNotes, {}),
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
  const canFinalize = isDraft
  const canVoid = isDraft || isFinalized
  const canDelete = isDraft

  return (
    <Flex className="h-full">
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
      <Flex direction="column" className="w-1/3 border-r border-border">
        {/* Fixed Header */}
        <Flex direction="column" className="gap-2 p-6 border-b border-border">
          <div className="flex justify-between items-center">
            <div className="flex items-center gap-3">
              <CreditNoteStatusBadge status={creditNote.status} />
              <div className="text-lg font-medium">Credit Note {creditNote.creditNoteNumber}</div>
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary" size="sm" hasIcon>
                  Actions
                  <ChevronDown className="w-4 h-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem
                  disabled={!canFinalize}
                  onClick={() => setShowFinalizeConfirmation(true)}
                >
                  <CheckCircleIcon size="16" className="mr-2" />
                  Finalize
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

          <Separator className="-my-3" />

          <Flex direction="column" className="gap-2 p-6">
            <div className="text-[15px] font-medium">Timeline</div>
            <div className="space-y-2">
              <div className="flex items-start gap-2">
                <div className="w-1.5 h-1.5 rounded-full bg-muted-foreground mt-1.5 flex-shrink-0"></div>
                <div>
                  <div className="text-[13px] font-medium">Credit Note Created</div>
                  <div className="text-[11px] text-muted-foreground">
                    {parseAndFormatDate(creditNote.createdAt)}
                  </div>
                </div>
              </div>
              {creditNote.finalizedAt && (
                <div className="flex items-start gap-2">
                  <div className="w-1.5 h-1.5 rounded-full bg-success mt-1.5 flex-shrink-0"></div>
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
                  <div className="w-1.5 h-1.5 rounded-full bg-red-500 mt-1.5 flex-shrink-0"></div>
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
      <div className="w-2/3 flex flex-col">
        <div className="flex-1 overflow-auto p-6">
          <CreditNotePreviewFrame creditNoteId={creditNoteId} />
        </div>
      </div>
    </Flex>
  )
}

const CreditNoteStatusBadge = ({ status }: { status: CreditNoteStatus }) => {
  switch (status) {
    case CreditNoteStatus.DRAFT:
      return <Badge variant="primary">Draft</Badge>
    case CreditNoteStatus.FINALIZED:
      return <Badge variant="success">Finalized</Badge>
    case CreditNoteStatus.VOIDED:
      return <Badge variant="secondary">Voided</Badge>
    default:
      return null
  }
}

const CreditNoteSummaryLines: React.FC<{ creditNote: DetailedCreditNote }> = ({ creditNote }) => {
  const subtotal = Math.abs(Number(creditNote.subtotal)) || 0
  const taxAmount = Math.abs(Number(creditNote.taxAmount)) || 0
  const total = Math.abs(Number(creditNote.total)) || 0

  return (
    <div className="space-y-1">
      <FlexDetails title="Subtotal" value={formatCurrency(subtotal, creditNote.currency)} />

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
    </div>
  )
}

const CreditNoteLineItems: React.FC<{ items: LineItem[]; currency: string }> = ({
  items,
  currency,
}) => {
  return (
    <div className="space-y-2">
      {items
        .sort((a, b) => a.name.localeCompare(b.name))
        .map(item => (
          <div key={item.localId} className="py-2">
            <div className="flex justify-between items-start gap-2">
              <div className="flex-1 min-w-0">
                <div className="text-[13px] font-medium break-words">{item.name}</div>
                {item.startDate && item.endDate && (
                  <div className="text-[11px] text-muted-foreground mt-1">
                    {parseAndFormatDate(item.startDate)} → {parseAndFormatDate(item.endDate)}
                  </div>
                )}
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
