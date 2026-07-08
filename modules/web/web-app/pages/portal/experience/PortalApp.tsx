import { AlertCircle } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { usePortalConfig } from './PortalThemeProvider'
import { PortalNav, PortalNavContext, PORTAL_TABS, PortalTab } from './context'
import { useInvalidatePortal, usePortalOverview, usePortalToken } from './hooks'
import { ChangePlanModal } from './modals/ChangePlanModal'
import { CenterState, Spinner } from './primitives'
import { InvoicesScreen } from './screens/InvoicesScreen'
import { OverviewScreen } from './screens/OverviewScreen'
import { SettingsScreen } from './screens/SettingsScreen'
import { SubscriptionDetailScreen } from './screens/SubscriptionDetailScreen'
import { SubscriptionsScreen } from './screens/SubscriptionsScreen'
import { UsageScreen } from './screens/UsageScreen'

import type { CustomerPortalOverview } from '@/rpc/portal/customer/v1/models_pb'

const BrandMark = ({ logoUrl, name }: { logoUrl?: string; name: string }) =>
  logoUrl ? (
    <img
      src={logoUrl}
      alt={name}
      style={{ height: 24, width: 'auto', maxWidth: 120, objectFit: 'contain' }}
    />
  ) : (
    <div
      style={{
        width: 26,
        height: 26,
        borderRadius: 'var(--mtp-r-sm)',
        background: 'var(--mtp-accent)',
        color: 'var(--mtp-on-accent)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontWeight: 700,
        fontSize: 14,
      }}
    >
      {(name || 'B').charAt(0).toUpperCase()}
    </div>
  )

const Header = ({
  overview,
  brandName,
  logoUrl,
}: {
  overview: CustomerPortalOverview
  brandName: string
  logoUrl?: string
}) => (
  <header
    style={{
      position: 'sticky',
      top: 0,
      zIndex: 10,
      borderBottom: '1px solid var(--mtp-border)',
      background: 'var(--mtp-header-bg)',
      backdropFilter: 'saturate(1.4) blur(10px)',
    }}
  >
    <div
      style={{
        maxWidth: 920,
        margin: '0 auto',
        padding: '0 20px',
        height: 56,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 16,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
        <BrandMark logoUrl={logoUrl} name={brandName} />
        {!logoUrl && (
          <span style={{ fontWeight: 600, fontSize: 15, letterSpacing: '-0.02em' }}>
            {brandName}
          </span>
        )}
        <span
          style={{
            fontSize: 12,
            color: 'var(--mtp-text-2)',
            borderLeft: '1px solid var(--mtp-border-2)',
            paddingLeft: 12,
            marginLeft: 2,
          }}
        >
          Billing
        </span>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>

        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '5px 12px',
            borderRadius: 20,
            border: '1px solid var(--mtp-border)',
            background: 'var(--mtp-surface)',
          }}
        >
          <div
            style={{
              width: 20,
              height: 20,
              borderRadius: '50%',
              background: 'var(--mtp-accent-weak)',
              color: 'var(--mtp-accent-ink)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              fontSize: 10,
              fontWeight: 700,
            }}
          >
            {(overview.customer?.name ?? '?').charAt(0).toUpperCase()}
          </div>
          <span style={{ fontSize: 12.5, fontWeight: 500 }}>{overview.customer?.name}</span>
        </div>
      </div>
    </div>
  </header>
)

const Tabs = ({ active, onChange }: { active: PortalTab; onChange: (t: PortalTab) => void }) => (
  <div
    style={{
      maxWidth: 920,
      margin: '0 auto',
      padding: '0 20px',
      display: 'flex',
      gap: 24,
      overflowX: 'auto',
      marginBottom: -1,
    }}
    className="mtp-scroll"
  >
    {PORTAL_TABS.map(t => {
      const isActive = active === t.key
      return (
        <button
          key={t.key}
          onClick={() => onChange(t.key)}
          className="mtp-tab"
          style={{
            padding: '15px 0',
            fontSize: 13.5,
            fontWeight: 500,
            fontFamily: 'inherit',
            cursor: 'pointer',
            background: 'none',
            border: 'none',
            borderBottom: isActive ? '2px solid var(--mtp-accent-ink)' : '2px solid transparent',
            color: isActive ? 'var(--mtp-text)' : 'var(--mtp-text-2)',
            whiteSpace: 'nowrap',
            transition: 'color .12s',
          }}
        >
          {t.label}
        </button>
      )
    })}
  </div>
)

