import { type WebhookAttempt } from '@/rpc/api/webhooksout/v1/models_pb'

/** JSON bodies are indented; anything else, including JSON cut at the 1 KB cap, stays as is. */
const formatBody = (body: string): string => {
  try {
    return JSON.stringify(JSON.parse(body), null, 2)
  } catch {
    return body
  }
}

/**
 * What an attempt got back. Without a response, the error says why (refused, TLS, timeout).
 * With one, the status is shown next to it already, so the body is what explains a failure.
 */
export const AttemptOutput = ({ attempt }: { attempt: WebhookAttempt }) => {
  const error = attempt.responseStatus == null ? attempt.error : undefined
  const body = attempt.responseBody?.trim()

  if (!error && !body) return null

  return (
    <div className="mt-1 space-y-1">
      {error && <p className="break-words font-mono text-xs text-muted-foreground">{error}</p>}
      {body && (
        <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-md bg-muted px-2 py-1 font-mono text-xs">
          {formatBody(body)}
        </pre>
      )}
    </div>
  )
}
