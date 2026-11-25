import {
  Button,
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { PlusIcon } from 'lucide-react'
import { useState } from 'react'

import { Gate } from '@/components/Gate'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { useQuery } from '@/lib/connectrpc'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { listRateCards } from '@/rpc/api/ratecards/v1/ratecards-RateCardsService_connectquery'

import { NewRateCardDialog } from './NewRateCardDialog'
import { RateCardSheet, unmatchedLabel } from './RateCardSheet'
import { dimensionName, unitOf, unitSizeLabel } from './rateTable'

/**
 * The rate cards of a metric: a card rates its usage by dimensions (model, token type) for every
 * plan that prices the metric with it. Only a sum or a count without a matrix can have one.
 */
export const MetricRateCards = ({
  metricId,
  blockedReason,
}: {
  metricId: string
  /** Why the metric can't get new cards; unset when it can. */
  blockedReason?: string
}) => {
  const eligible = !blockedReason
  const cardsQuery = useQuery(listRateCards, { billableMetricId: metricId })
  const cards = cardsQuery.data?.cards ?? []
  const unit = useQuery(getBillableMetric, { id: metricId }).data?.billableMetric?.unitLabel
  const creditUnits = useCreditUnits()
  const [creating, setCreating] = useState(false)
  const [openId, setOpenId] = useState<string | null>(null)

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-4">
        <p className="text-xs text-muted-foreground">
          {eligible
            ? 'Rates this metric’s usage by its dimensions, such as model and token type, for every plan that prices it with a card.'
            : blockedReason}
        </p>
        {eligible && (
          <Gate perm="catalog:manage">
            <Button size="sm" variant="outline" hasIcon onClick={() => setCreating(true)}>
              <PlusIcon className="h-4 w-4" />
              New rate card
            </Button>
          </Gate>
        )}
      </div>
      {cardsQuery.isLoading ? (
        <div className="p-4 text-center">
          <Spinner />
        </div>
      ) : cards.length === 0 ? (
        eligible && <p className="text-sm text-muted-foreground">No rate card yet.</p>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Code</TableHead>
              <TableHead>Name</TableHead>
              <TableHead>Dimensions</TableHead>
              <TableHead>Unit</TableHead>
              <TableHead>Unrated usage</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {cards.map(card => {
              const credit = creditUnits?.find(u => u.id === card.creditUnitId)
              return (
                <TableRow
                  key={card.id}
                  className="cursor-pointer"
                  onClick={() => setOpenId(card.id)}
                >
                  <TableCell className="font-mono">{card.code}</TableCell>
                  <TableCell>{card.name}</TableCell>
                  <TableCell>{card.dimensions.map(dimensionName).join(', ')}</TableCell>
                  <TableCell>
                    {unitSizeLabel(card.unitSize, unitOf(unit))}
                    {card.creditUnitId && (
                      <span className="text-muted-foreground">
                        {' '}
                        in {credit?.pluralName || credit?.name || 'credits'}
                      </span>
                    )}
                  </TableCell>
                  <TableCell>{unmatchedLabel(card)}</TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      )}
      {creating && (
        <NewRateCardDialog
          metricId={metricId}
          onClose={() => setCreating(false)}
          onCreated={card => {
            setCreating(false)
            setOpenId(card.id)
          }}
        />
      )}
      {openId && <RateCardSheet id={openId} onClose={() => setOpenId(null)} />}
    </div>
  )
}
