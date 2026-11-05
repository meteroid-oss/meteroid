import { Dialog, DialogContent, DialogTitle, cn } from '@md/ui'
import {
  Blocks,
  Building2,
  Coins,
  CreditCard,
  FileText,
  KeyRound,
  Landmark,
  Layers,
  Mail,
  Menu,
  Palette,
  ReceiptText,
  Settings2,
  ShieldCheck,
  Sparkles,
  Users,
  Webhook,
  type LucideIcon,
} from 'lucide-react'
import { FunctionComponent, ReactNode, useState } from 'react'
import { Navigate, Outlet, useLocation, useNavigate, useParams } from 'react-router-dom'

import { useCloseOverlay } from '@/components/layouts/TenantLayout/components/OverlayOutlet'
import { editionId, placeAfter, slotItems, useVisibleItems } from '@/edition'
import { BatchJobsTab } from '@/features/batch-jobs/BatchJobsTab'
import { CustomerEmailsTab } from '@/features/customer-emails/CustomerEmailsTab'
import { DeliveryModeBadge } from '@/features/customer-emails/DeliveryView'
import { SecuritySection } from '@/features/settings/SecuritySection'
import { ApiDocsTab } from '@/features/settings/tabs/ApiDocsTab'
import { ApiKeysTab } from '@/features/settings/tabs/ApiKeysTab'
import { BrandingTab } from '@/features/settings/tabs/BrandingTab'
import { CompanyTab } from '@/features/settings/tabs/CompanyTab'
import { CreditTab } from '@/features/settings/tabs/CreditTab'
import { GeneralTab } from '@/features/settings/tabs/GeneralTab'
import { IntegrationsTab } from '@/features/settings/tabs/IntegrationsTab'
import { InvoiceTab } from '@/features/settings/tabs/InvoiceTab'
import { PaymentMethodsTab } from '@/features/settings/tabs/PaymentsTab'
import { TaxesTab } from '@/features/settings/tabs/TaxesTab'
import { UpgradeTab } from '@/features/settings/tabs/UpgradeTab'
import { UsersTab } from '@/features/settings/tabs/UsersTab'
import { WebhooksTab } from '@/features/settings/tabs/WebhooksTab'
import { useBasePath } from '@/hooks/useBasePath'
import {
  useExpressOrganizationState,
  useIsExpressOrganization,
} from '@/hooks/useIsExpressOrganization'
import { useTenant } from '@/hooks/useTenant'
import { EmailDeliveryMode, TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

import type { SettingsSectionTitle } from '@/edition/types'

interface SettingsItem {
  value: string
  label: string
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

export const TenantSettings: FunctionComponent = () => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  const isExpress = useIsExpressOrganization()
  const { tab } = useParams()
  // replace: tab switches shouldn't pile up history entries the close handler has to step back through
  const setTab = (value: string) => navigate(`../${value}`, { replace: true })
  const close = useCloseOverlay(basePath)
  const [mobileNavOpen, setMobileNavOpen] = useState(false)
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
        { value: 'upgrade', label: 'Upgrade', icon: Sparkles, show: editionId === 'oss', content: <UpgradeTab /> },
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

  return (
    <Dialog open onOpenChange={open => !open && close()}>
      <DialogContent
        aria-describedby={undefined}
        className="flex gap-0 overflow-hidden p-0 w-[calc(100vw-2rem)] max-w-[1240px] h-[calc(100dvh-3rem)] max-h-[900px] sm:rounded-xl"
      >
        {mobileNavOpen && (
          <div
            className="absolute inset-0 z-20 bg-black/40 md:hidden"
            onClick={() => setMobileNavOpen(false)}
            aria-hidden
          />
        )}
        <nav
          className={cn(
            'absolute inset-y-0 left-0 z-30 flex w-64 max-w-[85%] flex-col gap-5 overflow-y-auto border-r border-border bg-background dark:bg-[#171717] px-3 py-5 shadow-xl transition-transform duration-200 ease-in-out',
            'md:static md:z-auto md:w-60 md:max-w-none md:shrink-0 md:translate-x-0 md:bg-muted/30 md:shadow-none md:transition-none',
            mobileNavOpen ? 'translate-x-0' : '-translate-x-full'
          )}
        >
          <DialogTitle className="px-2 text-base font-semibold">Settings</DialogTitle>
          {visibleSections.map(section => (
            <div key={section.title} className="flex flex-col gap-0.5">
              <div className="px-2 pb-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                {section.title}
              </div>
              {section.items.map(item => (
                <button
                  key={item.value}
                  onClick={() => {
                    setTab(item.value)
                    setMobileNavOpen(false)
                  }}
                  className={cn(
                    'flex items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm transition-colors',
                    current?.value === item.value
                      ? 'bg-accent font-medium text-accent-foreground'
                      : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'
                  )}
                >
                  <item.icon size={16} className="shrink-0" />
                  <span className="truncate">{item.label}</span>
                  {item.badge && <span className="ml-auto shrink-0">{item.badge}</span>}
                </button>
              ))}
            </div>
          ))}
        </nav>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-14 shrink-0 items-center gap-2 border-b border-border px-4 sm:px-6">
            <button
              type="button"
              onClick={() => setMobileNavOpen(true)}
              className="-ml-1.5 rounded-md p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground md:hidden"
              aria-label="Open settings menu"
            >
              <Menu size={18} />
            </button>
            <h2 className="truncate pr-12 text-base font-semibold">{current?.label}</h2>
          </header>
          <div className="flex-1 overflow-y-auto px-4 py-5 sm:px-6">{current?.content}</div>
        </div>
      </DialogContent>

      <Outlet />
    </Dialog>
  )
}
