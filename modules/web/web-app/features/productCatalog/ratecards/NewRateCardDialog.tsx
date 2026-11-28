import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  GenericFormField,
  Input,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { AutocompleteTags, type AutocompleteItem } from '@/components/form/Autocomplete'
import { slugifyCode } from '@/features/entitlements/utils'
import { useCreditUnit } from '@/features/pricing/CreditUnitPriceFields'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { discoverProperties } from '@/rpc/api/eventinsights/v1/eventinsights-EventInsightsService_connectquery'
import { RateCardUnmatched, type RateCard } from '@/rpc/api/ratecards/v1/models_pb'
import {
  createRateCard,
  listRateCards,
  setRateCardRates,
} from '@/rpc/api/ratecards/v1/ratecards-RateCardsService_connectquery'

import { priceRatesForCard } from './bridge'
import { UNIT_SIZES, dimensionName, listNames, unitSizeLabel } from './rateTable'

import type { PriceRates } from './bridge'

/** What a card is made of, as the dialog collects it. */
export type NewCard = {
  code: string
  name: string
  metricId: string
  dimensions: string[]
  unitSize: string
}

const schema = z.object({
  name: z.string().trim().min(1, 'Name is required'),
  code: z
    .string()
    .trim()
    .regex(/^[a-z0-9][a-z0-9_-]*$/, "Lowercase letters, digits, '-', '_'"),
  dimensions: z
    .array(z.string())
    .min(1, 'At least one dimension')
    .max(4, 'At most four dimensions'),
  unitSize: z.string(),
})

/** The metric's event properties a card can rate by: those it already names, then the seen. */
function useMetricKeys(metricId: string) {
  const metric = useQuery(getBillableMetric, { id: metricId }).data?.billableMetric
  const discovered = useQuery(
    discoverProperties,
    { eventCode: metric?.code ?? '', lookbackDays: 90, limit: 100 },
    { enabled: Boolean(metric?.code) }
  )
  const items = useMemo<AutocompleteItem[]>(() => {
    const measured = metric?.aggregation?.aggregationKey
    const named = [
      metric?.usageGroupKey,
      metric?.usageBreakdownKey,
      ...(metric?.filters ?? []).map(filter => filter.property),
    ].filter((key): key is string => Boolean(key))
    const seen = (discovered.data?.properties ?? []).filter(p => p.key !== measured)
    const keys = [...new Set([...named, ...seen.map(p => p.key)])]
    return keys.map(key => {
      const property = seen.find(p => p.key === key)
      return {
        value: key,
        keywords: property?.sampleValues,
        node: (
          <>
            <span className="truncate font-mono">{key}</span>
            {property && (
              <span className="ml-auto shrink-0 pl-2 text-[10px] text-muted-foreground">
                {property.sampleValues.slice(0, 3).join(', ')}
              </span>
            )}
          </>
        ),
      }
    })
  }, [metric, discovered.data])
  return { metric, items, isLoading: discovered.isLoading && Boolean(metric?.code) }
}

/**
 * The few choices a rate card is made of; its rates are entered in its table once created. A
 * card made from a price in credits rates in the price's credit unit, any other in money. Made
 * `from` a price's own rates, it takes their dimensions and rates, in the price's `currency`.
 */
