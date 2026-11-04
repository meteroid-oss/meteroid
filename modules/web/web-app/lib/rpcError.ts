import { ConnectError } from '@connectrpc/connect'

/** The server's own reason for a failed call, without the "[code]" prefix, else `fallback`. */
export const rpcErrorMessage = (error: unknown, fallback: string): string => {
  if (error instanceof ConnectError) return error.rawMessage || fallback
  if (error instanceof Error) return error.message.replace(/^\[[a-z_]+\]\s*/i, '') || fallback
  return fallback
}
