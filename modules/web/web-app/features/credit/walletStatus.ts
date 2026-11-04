// Wording shared by the dashboard and the customer portal: both proto WalletStatus enums map here.
export type WalletHealthKey = 'healthy' | 'low' | 'depleted' | 'blocked'

// A wallet in good standing is not labelled: only the states that need attention are named.
export const walletHealthLabel: Record<WalletHealthKey, string> = {
  healthy: '',
  low: 'Low',
  depleted: 'Depleted',
  blocked: 'Access denied: no credit',
}

export const walletHealthDescription: Record<WalletHealthKey, string> = {
  healthy: 'Enough credit left after unbilled usage.',
  low: 'Below the low-balance alert or top-up threshold.',
  depleted: 'Credit is used up. Further usage is billed on the next invoice.',
  blocked:
    'Credit is used up. Access checks return denied until credit is added; usage that still arrives is recorded and billed.',
}

/** Shown instead of the after-usage figure and status while a ledger change outpaces the live
 * estimate, so a stale number is never shown next to the balance it contradicts. */
export const liveUpdatingLabel = 'Updating…'
export const liveUpdatingDescription = 'Recalculating after a recent change.'
