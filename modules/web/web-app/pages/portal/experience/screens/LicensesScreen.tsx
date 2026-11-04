import { Code, ConnectError } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import { Copy, KeyRound } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { getLicenseActivationCode } from '@/rpc/portal/customer/v1/customer-PortalCustomerService_connectquery'

import { date } from '../format'
import { useLicenses } from '../hooks'
import { CenterState, LinkButton, Mono, PanelCard, Pill, Spinner } from '../primitives'

import type { StatusBadge } from '../format'
import type { PortalLicense } from '@/rpc/portal/customer/v1/models_pb'

const licenseBadge = (status: string): StatusBadge => {
  switch (status.toUpperCase()) {
    case 'ACTIVE':
      return { label: 'Active', tone: 'ok' }
    case 'PENDING':
      return { label: 'Not activated', tone: 'warn' }
    case 'REVOKED':
      return { label: 'Revoked', tone: 'neutral' }
    default:
      return { label: status, tone: 'neutral' }
  }
}

const copy = (value: string, label: string) => {
  void navigator.clipboard?.writeText(value)
  toast.success(`${label} copied`)
}

const MUTED: React.CSSProperties = { fontSize: 12.5, color: 'var(--mtp-text-2)' }

const CopyableValue = ({ value, label }: { value: string; label: string }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      padding: '8px 12px',
      border: '1px solid var(--mtp-border)',
      borderRadius: 'var(--mtp-r-ctrl)',
      background: 'var(--mtp-surface-2)',
      minWidth: 0,
    }}
  >
    <Mono style={{ fontSize: 12.5, overflow: 'hidden', textOverflow: 'ellipsis', flex: 1 }}>
      {value}
    </Mono>
    <LinkButton onClick={() => copy(value, label)}>
      <Copy size={13} />
      Copy
    </LinkButton>
  </div>
)

const LicenseRow = ({
  license,
  activationUrl,
  first,
}: {
  license: PortalLicense
  activationUrl?: string
  first: boolean
}) => {
  const [code, setCode] = useState<string | null>(null)
  const [revealError, setRevealError] = useState<string | null>(null)
  const reveal = useMutation(getLicenseActivationCode)
  const pending = license.status.toUpperCase() === 'PENDING'

  const onReveal = async () => {
    setRevealError(null)
    try {
      const res = await reveal.mutateAsync({ licenseId: license.id })
      setCode(res.activationCode)
    } catch (error) {
      setRevealError(
        error instanceof ConnectError && error.code === Code.PermissionDenied
          ? 'Open the billing portal from your account to see the activation code.'
          : 'The activation code could not be retrieved.'
      )
    }
  }

  return (
    <div
      style={{
        padding: '16px 20px',
        borderTop: first ? 'none' : '1px solid var(--mtp-border)',
        display: 'flex',
        flexDirection: 'column',
        gap: 12,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <span style={{ fontSize: 13.5, fontWeight: 600 }}>
              {license.planName ?? 'License'}
            </span>
            <Pill badge={licenseBadge(license.status)} />
          </div>
          <span style={MUTED}>
            <Mono>{license.id}</Mono> · issued {date(license.createdAt)}
            {license.lastRefreshedAt && ` · last seen ${date(license.lastRefreshedAt)}`}
          </span>
          {license.instanceId && (
            <span style={MUTED}>
              Installation <Mono>{license.instanceId}</Mono>
            </span>
          )}
        </div>
        {pending && !code && (
          <LinkButton onClick={() => void onReveal()}>
            {reveal.isPending ? <Spinner size={13} /> : <KeyRound size={13} />}
            Show activation code
          </LinkButton>
        )}
      </div>

      {revealError && (
        <span style={{ fontSize: 12.5, color: 'var(--mtp-danger)' }}>{revealError}</span>
      )}

      {code && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <span style={MUTED}>Enter this code in your installation to activate the license.</span>
          <CopyableValue value={code} label="Activation code" />
          {activationUrl && (
            <>
              <span style={MUTED}>Activation server, if your installation asks for one:</span>
              <CopyableValue value={activationUrl} label="Activation URL" />
            </>
          )}
        </div>
      )}
    </div>
  )
}

export const LicensesScreen = () => {
  const query = useLicenses()
  const licenses = query.data?.licenses ?? []

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <h1 style={{ fontSize: 22, fontWeight: 600, letterSpacing: '-0.02em', margin: 0 }}>
          Licenses
        </h1>
        <p style={{ fontSize: 13, color: 'var(--mtp-text-2)', margin: 0 }}>
          Software licenses included with your subscriptions.
        </p>
      </div>

      <PanelCard title="Licenses">
        {query.isLoading ? (
          <div
            style={{
              display: 'flex',
              justifyContent: 'center',
              padding: '48px 0',
              color: 'var(--mtp-text-3)',
            }}
          >
            <Spinner size={20} />
          </div>
        ) : licenses.length === 0 ? (
          <CenterState
            title="No licenses yet"
            hint="Licenses appear here once a subscription that includes one is active."
          />
        ) : (
          licenses.map((license, i) => (
            <LicenseRow
              key={license.id}
              license={license}
              activationUrl={query.data?.activationUrl}
              first={i === 0}
            />
          ))
        )}
      </PanelCard>
    </div>
  )
}
