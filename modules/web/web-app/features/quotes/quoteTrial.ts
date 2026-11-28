import { QuoteTrialEndAction } from '@/rpc/api/quotes/v1/models_pb'

export type QuoteTrialEnd = 'convert' | 'manual'

export const DECIDE_AT_THE_END = 'Decide at the end'

export const trialEndToForm = (action: QuoteTrialEndAction | undefined): QuoteTrialEnd =>
  action === QuoteTrialEndAction.MANUAL ? 'manual' : 'convert'

/** Without a trial there is nothing to decide: always CONVERT. */
export const trialEndToProto = (days: number | undefined, end: QuoteTrialEnd | undefined) =>
  days && end === 'manual' ? QuoteTrialEndAction.MANUAL : QuoteTrialEndAction.CONVERT

/** "14 days, free · Decide at the end"; undefined without a trial. */
export const quoteTrialLabel = ({
  days,
  free,
  manual,
}: {
  days?: number
  free?: boolean
  manual?: boolean
}) => {
  if (!days) return undefined
  const freeness = free === false ? ', billed' : free ? ', free' : ''
  return `${days} days${freeness}${manual ? ` · ${DECIDE_AT_THE_END}` : ''}`
}
