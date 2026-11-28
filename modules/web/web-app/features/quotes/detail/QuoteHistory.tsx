import {
  Badge,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { cn } from '@ui/lib'
import { useMemo, useState } from 'react'

import {
  QuoteContractValueSection,
  QuotePriceSchedule,
} from '@/features/quotes/QuotePriceSchedule'
import { formatDiscount } from '@/features/quotes/detail/QuoteLinesTable'
import { QuoteMarkdown } from '@/features/quotes/detail/QuoteMarkdown'
import { shortHash } from '@/features/quotes/detail/quoteFormat'
import { parseRevisionDocument } from '@/features/quotes/detail/revisionDocument'
import { quoteTrialLabel } from '@/features/quotes/quoteTrial'
import { useQuery } from '@/lib/connectrpc'
import { QuoteRevision } from '@/rpc/api/quotes/v1/models_pb'
import { listQuoteRevisions } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { listUsers } from '@/rpc/api/users/v1/users-UsersService_connectquery'
import { formatDateOnly, parseAndFormatDateTime } from '@/utils/date'

export const useUserLabel = () => {
  const users = useQuery(listUsers).data?.users
  return useMemo(() => {
    const byId = new Map(
      (users ?? []).map(u => [u.id, [u.firstName, u.lastName].filter(Boolean).join(' ') || u.email])
    )
    return (id?: string) => (id ? (byId.get(id) ?? id) : '—')
  }, [users])
}

const Field = ({ label, value }: { label: string; value?: string | number }) =>
  value === undefined || value === '' ? null : (
    <div className="flex justify-between gap-4 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className="text-right">{value}</span>
    </div>
  )

const RevisionDocument = ({ revision }: { revision: QuoteRevision }) => {
  const summary = useMemo(() => parseRevisionDocument(revision.document), [revision.document])
  if (!summary) {
    return <div className="text-sm text-muted-foreground">Unreadable revision</div>
  }

  return (
    <div className="space-y-4 rounded-lg border border-border p-4">
      <div className="flex items-center justify-between">
        <div className="font-medium">
          Quote {summary.quoteNumber} — revision {revision.revision}
        </div>
        <span className="font-mono text-xs text-muted-foreground" title={revision.documentHash}>
          {shortHash(revision.documentHash)}
        </span>
      </div>
      <div className="grid gap-1 md:grid-cols-2 md:gap-x-8">
        <Field label="Seller" value={summary.seller} />
        <Field label="Customer" value={summary.customer} />
        <Field label="Currency" value={summary.currency} />
        <Field label="Start" value={summary.startDate && formatDateOnly(summary.startDate)} />
        <Field label="End" value={summary.endDate && formatDateOnly(summary.endDate)} />
        <Field label="Expires" value={summary.expiresAt && formatDateOnly(summary.expiresAt)} />
        <Field
          label="Trial"
          value={quoteTrialLabel({
            days: summary.trialDays,
            free: summary.trialFree,
            manual: summary.trialManual,
          })}
        />
        <Field
          label="Payment terms"
          value={
            summary.netTerms === undefined
              ? undefined
              : summary.netTerms === 0
                ? 'Due on receipt'
                : `Net ${summary.netTerms} days`
          }
        />
      </div>

      <div>
        <div className="text-[13px] font-medium mb-1">Lines</div>
        {summary.lines.length === 0 ? (
          <div className="text-sm text-muted-foreground">No lines</div>
        ) : (
          <div className="divide-y divide-border">
            {summary.lines.map((line, i) => (
              <div key={i} className="flex justify-between gap-4 py-2 text-sm">
                <div className="min-w-0">
                  <div className="font-medium">{line.name}</div>
                  {line.description && (
                    <div className="text-xs text-muted-foreground">{line.description}</div>
                  )}
                  <div className="text-xs text-muted-foreground">{line.period}</div>
                </div>
                <div className="text-right shrink-0">
                  <div>{line.price}</div>
                  {line.discountPercent && Number(line.discountPercent) > 0 && (
                    <div className="text-xs text-muted-foreground">
                      {line.listPrice && (
                        <span className="line-through mr-1">{line.listPrice}</span>
                      )}
                      {formatDiscount(line.discountPercent)}
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {summary.coupons.length > 0 && (
        <div>
          <div className="text-[13px] font-medium mb-1">Discounts</div>
          <div className="divide-y divide-border">
            {summary.coupons.map((coupon, i) => (
              <div key={i} className="flex justify-between gap-4 py-2 text-sm">
                <div className="min-w-0">
                  <div className="font-mono font-medium">{coupon.code}</div>
                  {coupon.description && (
                    <div className="text-xs text-muted-foreground">{coupon.description}</div>
                  )}
                </div>
                <div className="text-right shrink-0">
                  <div>{coupon.discount}</div>
                  {coupon.duration && (
                    <div className="text-xs text-muted-foreground">{coupon.duration}</div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {summary.currency && (
        <>
          <QuotePriceSchedule
            steps={revision.priceSchedule}
            contract={revision.contractValue}
            currency={summary.currency}
          />
          <QuoteContractValueSection value={revision.contractValue} currency={summary.currency} />
        </>
      )}

      {summary.sections.map((section, i) => (
        <div key={i}>
          <div className="text-[13px] font-medium mb-1">{section.title}</div>
          <QuoteMarkdown source={section.body} />
        </div>
      ))}
      {summary.overview && (
        <div>
          <div className="text-[13px] font-medium mb-1">Overview</div>
          <p className="text-sm text-muted-foreground whitespace-pre-line">{summary.overview}</p>
        </div>
      )}
      {summary.terms && (
        <div>
          <div className="text-[13px] font-medium mb-1">Terms & Services</div>
          <p className="text-sm text-muted-foreground whitespace-pre-line">{summary.terms}</p>
        </div>
      )}
      <div className="grid gap-1">
        <Field label="Recipients" value={summary.recipients.join(', ')} />
        <Field label="Countersigners" value={summary.countersigners.join(', ')} />
        <Field label="Consent text" value={summary.consentText} />
      </div>
    </div>
  )
}

export const QuoteHistory = ({ quoteId }: { quoteId: string }) => {
  const query = useQuery(listQuoteRevisions, { quoteId })
  const userLabel = useUserLabel()
  const revisions = useMemo(
    () => [...(query.data?.revisions ?? [])].sort((a, b) => b.revision - a.revision),
    [query.data]
  )
  const [selected, setSelected] = useState<number>()
  const current = revisions.find(r => r.revision === selected) ?? revisions[0]

  if (query.isLoading) return <Skeleton height={120} />
  if (revisions.length === 0) {
    return <div className="text-sm text-muted-foreground py-6 text-center">No revisions yet</div>
  }

  return (
    <div className="space-y-4">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Revision</TableHead>
            <TableHead>Created</TableHead>
            <TableHead>By</TableHead>
            <TableHead>Hash</TableHead>
            <TableHead />
          </TableRow>
        </TableHeader>
        <TableBody>
          {revisions.map(revision => (
            <TableRow
              key={revision.revision}
              onClick={() => setSelected(revision.revision)}
              className={cn('cursor-pointer', current === revision && 'bg-muted/60')}
            >
              <TableCell>Rev {revision.revision}</TableCell>
              <TableCell>{parseAndFormatDateTime(revision.createdAt)}</TableCell>
              <TableCell>{userLabel(revision.createdBy)}</TableCell>
              <TableCell className="font-mono text-xs" title={revision.documentHash}>
                {shortHash(revision.documentHash)}
              </TableCell>
              <TableCell className="text-right">
                {revision.supersededAt ? (
                  <Badge
                    variant="outline"
                    size="sm"
                    title={parseAndFormatDateTime(revision.supersededAt)}
                  >
                    Superseded
                  </Badge>
                ) : (
                  <Badge variant="success" size="sm">
                    Current
                  </Badge>
                )}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
      {current && <RevisionDocument revision={current} />}
    </div>
  )
}