export const NewRateCardDialog = ({
  metricId,
  creditUnitId,
  from,
  currency,
  create: createCard,
  title,
  onClose,
  onCreated,
}: {
  metricId: string
  creditUnitId?: string
  from?: PriceRates
  currency?: string
  /** Creates the card instead of core's rate card API: a module serving its own cards. */
  create?: (card: NewCard) => Promise<RateCard | undefined>
  title?: string
  onClose: () => void
  onCreated: (card: RateCard) => void
}) => {
  const queryClient = useQueryClient()
  const { metric, items, isLoading } = useMetricKeys(metricId)
  const creditUnit = useCreditUnit(creditUnitId)
  const [codeEdited, setCodeEdited] = useState(false)
  const methods = useZodForm({
    schema,
    defaultValues: {
      name: '',
      code: '',
      dimensions: from?.dimensions.map(d => d.trim()) ?? [],
      unitSize: from?.unitSize ?? '1',
    },
  })
  const listed = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: listRateCards, cardinality: undefined }),
    })
  const create = useMutation(createRateCard)
  const setRates = useMutation(setRateCardRates)
  const unit = metric?.unitLabel

  const submit = async (data: z.infer<typeof schema>) => {
    const fields = {
      code: data.code,
      name: data.name,
      metricId,
      dimensions: data.dimensions,
      unitSize: data.unitSize,
    }
    const card = await (
      createCard
        ? createCard(fields)
        : create
            .mutateAsync({
              ...fields,
              creditUnitId,
              onUnmatched:
                from?.onUnmatched === 'ignore' ? RateCardUnmatched.IGNORE : RateCardUnmatched.HOLD,
            })
            .then(created => created.card)
    ).catch(error => {
      toast.error(rpcErrorMessage(error, 'The rate card could not be created'))
      return undefined
    })
    if (!card) return
    // Its first rates, from today: the price's, as they are.
    const rated =
      !from ||
      (await setRates
        .mutateAsync({
          rateCardId: card.id,
          rates: priceRatesForCard(from.keyRates, currency ?? '', creditUnitId),
        })
        .then(
          () => true,
          error => {
            toast.error(rpcErrorMessage(error, 'The rate card was created without its rates'))
            return false
          }
        ))
    if (!createCard) await listed()
    if (rated)
      toast.success(
        from ? "Rate card created with the price's rates" : `${title ?? 'Rate card'} created`
      )
    onCreated(card)
  }

  return (
    <Dialog open onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[460px]">
        <DialogHeader>
          <DialogTitle>
            {from ? 'Save as a rate card' : `New ${(title ?? 'Rate card').toLowerCase()}`}
          </DialogTitle>
        </DialogHeader>
        <Form {...methods}>
          <form
            className="space-y-4"
            onSubmit={event => {
              // The dialog renders inside a price form's tree: its submit must not reach it.
              event.stopPropagation()
              methods.handleSubmit(data => submit(data))(event)
            }}
          >
            <div className="grid grid-cols-[1fr_10rem] gap-3">
              <GenericFormField
                name="name"
                label="Name"
                control={methods.control}
                render={({ field }) => (
                  <Input
                    {...field}
                    placeholder="LLM tokens"
                    autoFocus
                    onChange={event => {
                      field.onChange(event)
                      if (!codeEdited) methods.setValue('code', slugifyCode(event.target.value))
                    }}
                  />
                )}
              />
              <GenericFormField
                name="code"
                label="Code"
                control={methods.control}
                render={({ field }) => (
                  <Input
                    {...field}
                    placeholder="llm_tokens"
                    className="font-mono"
                    onChange={event => {
                      setCodeEdited(true)
                      field.onChange(event)
                    }}
                  />
                )}
              />
            </div>
            {from ? (
              <p className="text-sm text-muted-foreground">
                Rated by {listNames(from.dimensions.map(dimensionName))},{' '}
                {unitSizeLabel(from.unitSize, unit?.singular ? unit : undefined)}:{' '}
                {from.keyRates.filter(row => row.rate?.trim()).length} rates in{' '}
                {creditUnit ? creditUnit.pluralName || creditUnit.name : currency}, from today.
              </p>
            ) : (
              <>
                <GenericFormField
                  name="dimensions"
                  label="Rated by"
                  description="Event properties, such as model and token type. The first decides between rates equally specific."
                  control={methods.control}
                  render={({ field }) => (
                    <AutocompleteTags
                      value={field.value}
                      onChange={field.onChange}
                      items={items}
                      isLoading={isLoading}
                      placeholder="model, token_type"
                      emptyText="No property seen on this metric's events. Type one and press Enter."
                    />
                  )}
                />
                <SelectFormField
                  name="unitSize"
                  label="Rates"
                  control={methods.control}
                  containerClassName="max-w-[12rem]"
                >
                  {UNIT_SIZES.map(size => (
                    <SelectItem key={size.value} value={size.value}>
                      {unit?.singular ? unitSizeLabel(size.value, unit) : size.label}
                    </SelectItem>
                  ))}
                </SelectFormField>
              </>
            )}
            {creditUnit && (
              <p className="text-xs text-muted-foreground">
                In {creditUnit.pluralName || creditUnit.name}, like the price.
              </p>
            )}
            <DialogFooter>
              <Button variant="outline" type="button" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" disabled={create.isPending || setRates.isPending}>
                Create
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
