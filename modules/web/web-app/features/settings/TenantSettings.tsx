import { Dialog, DialogContent, DialogTitle, cn } from '@md/ui'
import {
  Blocks,
  Boxes,
  Briefcase,
  Building2,
  CalendarClock,
  Coins,
  CreditCard,
  FileText,
  FlaskConical,
  KeyRound,
  Landmark,
  Layers,
  Mail,
  Menu,
  Palette,
  ReceiptText,
  Settings2,
  ShieldCheck,
  Users,
  Webhook,
  type LucideIcon,
} from 'lucide-react'
import { FunctionComponent, ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { Navigate, Outlet, useLocation, useNavigate, useParams } from 'react-router'

import { useCloseOverlay } from '@/components/layouts/TenantLayout/components/OverlayOutlet'
import { editionId, placeAfter, slotItems, useVisibleItems } from '@/edition'
import { BatchJobsTab } from '@/features/batch-jobs/BatchJobsTab'
import { CustomerEmailsTab } from '@/features/customer-emails/CustomerEmailsTab'
import { DeliveryModeBadge } from '@/features/customer-emails/DeliveryView'
import { CheckPlayground } from '@/features/entitlements/explain/CheckPlayground'
import { SecuritySection } from '@/features/settings/SecuritySection'
import { SettingsSearch } from '@/features/settings/SettingsSearch'
import { findSettingsText, type SettingsSearchEntry } from '@/features/settings/settingsSearchIndex'
import { ApiDocsTab } from '@/features/settings/tabs/ApiDocsTab'
import { ApiKeysTab } from '@/features/settings/tabs/ApiKeysTab'
import { BrandingTab } from '@/features/settings/tabs/BrandingTab'
import { CompanyTab } from '@/features/settings/tabs/CompanyTab'
import { CreditTab } from '@/features/settings/tabs/CreditTab'
import { EnterpriseTab } from '@/features/settings/tabs/EnterpriseTab'
import { GeneralTab } from '@/features/settings/tabs/GeneralTab'
import { IntegrationsTab } from '@/features/settings/tabs/IntegrationsTab'
import { InvoiceTab } from '@/features/settings/tabs/InvoiceTab'
import { PaymentMethodsTab } from '@/features/settings/tabs/PaymentsTab'
import { RenewalsTab } from '@/features/settings/tabs/RenewalsTab'
import { ResourceTypesTab } from '@/features/settings/tabs/ResourceTypesTab'
import { TaxesTab } from '@/features/settings/tabs/TaxesTab'
import { UsersTab } from '@/features/settings/tabs/UsersTab'
import { WebhooksTab } from '@/features/settings/tabs/WebhooksTab'
import { useBasePath } from '@/hooks/useBasePath'
import {
  useExpressOrganizationState,
  useIsExpressOrganization,
} from '@/hooks/useIsExpressOrganization'
import { useTenant } from '@/hooks/useTenant'
import { env } from '@/lib/env'
import { EmailDeliveryMode, TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

import type { SettingsSectionTitle } from '@/edition/types'

interface SettingsItem {
  value: string
  label: string
  /** The page header, when it differs from the nav label. */
  title?: string
  icon: LucideIcon
  show: boolean
  content: ReactNode
  badge?: ReactNode
  after?: string
}

interface SettingsSection {
  title: SettingsSectionTitle
  items: SettingsItem[]
}

/** `settings` alone, or a legacy `settings?tab=x` link: redirect to the tab's path. */
export const SettingsIndexRedirect: FunctionComponent = () => {
  const { search, hash } = useLocation()
  const { isExpress, isLoading } = useExpressOrganizationState()
  if (isLoading) return null

  const params = new URLSearchParams(search)
  const tab = params.get('tab') ?? (isExpress ? 'merchant' : 'general')
  params.delete('tab')
  const query = params.toString()
  return <Navigate to={{ pathname: tab, search: query && `?${query}`, hash }} replace />
}

// Renamed tabs: bookmarks and older links land on the new path.
const RENAMED_TABS = new Map([['upgrade', 'enterprise']])

export const TenantSettings: FunctionComponent = () => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  const isExpress = useIsExpressOrganization()
  const { tab } = useParams()
  // replace: tab switches shouldn't pile up history entries the close handler has to step back through
  const setTab = (value: string) => navigate(`../${value}`, { replace: true })
  const close = useCloseOverlay(basePath)
  const [mobileNavOpen, setMobileNavOpen] = useState(false)
  // A search result inside a tab: once the tab shows it, scroll to it.
  const [target, setTarget] = useState<{ tab: string; entry: SettingsSearchEntry } | null>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  // The nav's list fades out at the bottom while there is more of it below.
  const [navHasMore, setNavHasMore] = useState(false)
  const updateNavHasMore = (list: HTMLElement) =>
    setNavHasMore(list.scrollTop + list.clientHeight < list.scrollHeight - 1)
  // The list and its content: either resizing can change whether there is more below.
  const observeNavList = useCallback((list: HTMLDivElement | null) => {
    if (!list) return
    const observer = new ResizeObserver(() => updateNavHasMore(list))
    observer.observe(list)
    if (list.firstElementChild) observer.observe(list.firstElementChild)
    return () => observer.disconnect()
  }, [])
  const { tenant } = useTenant()
  // A production workspace not delivering to customers is easy to forget: flag it in the nav.
  const emailsBadge =
    tenant?.environment === TenantEnvironmentEnum.PRODUCTION &&
    tenant.emailDeliveryMode !== EmailDeliveryMode.LIVE ? (
      <DeliveryModeBadge mode={tenant.emailDeliveryMode} />
    ) : undefined

  const moduleTabs = slotItems('settings.tabs')
  const visibleModuleTabs = useVisibleItems(moduleTabs, undefined)

  const sections: SettingsSection[] = [
    {
      title: 'Workspace',
      items: [
        { value: 'general', label: 'General', icon: Settings2, show: !isExpress, content: <GeneralTab /> },
        { value: 'enterprise', label: 'Enterprise', title: 'Meteroid Enterprise', icon: Briefcase, show: editionId === 'oss', content: <EnterpriseTab /> },
      ],
    },
    {
      title: 'Invoicing',
      items: [
        { value: 'merchant', label: 'Merchant', icon: Building2, show: true, content: <CompanyTab /> },
        { value: 'branding', label: 'Branding', icon: Palette, show: true, content: <BrandingTab /> },
        { value: 'invoices', label: 'Invoices', icon: ReceiptText, show: true, content: <InvoiceTab /> },
        { value: 'taxes', label: 'Taxes', icon: Landmark, show: true, content: <TaxesTab /> },
        { value: 'payments', label: 'Payment methods', icon: CreditCard, show: true, content: <PaymentMethodsTab /> },
        { value: 'emails', label: 'Customer emails', icon: Mail, show: true, content: <CustomerEmailsTab />, badge: emailsBadge },
        { value: 'renewals', label: 'Renewals', icon: CalendarClock, show: editionId !== 'oss', content: <RenewalsTab /> },
      ],
    },
    {
      title: 'Finance',
      items: [
        { value: 'credit-units', label: 'Credit', icon: Coins, show: !isExpress, content: <CreditTab /> },
      ],
    },
    {
      title: 'Platform',
      items: [
        { value: 'integrations', label: 'Integrations', icon: Blocks, show: true, content: <IntegrationsTab /> },
        { value: 'batch-jobs', label: 'Batch jobs', icon: Layers, show: true, content: <BatchJobsTab /> },
        { value: 'resource-types', label: 'Resource types', icon: Boxes, show: !isExpress, content: <ResourceTypesTab /> },
      ],
    },
    {
      title: 'Organization',
      items: [
        { value: 'users', label: 'Members', icon: Users, show: !isExpress, content: <UsersTab /> },
        { value: 'security', label: 'Security', icon: ShieldCheck, show: !isExpress, content: <SecuritySection /> },
      ],
    },
    {
      title: 'Developers',
      items: [
        { value: 'api-keys', label: 'API keys', icon: KeyRound, show: true, content: <ApiKeysTab /> },
        { value: 'api-docs', label: 'API docs', icon: FileText, show: true, content: <ApiDocsTab /> },
        { value: 'webhooks', label: 'Webhooks', icon: Webhook, show: true, content: <WebhooksTab /> },
        { value: 'test-check', label: 'Test a check', icon: FlaskConical, show: !isExpress && env.entitlementsEnabled, content: <CheckPlayground /> },
      ],
    },
  ]
  const moduleItems = moduleTabs.map(({ section, value, label, icon, after, Content }) => ({
    section,
    value,
    label,
    icon,
    after,
    show: visibleModuleTabs.some(tab => tab.value === value),
    content: <Content />,
  }))

  const visibleSections = sections
    .map(section => ({
      ...section,
      items: placeAfter<SettingsItem>(
        section.items,
        moduleItems.filter(item => item.section === section.title),
        item => item.value
      ).filter(item => item.show),
    }))
    .filter(section => section.items.length > 0)
  const visibleItems = visibleSections.flatMap(section => section.items)
  const current = visibleItems.find(item => item.value === tab) ?? visibleItems[0]

  // A tab may still be loading, or keep the entry under one of its own sub-tabs: look for the
  // entry's text each frame for a few seconds, switching to its sub-tab first. Display only.
  useEffect(() => {
    if (!target || current?.value !== target.tab) return
    const startedAt = performance.now()
    let subTabShown = !target.entry.subTab
    let frame = 0
    const look = () => {
      const root = contentRef.current
      if (root && !subTabShown) {
        const trigger = Array.from(root.querySelectorAll<HTMLElement>('[role="tab"]')).find(
          element => element.textContent?.trim() === target.entry.subTab
        )
        if (trigger) {
          // Radix tabs switch on mousedown.
          trigger.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 }))
          subTabShown = true
        }
      }
      const found = root && subTabShown ? findSettingsText(root, target.entry.text) : null
      if (found) {
        found.scrollIntoView({ block: 'center', behavior: 'smooth' })
        const brand = getComputedStyle(found).getPropertyValue('--brand').trim()
        found.animate(
          [
            { backgroundColor: `hsl(${brand} / 0.22)`, boxShadow: `0 0 0 4px hsl(${brand} / 0.22)` },
            { backgroundColor: 'transparent', boxShadow: '0 0 0 4px transparent' },
          ],
          { duration: 1800, easing: 'ease-out' }
        )
        setTarget(null)
      } else if (performance.now() - startedAt < 4000) {
        frame = requestAnimationFrame(look)
      } else {
        setTarget(null)
      }
    }
    frame = requestAnimationFrame(look)
    return () => cancelAnimationFrame(frame)
  }, [target, current?.value])

  const openTab = (value: string) => {
    setTab(value)
    setMobileNavOpen(false)
  }

  const renamedTab = tab && RENAMED_TABS.get(tab)
  if (renamedTab) return <Navigate to={`../${renamedTab}`} replace />

  return (
    <Dialog open onOpenChange={open => !open && close()}>
      <DialogContent
        aria-describedby={undefined}
        // The dialog's close button sits centred in the 56px header.
        className="mt-modal-workspace flex flex-col gap-0 overflow-hidden p-0 w-[calc(100vw-2rem)] max-w-[1240px] h-[calc(100dvh-3rem)] max-h-[900px] sm:rounded-xl [&>button.absolute]:top-[15px] [&>button.absolute]:right-4"
      >
        {/* The header's and the nav's edges: white at 9.2% in dark, a step under the dividers. */}
        <header className="flex h-14 shrink-0 items-center gap-2 border-b border-border pl-4 pr-14 dark:border-white/[0.092] sm:pl-5">
          <button
            type="button"
            onClick={() => setMobileNavOpen(true)}
            className="-ml-1.5 rounded-md p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground md:hidden"
            aria-label="Open settings menu"
          >
            <Menu size={18} />
          </button>
          <DialogTitle className="font-sans text-base font-semibold tracking-tight [font-variation-settings:normal]">
            Settings
          </DialogTitle>
        </header>

        <div className="relative flex min-h-0 flex-1">
          {mobileNavOpen && (
            <div
              className="mt-backdrop absolute inset-0 z-20 md:hidden"
              onClick={() => setMobileNavOpen(false)}
              aria-hidden
            />
          )}
          <nav
            className={cn(
              // In dark, black at 3% over the settings surface (which the themes tint), laid as an
              // image over the surface's colour so the sliding mobile menu stays opaque; in light,
              // the app sidebar's colour.
              'absolute inset-y-0 left-0 z-30 flex w-64 max-w-[85%] flex-col border-r border-border bg-sidebar shadow-xl transition-transform duration-200 ease-in-out dark:border-white/[0.092] dark:bg-[hsl(var(--workspace-surface))] dark:[background-image:linear-gradient(rgb(0_0_0/0.03),rgb(0_0_0/0.03))]',
              'md:static md:z-auto md:w-60 md:max-w-none md:shrink-0 md:translate-x-0 md:shadow-none md:transition-none',
              mobileNavOpen ? 'translate-x-0' : '-translate-x-full'
            )}
          >
            {/* Level with the tab's title across the divider. */}
            <div className="flex h-14 shrink-0 items-center px-3">
              <SettingsSearch
                sections={visibleSections}
                onSelect={(value, entry) => {
                  openTab(value)
                  setTarget(entry ? { tab: value, entry } : null)
                }}
              />
            </div>
            <div
              ref={observeNavList}
              onScroll={event => updateNavHasMore(event.currentTarget)}
              className={cn(
                'min-h-0 flex-1 overflow-y-auto px-3',
                navHasMore &&
                  '[mask-image:linear-gradient(to_bottom,black_calc(100%-40px),transparent)]'
              )}
            >
              <div className="flex flex-col gap-5 pb-5 pt-1">
                {visibleSections.map(section => (
                  <div key={section.title} className="flex flex-col gap-0.5">
                    <div className="px-2 pb-1 text-xs font-medium text-muted-foreground">
                      {section.title}
                    </div>
                    {section.items.map(item => (
                      <button
                        key={item.value}
                        onClick={() => openTab(item.value)}
                        className={cn(
                          'group/item flex items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm transition-colors',
                          current?.value === item.value
                            ? 'bg-accent font-medium text-accent-foreground'
                            : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'
                        )}
                      >
                        {/* At rest a step darker than the label, as in the main nav. */}
                        <item.icon
                          size={16}
                          className={cn(
                            'shrink-0 transition-colors',
                            current?.value !== item.value &&
                              'text-sidebar-icon group-hover/item:text-foreground'
                          )}
                        />
                        <span className="truncate">{item.label}</span>
                        {item.badge && <span className="ml-auto shrink-0">{item.badge}</span>}
                      </button>
                    ))}
                  </div>
                ))}
              </div>
            </div>
          </nav>

          <div className="flex min-w-0 flex-1 flex-col">
            <div
              ref={contentRef}
              className="flex-1 overflow-y-auto px-4 pb-5 pt-10 sm:px-6"
            >
              {/* The tab's column, centred in the panel; its title scrolls with it. */}
              <div className="mx-auto w-full max-w-3xl">
                <h2 className="mb-6 border-b border-border pb-4 text-xl font-semibold tracking-tight">
                  {current?.title ?? current?.label}
                </h2>
                {current?.content}
              </div>
            </div>
          </div>
        </div>
      </DialogContent>

      <Outlet />
    </Dialog>
  )
}
