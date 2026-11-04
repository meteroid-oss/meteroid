import { askMockChallenge } from '@/features/checkout/components/MockChallengeDialog'
import { env } from '@/lib/env'

/** Backend `MOCK_PUBLISHABLE_KEY`: marks a next-action as the test gateway's, never an SDK's. */
export const MOCK_PUBLISHABLE_KEY_PREFIX = 'mock_pk'

/** 3DS stand-in: the `client_secret` slot carries the backend's completion path; the customer's
 *  answer is posted there and settles through the production dispatcher. Throws on failure. */
export const completeMockChallenge = async (challengePath: string): Promise<void> => {
  if (!challengePath) {
    throw new Error('The test gateway returned no challenge to complete')
  }

  const approve = await askMockChallenge()

  const res = await fetch(new URL(challengePath, env.meteroidRestApiUri).toString(), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ approve }),
  })

  if (!res.ok) {
    throw new Error('The mock challenge could not be completed')
  }

  const body = (await res.json()) as { status?: string }
  if (body.status === 'failed') {
    throw new Error('Authentication failed (test gateway 3-D Secure)')
  }
}
