import { Code, ConnectError } from '@connectrpc/connect'
import { Badge, cn } from '@md/ui'

import {
  Connector,
  ConnectorStatusEnum,
  PaymentMethodAvailability,
  PaymentMethodStatus,
  PaymentRailEnum,
} from '@/rpc/api/connectors/v1/models_pb'

import { PaymentProviderDefinition, PaymentRail, paymentProvider, railsOf } from './providers'

/** Connector lifecycle helpers shared by the integrations list, wizard, manage modal and routing. */

export const METHOD_LABELS: Record<PaymentRailEnum, string> = {
  [PaymentRailEnum.CARD]: 'Card',
  [PaymentRailEnum.DIRECT_DEBIT_SEPA]: 'SEPA',
  [PaymentRailEnum.DIRECT_DEBIT_ACH]: 'ACH',
  [PaymentRailEnum.DIRECT_DEBIT_BACS]: 'Bacs',
}

export const RAIL_LABELS: Record<PaymentRail, string> = {
  card: 'Card payments',
  directDebit: 'Direct debit',
}

const railOfMethod = (rail: PaymentRailEnum): PaymentRail =>
  rail === PaymentRailEnum.CARD ? 'card' : 'directDebit'

export interface RailStatus {
  /** The connection can serve this rail at all. */
  supported: boolean
  /** Enabled on the provider account, so it can be routed. */
  enabled: boolean
  /** Why a supported rail isn't enabled. */
  reason?: string
}

/**
 * What the provider account allows for a rail. Without detected account details (legacy
 * connectors), falls back to the effective capabilities.
 */
export const railStatus = (connector: Connector, rail: PaymentRail): RailStatus => {
  const methods = connector.account?.methods.filter(m => railOfMethod(m.rail) === rail)
  if (!methods) {
    const supported = railsOf(connector.paymentCapabilities)[rail]
    return { supported, enabled: supported }
  }
  if (methods.length === 0) return { supported: false, enabled: false }
  if (methods.some(m => m.availability === PaymentMethodAvailability.ENABLED)) {
    return { supported: true, enabled: true }
  }
  return { supported: true, enabled: false, reason: methodsReason(connector, methods) }
}

const methodsReason = (connector: Connector, methods: PaymentMethodStatus[]) => {
  const provider = paymentProvider(connector.provider)?.name ?? 'the provider'
  const labels = methods.map(m => METHOD_LABELS[m.rail]).join(' / ')
  const pending = methods.find(m => m.availability === PaymentMethodAvailability.PENDING)
  if (pending) {
    return `${labels} is pending on ${provider}${pending.reason ? ` (${pending.reason})` : ''}.`
  }
  return `${labels} isn't enabled on this ${provider} account.`
}

export interface MethodChip {
  key: string
  label: string
  availability: PaymentMethodAvailability
  reason?: string
}

const AVAILABILITY_ORDER: Record<PaymentMethodAvailability, number> = {
  [PaymentMethodAvailability.ENABLED]: 0,
  [PaymentMethodAvailability.PENDING]: 1,
  [PaymentMethodAvailability.UNAVAILABLE]: 2,
}

/** Every method the integration supports, with its state on the account. */
export const methodChips = (connector: Connector): MethodChip[] => {
  if (connector.account) {
    return connector.account.methods
      .map(m => ({
        key: String(m.rail),
        label: METHOD_LABELS[m.rail],
        availability: m.availability,
        reason: m.reason,
      }))
      // Enabled first, then pending, then unavailable; provider order within each group.
      .sort((a, b) => AVAILABILITY_ORDER[a.availability] - AVAILABILITY_ORDER[b.availability])
  }
  const rails = railsOf(connector.paymentCapabilities)
  return (['card', 'directDebit'] as const)
    .filter(rail => rails[rail])
    .map(rail => ({
      key: rail,
      label: rail === 'card' ? 'Card' : 'Direct Debit',
      availability: PaymentMethodAvailability.ENABLED,
    }))
}

