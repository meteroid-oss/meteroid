import { Checkbox } from '@md/ui'
import { useState } from 'react'

import { templateFor } from '@/features/plans/canvas/chargeFormat'
import {
  optionCell,
  termFormValue,
} from '@/features/plans/currencies/priceCells'
import { usePlanOverview } from '@/features/plans/hooks/usePlan'
import { useComponentMetric } from '@/features/plans/pricecomponents/componentRefs'
import { buildPriceInputs } from '@/features/pricing/conversions'
import { buildDefaultsFromPrices } from '@/features/pricing/formData'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import { ChargeForm, useChargeLock } from './ChargeForm'
import { ChargeNameInput, ContextChip } from './InlineChargeFields'
import { chargeTarget, decompileCharge } from './compile'

import type { ChargeFormResult, StoredCharge } from './ChargeForm'
import type { ChargeMutations } from './useChargeMutations'
import type { OptionLike } from '@/features/plans/currencies/priceCells'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

type Props = {
  component: PriceComponent
  currency: string
  /** The option whose cell is edited; a merged cell opens on the first one. */
  option: OptionLike
  options: OptionLike[]
  /** Opened on the cell every option shares: saving keeps it shared unless split. */
  merged: boolean
  mutations: ChargeMutations
  onClose: () => void
}

/** A charge edited in place, in its product block: its price for one option, in one currency. */
export const InlineChargeEditor = ({
  component,
  currency,
  option,
  options,
  merged,
  mutations,
  onClose,
}: Props) => {
  const confirm = useConfirmationModal()
  const familyLocalId = usePlanOverview()?.productFamilyLocalId
  const { feeType, structural, lock } = useChargeLock(component)
  const metric = useComponentMetric(structural.metricId)
  const wasIncluded = component.prices.length === 0
  const [same, setSame] = useState(merged)
  // Read once: the cell this editor opened on.
  const [cell] = useState(() => optionCell(component, option.id, currency))

  // An empty cell starts from the closest other one: its structure, amounts to re-enter.
  const [from] = useState(() => {
    if (cell.length) return undefined
    const sibling = options.find(o => optionCell(component, o.id, currency).length > 0)
    if (sibling) return { prices: optionCell(component, sibling.id, currency), label: `${sibling.name} one` }
    const closest = templateFor(component.prices, currency, undefined)
    return closest ? { prices: [closest], label: `${closest.currency} one` } : undefined
  })
  const template: Price[] = cell.length ? cell : (from?.prices ?? [])
  const cadence = template[0]?.cadence ?? BillingPeriod.MONTHLY

  const [initial] = useState((): StoredCharge => ({
    feeType,
    formData: {
      ...buildDefaultsFromPrices(feeType, template),
      metricId: structural.metricId,
      slotUnitName: structural.slotUnitName,
      billingType: structural.billingType,
      term: termFormValue(cadence),
    },
  }))
  const [initialShape] = useState(() => decompileCharge(initial.feeType, initial.formData).shape)
  const [name, setName] = useState(component.name)

  const trimmed = name.trim() || component.name
  const renamed = trimmed !== component.name
  // The option names the price edited once the plan has several and the cell is its own.
  const own = options.length > 1 && !same
  const where = own ? `${option.name} ${currency}` : currency
  const chip = own ? `${option.name} · ${currency}` : options.length > 1 ? `Every option · ${currency}` : currency
  const targets = same ? [] : [option.id]

  const restructure = async (inputs: ReturnType<typeof buildPriceInputs>) => {
    await mutations.restructure(component, trimmed, inputs)
    onClose()
  }

  const save = (result: ChargeFormResult) => {
    if (result.included) {
      if (wasIncluded) {
        if (renamed) mutations.rename(component, trimmed).then(onClose, () => undefined)
        else onClose()
        return
      }
      confirm(() => void restructure([]).catch(() => undefined), {
        header: `Include ${trimmed} without a price`,
        message: 'Its prices in every currency and option are removed. Its features stay with the plan.',
      })
      return
    }
    // Rates in the plan and a rate card's are one model kind, priced apart.
    const sameKind =
      !wasIncluded &&
      chargeTarget(result.shape, lock).kind === chargeTarget(initialShape, lock).kind &&
      result.formData.usageModel === initial.formData.usageModel
    const inputs = buildPriceInputs(result.pricingType, result.formData, currency)
    if (!inputs[0]) return
    if (!sameKind) {
      const run = () => void restructure(inputs).catch(() => undefined)
      if (wasIncluded) return run()
      confirm(run, {
        header: `Change how ${trimmed} is priced`,
        message: `Its prices in every currency and option are replaced by this one, in ${currency}. Price it again where it is sold.`,
      })
      return
    }
    const write = async () => {
      if (renamed) await mutations.rename(component, trimmed)
      // A capacity charge's cell is all its levels, written at once: those removed leave it.
      if (result.feeType === 'capacity') await mutations.setCellPrices(component, targets, inputs)
      else await mutations.setCell(component, targets, inputs[0])
      onClose()
    }
    write().catch(() => undefined)
  }

  const clearCell = () => {
    const run = () =>
      void mutations
        .clearCell(component, targets, currency)
        .then(onClose)
        .catch(() => undefined)
    confirm(
      run,
      own
        ? {
            header: `Remove the ${where} price of ${component.name}`,
            message: `${option.name} isn't sold in ${currency} until it has one again.`,
          }
        : {
            header: `Stop selling ${component.name} in ${currency}`,
            message: `Its price in ${currency} is removed: customers billed in ${currency} can't buy the plan until it has one.`,
          }
    )
  }

  return (
    <div className="space-y-4 bg-muted/30 px-4 py-4">
      <div className="flex items-end gap-3">
        <ChargeNameInput value={name} onChange={setName} autoFocus />
        <ContextChip>{chip}</ContextChip>
      </div>
      <ChargeForm
        currency={currency}
        cadence={cadence}
        name={trimmed}
        lock={lock}
        product={{ metric, metricId: structural.metricId, slotUnitName: structural.slotUnitName }}
        newProduct={false}
        initial={initial}
        familyLocalId={familyLocalId}
        includable
        startsIncluded={wasIncluded}
        manyLevels
        notice={({ kind }) => {
          const included = kind === 'included'
          return (
            <>
              {merged && !included && (
                <label className="flex items-center gap-2 text-sm">
                  <Checkbox checked={same} onCheckedChange={checked => setSame(checked === true)} />
                  Same in every option
                  {!same && (
                    <span className="text-xs text-muted-foreground">
                      · saved for {option.name} only, the others keep this price
                    </span>
                  )}
                </label>
              )}
              {from && !included && (
                <p role="status" className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs">
                  No {where} price yet: starting from the {from.label}. Check the amounts before saving.
                </p>
              )}
            </>
          )
        }}
        submitLabel="Save"
        pending={mutations.pending}
        remove={
          cell.length > 0
            ? { label: 'Remove this price', onClick: clearCell, disabled: mutations.pending }
            : undefined
        }
        onCancel={onClose}
        onSubmit={save}
      />
    </div>
  )
}
