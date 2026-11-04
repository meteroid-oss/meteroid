interface AltchaChallenge {
  algorithm: string
  challenge: string
  maxnumber: number
  salt: string
  signature: string
}

const BATCH = 500
const encoder = new TextEncoder()

const toHex = (buf: ArrayBuffer) =>
  Array.from(new Uint8Array(buf), b => b.toString(16).padStart(2, '0')).join('')

const yieldToEventLoop = () => new Promise(resolve => setTimeout(resolve, 0))

/** Solves an ALTCHA SHA-256 challenge in batches, yielding so the page stays responsive. */
export const solveAltcha = async (raw: string, signal: AbortSignal): Promise<string> => {
  const { algorithm, challenge, maxnumber, salt, signature } = JSON.parse(raw) as AltchaChallenge
  if (algorithm !== 'SHA-256') throw new Error(`Unsupported algorithm ${algorithm}`)
  const target = challenge.toLowerCase()

  for (let start = 0; start <= maxnumber; start += BATCH) {
    if (signal.aborted) throw new DOMException('Aborted', 'AbortError')
    const end = Math.min(start + BATCH - 1, maxnumber)
    const numbers = Array.from({ length: end - start + 1 }, (_, i) => start + i)
    const hashes = await Promise.all(
      numbers.map(n => crypto.subtle.digest('SHA-256', encoder.encode(salt + n)))
    )
    const found = hashes.findIndex(h => toHex(h) === target)
    if (found >= 0) {
      return btoa(JSON.stringify({ algorithm, challenge, number: numbers[found], salt, signature }))
    }
    await yieldToEventLoop()
  }
  throw new Error('No solution found for the challenge')
}