export const MethodChips = ({
  chips,
  className,
}: {
  chips: MethodChip[]
  className?: string
}) => (
  <span className={cn('flex flex-wrap items-center gap-1', className)}>
    {chips.map(chip => (
      <span
        key={chip.key}
        title={chip.reason}
        className={cn(
          'rounded px-1.5 py-0.5 text-[11px] font-medium leading-none',
          chip.availability === PaymentMethodAvailability.ENABLED &&
            'bg-success/15 text-success',
          chip.availability === PaymentMethodAvailability.PENDING && 'bg-warning/15 text-warning',
          chip.availability === PaymentMethodAvailability.UNAVAILABLE &&
            'bg-muted text-muted-foreground line-through'
        )}
      >
        {chip.availability === PaymentMethodAvailability.ENABLED && '✓ '}
        {chip.label}
        {chip.availability === PaymentMethodAvailability.PENDING && ' · pending'}
      </span>
    ))}
  </span>
)

/** Shown next to the status when the provider refused the stored credentials. */
export const CredentialsErrorBadge = ({ connector }: { connector: Connector }) =>
  connector.credentialsError ? (
    <Badge variant="destructive" size="sm" title={connector.credentialsError.message}>
      Invalid credentials
    </Badge>
  ) : null

export const ConnectorStatusBadge = ({ status }: { status: ConnectorStatusEnum }) => {
  switch (status) {
    case ConnectorStatusEnum.ACTIVE:
      return (
        <Badge variant="success" size="sm">
          Active
        </Badge>
      )
    case ConnectorStatusEnum.DISABLED:
      return (
        <Badge variant="secondary" size="sm">
          Disabled
        </Badge>
      )
    case ConnectorStatusEnum.DRAFT:
      return (
        <Badge variant="warning" size="sm">
          Setup incomplete
        </Badge>
      )
    default:
      return (
        <Badge variant="outline" size="sm">
          Deleted
        </Badge>
      )
  }
}

export type WizardStep = 'name' | 'credentials' | 'webhook' | 'review'

const WIZARD_STEPS: { id: WizardStep; title: string }[] = [
  { id: 'name', title: 'Name' },
  { id: 'credentials', title: 'Credentials' },
  { id: 'webhook', title: 'Webhook' },
  { id: 'review', title: 'Review' },
]

/** The steps this provider's wizard shows: no webhook step when there's nothing to configure. */
export const wizardSteps = (definition: PaymentProviderDefinition | undefined) =>
  WIZARD_STEPS.filter(s => s.id !== 'webhook' || !!definition?.connect.webhook)

/** First unfinished step of a draft, where "Resume setup" lands. */
export const resumeStep = (
  connector: Connector,
  definition: PaymentProviderDefinition | undefined
): WizardStep => {
  if (!connector.hasCredentials) return 'credentials'
  if (definition?.connect.webhook && connector.requiresWebhookSecret && !connector.hasWebhookSecret) {
    return 'webhook'
  }
  return 'review'
}

export const modeLabel = (livemode: boolean, definition?: PaymentProviderDefinition) =>
  livemode ? 'Live' : definition?.connect.showsEnvironment ? 'Sandbox' : 'Test'

/**
 * The backend's user-facing message for validation-type failures (a rejected key, a name already
 * in use); a generic fallback otherwise.
 */
export const userFacingError = (error: unknown, fallback: string, providerName?: string) => {
  const err = ConnectError.from(error)
  switch (err.code) {
    case Code.InvalidArgument:
    case Code.FailedPrecondition:
    case Code.AlreadyExists:
      return err.rawMessage || fallback
    case Code.Unavailable:
    case Code.DeadlineExceeded:
      return providerName
        ? `${providerName} couldn't be reached. Try again in a moment.`
        : 'The server could not be reached. Try again in a moment.'
    default:
      return fallback
  }
}

/** Invoicing entity names as prose: "A", "A and B", "A, B and C". */
export const joinNames = (names: string[]) =>
  names.length <= 1
    ? (names[0] ?? '')
    : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
