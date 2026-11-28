import { Badge, Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@md/ui'

import { GrantChips, useGrantNames } from '@/features/offers/GrantChips'
import {
  GRANT_KIND_LABELS,
  GRANT_SOURCE_LABELS,
  KIND_TONES,
  SUBSCRIPTION_GRANT_STATUS,
  grantDefinitionLabel,
  grantFromProto,
  windowLabel,
} from '@/features/offers/grants'
import { useQuery } from '@/lib/connectrpc'
import { formatDateOnly } from '@/lib/utils/date'
import { listGrants, listOffers } from '@/rpc/api/offers/v1/offers-OffersService_connectquery'
import { SubscriptionGrant_Source } from '@/rpc/api/offers/v1/offers_pb'

import { SubscriptionSectionCard } from './SubscriptionSectionCard'

import type { GrantForm } from '@/features/offers/grants'
import type { AppliedCouponDetailed } from '@/rpc/api/coupons/v1/models_pb'
import type { SubscriptionGrant } from '@/rpc/api/offers/v1/offers_pb'

interface Row {
  grant: SubscriptionGrant
  form: GrantForm
}

/** What the subscription was given beside its plan: each grant, its dates and where it came from. */
export const SubscriptionGrantsSection = ({
  subscriptionId,
  appliedCoupons,
}: {
  subscriptionId: string
  appliedCoupons: AppliedCouponDetailed[]
}) => {
  const query = useQuery(listGrants, { subscriptionId })
  const catalog = useQuery(listOffers, { includeArchived: true }).data?.offers ?? []
  const rows: Row[] = (query.data?.grants ?? []).flatMap(grant => {
    const form = grant.grant && grantFromProto(grant.grant)
    return form ? [{ grant, form }] : []
  })
  const names = useGrantNames(rows.map(row => row.form))
  if (rows.length === 0) return null

  const source = (grant: SubscriptionGrant) => {
    const offer = catalog.find(o => o.id === grant.offerId)?.name
    const code = grant.couponId
      ? appliedCoupons.find(c => c.coupon?.id === grant.couponId)?.coupon?.code
      : undefined
    if (grant.source === SubscriptionGrant_Source.COUPON && code) {
      return (
        <>
          Coupon <code className="font-mono text-xs">{code}</code>
          {offer && ` · ${offer}`}
        </>
      )
    }
    return [GRANT_SOURCE_LABELS[grant.source], offer].filter(Boolean).join(' · ')
  }

  return (
    <SubscriptionSectionCard title="Grants">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Grant</TableHead>
            <TableHead>From</TableHead>
            <TableHead>Ends</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Source</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map(({ grant, form }) => {
            const status = SUBSCRIPTION_GRANT_STATUS[grant.status]
            return (
              <TableRow key={`${grant.applicationId}-${grant.grant?.id}`}>
                <TableCell>
                  <div className="flex items-center gap-2">
                    <GrantChips
                      chips={[
                        {
                          key: form.key,
                          label: GRANT_KIND_LABELS[form.kind],
                          tone: KIND_TONES[form.kind],
                        },
                      ]}
                    />
                    <span>{grantDefinitionLabel(form, names)}</span>
                  </div>
                </TableCell>
                <TableCell className="tabular-nums">{formatDateOnly(grant.from)}</TableCell>
                <TableCell className="tabular-nums">
                  {grant.until ? (
                    formatDateOnly(grant.until)
                  ) : (
                    <span className="text-muted-foreground">
                      {form.window.duration === 'invoices' ? windowLabel(form.window) : 'Never'}
                    </span>
                  )}
                </TableCell>
                <TableCell>
                  <Badge variant={status.tone} size="sm">
                    {status.label}
                  </Badge>
                </TableCell>
                <TableCell className="text-muted-foreground">{source(grant)}</TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    </SubscriptionSectionCard>
  )
}