export const PortalApp = () => {
  const { data, isLoading, error, refetch, overview } = usePortalOverview()
  const token = usePortalToken()
  const config = usePortalConfig()
  const invalidatePortal = useInvalidatePortal()

  // Screen state lives in the URL (?page=, ?sub=) so it survives reloads and is
  // deep-linkable from embedded widgets. The change-plan modal stays ephemeral.
  const [searchParams, setSearchParams] = useSearchParams()
  const [changePlanSubId, setChangePlanSubId] = useState<string | null>(null)

  const pageParam = searchParams.get('page')
  const tab: PortalTab = PORTAL_TABS.some(t => t.key === pageParam)
    ? (pageParam as PortalTab)
    : 'overview'
  const detailSubId = searchParams.get('sub')

  // Mutate the query string while preserving token / theme / embed params.
  const setParams = useCallback(
    (mutate: (params: URLSearchParams) => void) => {
      const next = new URLSearchParams(searchParams)
      mutate(next)
      setSearchParams(next, { replace: true })
    },
    [searchParams, setSearchParams]
  )

  const nav = useMemo<PortalNav | null>(() => {
    if (!overview?.customer) return null
    return {
      overview,
      token,
      currency: overview.customer.currency || overview.activeSubscriptions[0]?.currency || 'USD',
      goTo: (t: PortalTab) =>
        setParams(p => {
          p.delete('sub')
          if (t === 'overview') p.delete('page')
          else p.set('page', t)
        }),
      openSubscription: (id: string) =>
        setParams(p => {
          p.set('page', 'subscriptions')
          p.set('sub', id)
        }),
      openChangePlan: (id: string) => setChangePlanSubId(id),
      refetchOverview: () => invalidatePortal(),
    }
  }, [overview, token, invalidatePortal, setParams])

  if (error) {
    return (
      <CenterState
        icon={<AlertCircle size={26} style={{ color: 'var(--mtp-text-3)' }} />}
        title="Something went wrong"
        hint="There may be a connection issue or your session may have expired."
      />
    )
  }

  if (isLoading || !nav || !data?.overview) {
    return (
      <div
        style={{
          minHeight: '60vh',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--mtp-text-3)',
        }}
      >
        <Spinner size={22} />
      </div>
    )
  }

  const brandName = config.branding.companyName || overview!.customer!.name || 'Billing'

  const renderScreen = () => {
    if (tab === 'subscriptions' && detailSubId) {
      return (
        <SubscriptionDetailScreen
          subscriptionId={detailSubId}
          onBack={() => setParams(p => p.delete('sub'))}
        />
      )
    }
    switch (tab) {
      case 'overview':
        return <OverviewScreen />
      case 'subscriptions':
        return <SubscriptionsScreen />
      case 'usage':
        return <UsageScreen />
      case 'invoices':
        return <InvoicesScreen />
      case 'settings':
        return <SettingsScreen />
    }
  }

  return (
    <PortalNavContext.Provider value={nav}>
      <Header overview={overview!} brandName={brandName} logoUrl={config.branding.logoUrl} />
      <div style={{ borderBottom: '1px solid var(--mtp-border)' }}>
        <Tabs
          active={tab}
          onChange={t => nav.goTo(t)}
        />
      </div>

      <main style={{ maxWidth: 920, margin: '0 auto', padding: '28px 20px 48px' }}>
        {renderScreen()}

        {config.branding.showPoweredBy !== false && (
          <div
            style={{
              marginTop: 36,
              paddingTop: 20,
              borderTop: '1px solid var(--mtp-border)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              gap: 7,
            }}
          >
            <span style={{ fontSize: 11.5, color: 'var(--mtp-text-3)' }}>Powered by</span>
            <a
              href="https://meteroid.com/?utm_source=portal"
              target="_blank"
              rel="noopener noreferrer"
              style={{ fontSize: 12.5, fontWeight: 600, color: 'var(--mtp-text-2)', textDecoration: 'none' }}
            >
              Meteroid
            </a>
          </div>
        )}
      </main>

      {changePlanSubId && (
        <ChangePlanModal
          subscriptionId={changePlanSubId}
          onClose={() => setChangePlanSubId(null)}
          onApplied={() => {
            setChangePlanSubId(null)
            refetch()
          }}
        />
      )}
    </PortalNavContext.Provider>
  )
}
