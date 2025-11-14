import { useMutation, useQuery } from '@connectrpc/connect-query'
import { DialogDescription, DialogTitle, Input, Label, Modal } from '@md/ui'
import { InfoIcon, Loader2, TrendingDown, TrendingUp } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { SlotUpgradeBillingMode } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  previewSlotUpdate,
  updateSlots,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { formatCurrencyNoRounding } from '@/utils/numbers'

interface UpdateSlotModalProps {
  subscriptionId: string
  priceComponentId: string
  unit: string
  currentSlots: number
  unitRate: string
  currency: string
  minSlots?: number
  maxSlots?: number
  onClose: () => void
  onSuccess?: (newValue: number) => void
}

type BillingModeOption = 'optimistic' | 'invoice_paid'

export const UpdateSlotModal = ({
  subscriptionId,
  priceComponentId,
  unit,
  currentSlots,
  unitRate,
  currency,
  minSlots,
  maxSlots,
  onClose,
  onSuccess,
}: UpdateSlotModalProps) => {
  const [newSlots, setNewSlots] = useState<number>(currentSlots)
  const [billingMode, setBillingMode] = useState<BillingModeOption>('optimistic')
  const delta = newSlots - currentSlots
  const isUpgrade = delta > 0

  // Fetch accurate preview when delta changes
  const previewQuery = useQuery(
    previewSlotUpdate,
    {
      subscriptionId,
      priceComponentId,
      delta,
    },
    {
      enabled: delta !== 0,
    }
  )

  const updateMutation = useMutation(updateSlots, {
    onSuccess: data => {
      const action = delta > 0 ? 'upgraded' : 'downgraded'
      toast.success(`Successfully ${action} ${unit} count to ${data.currentValue}`)
      onSuccess?.(data.currentValue)
      onClose()
    },
    onError: error => {
      toast.error(
        `Failed to update ${unit}: ${error instanceof Error ? error.message : 'Unknown error'}`
      )
    },
  })

  const onConfirm = async () => {
    if (delta === 0) {
      toast.error('No change in slot count')
      return
    }

    if (minSlots !== undefined && newSlots < minSlots) {
      toast.error(`Minimum ${unit} count is ${minSlots}`)
      return
    }

    if (maxSlots !== undefined && newSlots > maxSlots) {
      toast.error(`Maximum ${unit} count is ${maxSlots}`)
      return
    }

    try {
      const billingModeEnum =
        billingMode === 'optimistic'
          ? SlotUpgradeBillingMode.SLOT_OPTIMISTIC
          : SlotUpgradeBillingMode.SLOT_ON_INVOICE_PAID

      await updateMutation.mutateAsync({
        subscriptionId,
        priceComponentId,
        delta,
        billingMode: billingModeEnum,
      })
    } catch (error) {
      console.error('Update slots error:', error)
    }
  }

  const unitRateNum = Number(unitRate)

  const preview = previewQuery.data
  const isLoadingPreview = previewQuery.isLoading && delta !== 0

  const proratedAmount = preview ? Number(preview.proratedAmount) : Math.abs(delta) * unitRateNum
  const fullPeriodAmount = preview
    ? Math.abs(Number(preview.fullPeriodAmount))
    : Math.abs(delta) * unitRateNum
  const daysRemaining = preview?.daysRemaining ?? 0

  const newTotalCost = newSlots * unitRateNum

  return (
    <Modal
      header={
        <>
          <DialogTitle className="text-base font-semibold">Update {unit} count</DialogTitle>
          <DialogDescription className="text-sm text-muted-foreground mt-1">
            Current: {currentSlots} {unit}(s) • {formatCurrencyNoRounding(unitRateNum, currency)}{' '}
            per {unit}
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={false}
      onCancel={onClose}
      onConfirm={onConfirm}
      confirmText={updateMutation.isPending ? 'Updating...' : 'Confirm Update'}
      confirmDisabled={updateMutation.isPending || delta === 0}
    >
      <Modal.Content>
        <div className="space-y-4 py-4">
          <div>
            <Label htmlFor="new-slots" className="text-sm font-medium mb-2 block">
              New {unit} count
            </Label>
            <Input
              id="new-slots"
              type="number"
              value={newSlots}
              onChange={e => setNewSlots(parseInt(e.target.value) || 0)}
              min={minSlots ?? 0}
              max={maxSlots}
              className="text-base"
            />
          </div>

          {delta !== 0 && (
            <div className="rounded-lg border border-border bg-muted/30 p-4">
              {isLoadingPreview ? (
                <div className="flex items-center justify-center py-6">
                  <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                  <span className="ml-2 text-sm text-muted-foreground">
                    Calculating proration...
                  </span>
                </div>
              ) : (
                <>
                  <div className="flex items-start justify-between mb-3">
                    <div className="flex-1">
                      <div className="text-sm text-muted-foreground mb-1">
                        {delta > 0 ? 'Adding' : 'Removing'} {Math.abs(delta)} {unit}(s)
                      </div>
                      <div className="text-2xl font-semibold tabular-nums">
                        {delta > 0 ? '+' : '-'}
                        {formatCurrencyNoRounding(fullPeriodAmount, currency)}
                        <span className="text-sm font-normal text-muted-foreground ml-1">
                          / month
                        </span>
                      </div>
                      {isUpgrade && daysRemaining > 0 && (
                        <div className="text-xs text-muted-foreground mt-2 space-y-0.5">
                          <div className="font-medium">
                            Prorated charge today:{' '}
                            {formatCurrencyNoRounding(proratedAmount, currency)}
                          </div>
                          <div>{daysRemaining} days remaining in period</div>
                        </div>
                      )}
                    </div>
                    {delta > 0 ? (
                      <TrendingUp className="w-5 h-5 text-success" />
                    ) : (
                      <TrendingDown className="w-5 h-5 text-warning" />
                    )}
                  </div>

                  <div className="text-xs text-muted-foreground border-t border-border pt-2">
                    New monthly total: {formatCurrencyNoRounding(newTotalCost, currency)}
                  </div>
                </>
              )}
            </div>
          )}

          {isUpgrade && (
            <div>
              <Label className="text-sm font-medium mb-2 block">Billing & Activation</Label>
              <div className="space-y-2">
                <label
                  className={`flex items-start gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
                    billingMode === 'optimistic'
                      ? 'border-primary bg-primary/5'
                      : 'border-border hover:border-primary/50'
                  }`}
                >
                  <input
                    type="radio"
                    name="billing-mode"
                    value="optimistic"
                    checked={billingMode === 'optimistic'}
                    onChange={e => setBillingMode(e.target.value as BillingModeOption)}
                    className="mt-0.5"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium">Optimistic</div>
                    <div className="text-xs text-muted-foreground mt-0.5">
                      Activated immediately. <br />
                      An adjustment invoice is emitted for the prorated amount for this period.
                      {!isLoadingPreview && proratedAmount > 0 && (
                        <>
                          <br />
                          Adjustment invoice amount (excl. tax):{' '}
                          {formatCurrencyNoRounding(proratedAmount, currency)}
                        </>
                      )}
                    </div>
                  </div>
                </label>

                <label
                  className={`flex items-start gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
                    billingMode === 'invoice_paid'
                      ? 'border-primary bg-primary/5'
                      : 'border-border hover:border-primary/50'
                  }`}
                >
                  <input
                    type="radio"
                    name="billing-mode"
                    value="invoice_paid"
                    checked={billingMode === 'invoice_paid'}
                    onChange={e => setBillingMode(e.target.value as BillingModeOption)}
                    className="mt-0.5"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium">After payment</div>
                    <div className="text-xs text-muted-foreground mt-0.5">
                      Activated on paid. <br />
                      An adjustment invoice is emitted for the prorated amount for this period.
                      {!isLoadingPreview && proratedAmount > 0 && (
                        <>
                          <br />
                          Adjustment invoice amount (excl. tax):{' '}
                          {formatCurrencyNoRounding(proratedAmount, currency)}
                        </>
                      )}
                    </div>
                  </div>
                </label>
              </div>
            </div>
          )}

          {delta < 0 && (
            <div className="flex gap-2 items-center text-xs text-muted-foreground bg-muted/30 rounded-lg p-3 border border-border">
              <span>
                <InfoIcon size={14} />
              </span>
              <span>
                This downgrade will take effect at the end of the current billing period. The
                customer will continue to have access to {currentSlots} {unit}(s) until then.
              </span>
            </div>
          )}
        </div>
      </Modal.Content>
    </Modal>
  )
}
