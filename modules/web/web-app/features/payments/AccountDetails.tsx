import { Connector } from '@/rpc/api/connectors/v1/models_pb'

import { modeLabel } from './connectors'
import { PaymentProviderDefinition } from './providers'

/** Label/value grid of what the provider account reported. */
export const AccountDetails = ({
  connector,
  definition,
  extra,
}: {
  connector: Connector
  definition?: PaymentProviderDefinition
  extra?: [string, string][]
}) => {
  const account = connector.account
  const rows: [string, string | undefined][] = [
    ['Account', account?.displayName],
    ['ID', account?.accountId || undefined],
    ['Mode', account ? modeLabel(account.livemode, definition) : undefined],
    ['Key', account?.keyHint],
    ...(extra ?? []),
  ]
  const shown = rows.filter((row): row is [string, string] => !!row[1])
  if (shown.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        Account details haven&apos;t been detected yet.
      </p>
    )
  }
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
      {shown.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-muted-foreground">{label}</dt>
          <dd className="truncate font-mono">{value}</dd>
        </div>
      ))}
    </dl>
  )
}
