import { useMutation } from '@connectrpc/connect-query'
import { useState } from 'react'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { formatCurrency } from '@/lib/utils/numbers'
import { redeemCode } from '@/rpc/portal/wallets/v1/wallets-PortalWalletsService_connectquery'

import { date } from './format'
import { useInvalidatePortal } from './hooks'
import { Card, Eyebrow, PButton } from './primitives'
import { customerError, day, errorStyle, fieldStyle, labelStyle } from './walletFormat'

import type { RedeemableSubscription, RedeemedCredit } from '@/rpc/portal/wallets/v1/wallets_pb'

const schema = z.object({
  code: z.string().trim().min(1, 'Enter a code').max(64, 'A code has at most 64 characters'),
  subscriptionId: z.string().optional(),
})

/** Redeems a promo code that adds credit to the customer's balance. A code for one subscription,
 * with several it may apply to, asks which one before redeeming. */
export const RedeemCodeCard = () => {
  const invalidatePortal = useInvalidatePortal()
  const redeemMutation = useMutation(redeemCode)
  const [redeemed, setRedeemed] = useState<RedeemedCredit>()
  const [choices, setChoices] = useState<RedeemableSubscription[]>()
  const [error, setError] = useState<string>()
  const methods = useZodForm({ schema, defaultValues: { code: '', subscriptionId: undefined } })

  const submit = methods.handleSubmit(async values => {
    setError(undefined)
    setRedeemed(undefined)
    try {
      const response = await redeemMutation.mutateAsync({
        code: values.code,
        subscriptionId: choices ? values.subscriptionId : undefined,
      })
      switch (response.outcome.case) {
        case 'redeemed':
          setRedeemed(response.outcome.value)
          setChoices(undefined)
          methods.reset({ code: '', subscriptionId: undefined })
          await invalidatePortal()
          break
        case 'subscriptionRequired': {
          const subscriptions = response.outcome.value.subscriptions
          setChoices(subscriptions)
          methods.setValue('subscriptionId', subscriptions[0]?.id)
          break
        }
        default:
          setError('The code could not be redeemed.')
      }
    } catch (cause) {
      // The server words every refusal (unknown, expired, used, not applicable) for the customer.
      setError(customerError(cause, 'The code could not be redeemed.'))
    }
  })

  const fieldError = methods.formState.errors.code?.message
  const onCodeChange = () => {
    setError(undefined)
    setChoices(undefined)
    methods.setValue('subscriptionId', undefined)
  }

  return (
    <Card>
      <form onSubmit={submit} style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <Eyebrow>Redeem a code</Eyebrow>
          <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>
            Have a promo code? Redeem it to add credit to your balance.
          </span>
        </div>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <input
            style={{ ...fieldStyle, flex: 1, width: 'auto', minWidth: 180 }}
            placeholder="Promo code"
            autoComplete="off"
            spellCheck={false}
            aria-label="Promo code"
            aria-invalid={!!fieldError || !!error}
            {...methods.register('code', { onChange: onCodeChange })}
          />
          <PButton type="submit" loading={redeemMutation.isPending}>
            Redeem
          </PButton>
        </div>
        {choices && (
          <label style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
            <span style={labelStyle}>This code applies to one subscription. Choose which one.</span>
            <select style={fieldStyle} {...methods.register('subscriptionId')}>
              {choices.map(choice => (
                <option key={choice.id} value={choice.id}>
                  {choice.planName} · since {date(choice.startDate)}
                </option>
              ))}
            </select>
          </label>
        )}
        {(fieldError || error) && (
          <span role="alert" style={errorStyle}>
            {fieldError ?? error}
          </span>
        )}
        {redeemed && (
          <span role="status" style={{ fontSize: 12.5, color: 'var(--mtp-ok-text)' }}>
            {formatCurrency(redeemed.amount, redeemed.currency)} credit added
            {redeemed.expiresAt && ` · expires ${day(redeemed.expiresAt)}`}
          </span>
        )}
      </form>
    </Card>
  )
}
