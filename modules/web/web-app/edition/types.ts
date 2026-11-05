import type { MinimumComponentOption } from '@/features/billing/minimumCommitment'
import type { Denomination } from '@/features/plans/creditRule'
import type { PortalCreditValues } from '@/features/settings/components/portalCreditForm'
import type { UsageProjection } from '@/features/subscriptions/PeriodUsageBar'
import type { ExpiryContext } from '@/features/subscriptions/subscriptionOverview'
import type { CreateCustomerSchema } from '@/lib/schemas/customers'
import type { EditPlanSchema } from '@/lib/schemas/plans'
import type { GeneralSettingsSchema } from '@/lib/schemas/settings'
import type { EmailKind } from '@/rpc/api/customeremails/v1/models_pb'
import type { CustomerBrief, CustomerPaymentMethod } from '@/rpc/api/customers/v1/models_pb'
import type { InvoiceAnnex } from '@/rpc/api/invoices/v1/models_pb'
import type { MinimumCommitment, PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import type { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'
import type { PendingInvoiceCredit } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import type { Wallet, WalletStatus, WalletSummary } from '@/rpc/api/wallets/v1/models_pb'
import type { Wallet as PortalWallet } from '@/rpc/portal/wallets/v1/models_pb'
import type { GetCreditOverviewResponse } from '@/rpc/portal/wallets/v1/wallets_pb'
import type { DescMethodUnary } from '@bufbuild/protobuf'
import type { LucideIcon } from 'lucide-react'
import type { ComponentType } from 'react'
import type { Control } from 'react-hook-form'
import type { RouteObject } from 'react-router-dom'

export interface CustomerDetailContext {
  customerId: string
}

export interface PlanVersionContext {
  planVersionId: string
}

export interface InvoicingEntityCreateContext {
  open: boolean
  setOpen: (open: boolean) => void
  setInvoicingEntity: (id: string) => void
}

export interface CustomerCreateFieldsContext {
  control: Control<CreateCustomerSchema>
}

export interface SubscriptionDetailContext {
  customerId: string
  subscriptionId: string
}

export interface DetailTab<C> {
  /** Former ids of the tab: links naming one open it. */
  aliases?: string[]
  id: string
  label: string
  /** The core tab this one follows; module tabs without one follow the core content tabs. */
  after?: string
  useVisible: (ctx: C) => boolean
  Content: ComponentType<C>
}

/** An entry of an entity's action menu, opening a dialog rendered by the page. */
export interface DetailAction<C> {
  id: string
  label: string
  icon: LucideIcon
  useVisible: (ctx: C) => boolean
  Dialog: ComponentType<C & { onClose: () => void }>
}

export type SettingsSectionTitle =
  'Workspace' | 'Invoicing' | 'Finance' | 'Platform' | 'Organization' | 'Developers'

export interface SettingsTab {
  value: string
  label: string
  icon: LucideIcon
  section: SettingsSectionTitle
  /** The core tab of the section this one follows; by default it ends the section. */
  after?: string
  useVisible: () => boolean
  Content: ComponentType
}

/** A tab of the Security settings page; tabs are sorted by `order`. */
export interface SecuritySettingsTab {
  value: string
  label: string
  order: number
  useVisible: () => boolean
  /** Shows the lock icon: the plan does not include the feature. */
  useLocked: () => boolean
  Content: ComponentType
}

/** An entry of the sidebar's secondary navigation (footer), after the core entries. */
export interface NavSecondaryItem {
  title: string
  url: string
  icon: LucideIcon
  useVisible: () => boolean
}

/** An entry of a module's main navigation section. */
export interface NavMainItem {
  title: string
  url: string
  icon?: LucideIcon
  beta?: boolean
  new?: boolean
  /** Read permission the entry requires. */
  permission?: string
}

/** A section of the sidebar's main navigation, outside Express organizations; hidden when no entry is left. */
export interface NavMainSection {
  title: string
  icon: LucideIcon
  new?: boolean
  /** Title of the core entry the section follows. */
  after: string
  items: NavMainItem[]
  /** What waits on the caller, by entry title. */
  useCounts?: () => Record<string, number>
}

/** A column of the customer list, after the name. */
export interface CustomerListColumn {
  id: string
  header: string
  /** Whether the rows of the page carry what the column shows. */
  isVisible: (rows: CustomerBrief[]) => boolean
  Cell: ComponentType<{ customer: CustomerBrief }>
}

export interface HelpTopicContext {
  /** Fills the feedback field, unless the user already typed in it. */
  prefill: (feedback: string) => void
}

/** A notice the help page shows above its content when opened with `?topic=<id>`. */
export interface HelpTopic {
  id: string
  Notice: ComponentType<HelpTopicContext>
}

/** The metrics core list pages ask a metric strip for. */
export type ListMetricKey = 'outstanding_ar' | 'active_subscriptions'

export interface ListMetricStripContext {
  metricKey: ListMetricKey
}

/** The annexes of a draft being edited; the form keeps them and sends the edited ones. */
export interface InvoiceEditAnnexesContext {
  invoiceId: string
  annexes: InvoiceAnnex[]
  /** A correction owns a frozen snapshot of its annexes; other drafts re-derive theirs. */
  isCorrection: boolean
  onChange: (annexes: InvoiceAnnex[]) => void
  /** Annexes rebuilt from the event stream, replacing the edited ones. */
  onRegenerated: (annexes: InvoiceAnnex[]) => void
}

export interface GeneralSettingsContext {
  control: Control<GeneralSettingsSchema>
}

export interface InvoiceSettingsContext {
  invoicingEntityId: string
}

export interface WalletContext {
  wallet: Wallet
}

/** A wallet's card on the customer's Credit tab. */
export interface CustomerWalletCardContext {
  /** Amounts print bare: the card's title and balance name the credit unit. */
  wallet: Wallet & { bareUnit?: boolean }
  paymentMethods: CustomerPaymentMethod[]
  canConfigure: boolean
}

/** What one subscription can spend of a wallet once every subscription's unbilled usage is paid. */
export interface ScopedWalletBalance {
  balance: bigint
  /** Its usage not invoiced yet that asks for credit, tax included. */
  unbilled: bigint
  /** Of `unbilled`, what no credit it can use pays. */
  uncovered: bigint
}

export interface WalletProjectionContext {
  wallet: Wallet
  /** The subscriptions drawing on the wallet whose usage is projected. */
  subscriptionIds: string[]
  /** Names each subscription's outcome when several are projected. */
  subscriptionLabel: (subscriptionId: string) => string
}

/** A module's live view of each wallet on the customer's Credit tab. */
export interface CustomerWalletLive {
  /** What the subscription can spend; unset until estimated, and without a subscription. */
  useScopedBalance: (
    wallet: Wallet,
    subscriptionId: string | undefined
  ) => ScopedWalletBalance | undefined
  /** The period's usage projected to its end; unset when it cannot be projected. */
  useProjection: (ctx: WalletProjectionContext) => UsageProjection | undefined
  /** Said after the balance, e.g. what access checks return at zero. */
  notes: (wallet: Wallet) => string[]
}

export interface WalletSettingsDialogContext {
  wallet: Wallet
  customerCurrency: string
  paymentMethods: CustomerPaymentMethod[]
  defaultPaymentMethodId?: string
  onClose: () => void
}

export interface SubscriptionNoticesContext {
  subscription: Subscription
  /** The wallets the subscription draws on. */
  wallets: Wallet[]
  canManageWallets: boolean
  onAddCredit: (wallet: Wallet) => void
  onConfigureWallet: (wallet: Wallet) => void
}

/** A wallet's row in the subscription's Credit section. */
export interface SubscriptionCreditRowContext {
  wallet: Wallet
  subscriptionId: string
  /** What the next invoice bills of the subscription's usage this wallet pays. */
  periodUsage?: bigint
  periodEnd?: string
  asOf: Date
  /** Credit bought on unpaid invoices, funded once they are paid. */
  pending: PendingInvoiceCredit[]
  expiry: ExpiryContext
  /** Set when the row offers its own Add credit button. */
  addCredit?: () => void
}

/** A module's live view of the credit a subscription draws on. */
export interface SubscriptionCreditLive {
  /** Replaces the core row of each wallet. */
  Row: ComponentType<SubscriptionCreditRowContext>
  /** The customer's other subscriptions sharing this one's credit. */
  useSharedWith: (subscriptionId: string) => string[]
  /** The subscription's usage stops once the credit of `wallets` (its currency's) runs out: its price reads as prepaid. */
  prepaid: (subscriptionId: string, wallets: Wallet[]) => boolean
}

export interface PlanCreditRuleContext {
  control: Control<EditPlanSchema>
  /** The currencies and credit units the version charges usage in. */
  denominations: Denomination[]
  /** The rule shown is the version's default, not chosen yet. */
  defaulted: boolean
}

export interface PlanMinimumCommitmentContext {
  planVersion: PlanVersion
}

/** The minimum of a subscription or quote being created: the form keeps it and sends it. */
export interface MinimumCommitmentFieldContext {
  currency: string
  components: MinimumComponentOption[]
  planDefault?: MinimumCommitment
  override?: MinimumCommitment
  /** No minimum although the plan has one. */
  dropped: boolean
  onChange: (next: { override?: MinimumCommitment; dropped: boolean }) => void
  /** Quotes only override the plan's minimum, never drop it. */
  allowDrop: boolean
  /** Inline new-product extras can be scoped: the server resolves them at creation. */
  allowPending: boolean
}

export interface PortalCreditSettingsContext {
  control: Control<PortalCreditValues>
  /** No invoicing entity is loaded yet. */
  disabled: boolean
}

/** A column of the Wallets list, after Status. */
export interface WalletListColumn {
  id: string
  header: string
  Cell: ComponentType<{ wallet: WalletSummary }>
}

/** An entry of the Wallets list's status filter, after the core ones. */
export interface WalletListFilter {
  /** Its `?status=` value. */
  value: string
  label: string
  request: { status?: WalletStatus; autoTopUpFailing?: boolean }
}

export interface PortalAlertsContext {
  credit?: GetCreditOverviewResponse
  /** Takes the customer to add credit to the wallet. */
  onAddCredit: (walletId: string) => void
}

export interface PortalBalanceContext {
  credit: GetCreditOverviewResponse
}

export interface PortalBalanceWalletContext extends PortalBalanceContext {
  wallet: PortalWallet
  /** A card of its own below the add-credit card shown while usage is paused; else inside the
   * wallet's balance card, after its Add credit button. */
  standalone: boolean
}

/**
 * Slots filled with components, keyed to the props each component receives. Named
 * `<screen>.<area>`; contributions render in module order at the place documented below.
 */
export interface ComponentSlots {
  /** Over the top of the tenant pages, above the tenant status banner. */
  'layout.banners': Record<never, never>
  /** Below the sidebar's tenant picker, above the main navigation; not in Express organizations. */
  'layout.sidebarActions': Record<never, never>
  /** Above the rows of the invoice and subscription lists; without one, no chart toggle. */
  'listPage.metricStrip': ListMetricStripContext
  /** Account settings fields of the customer creation form, after the currency. */
  'customerCreate.fields': CustomerCreateFieldsContext
  /** At the top of the customer's overview tab. */
  'customerDetail.attention': CustomerDetailContext
  /** Above the subscription, after the core notices. */
  'subscriptionDetail.attention': SubscriptionDetailContext
  /** At the end of the subscription's main column, after the core sections. */
  'subscriptionDetail.sections': SubscriptionDetailContext
  /** At the end of the plan page, after the core sections and above the version footer. */
  'planDetail.sections': PlanVersionContext
  /** The annexes section of the draft invoice form, after the discount. */
  'invoiceEdit.annexes': InvoiceEditAnnexesContext
  /** The dialog creating another invoicing entity; without one, the entity picker offers no creation. */
  'invoicingEntitySelect.createDialog': InvoicingEntityCreateContext
  /** Rows of the General settings form, after the customer emails row. */
  'generalSettings.rows': GeneralSettingsContext
  /** Rows of the Invoices settings form, before the billing-information row; each saves on its own. */
  'invoiceSettings.rows': InvoiceSettingsContext
  /** Rows of the customer email Sender settings' "Branding in emails" group, after the footer. */
  'emailSenderSettings.brandingRows': Record<never, never>
  /** Sections of the Branding settings form, after the core sections. */
  'brandingSettings.sections': Record<never, never>
  /** Sections of the Security page's two-factor tab, after the member's own settings. */
  'securitySettings.policies': Record<never, never>
  /** Next to the status badge of each wallet's card on the customer's Credit tab. */
  'customerWallet.badges': WalletContext
  /** Beside the period's usage on each wallet's card on the customer's Credit tab. */
  'customerWallet.sections': CustomerWalletCardContext
  /** The wallet settings dialog; the first one replaces the core one, which edits a credit unit wallet's agreed price. */
  'wallet.settingsDialog': WalletSettingsDialogContext
  /** Among the subscription's notices, after the errored one; each renders `Band`s. */
  'subscriptionDetail.notices': SubscriptionNoticesContext
  /** In the header of the subscription's Usage limits section. */
  'subscriptionDetail.usageLimitsAside': SubscriptionDetailContext
  /** In the Status column of the Wallets list, after the wallet's status. */
  'walletList.statusBadges': { wallet: WalletSummary }
  /** The "When credit runs out" field of a draft plan version; without one, the stored rule is shown read-only. */
  'planEdit.creditRule': PlanCreditRuleContext
  /** The plan version's minimum, in its Contract terms; without one, a stored minimum shows read-only. */
  'planDetail.minimumCommitment': PlanMinimumCommitmentContext
  /** The Contract terms of the subscription creation; without one, the subscription takes the plan's minimum. */
  'subscriptionCreate.minimumCommitment': MinimumCommitmentFieldContext
  /** The Contract terms of the quote form; without one, an edited quote keeps its minimum and a new one takes the plan's. */
  'quoteCreate.minimumCommitment': MinimumCommitmentFieldContext
  /** Rows of the portal credit settings, after Top-ups; bound to the core form and saved with it. */
  'portalCreditSettings.rows': PortalCreditSettingsContext
  /** Portal banners, before the hard-limit one. */
  'portal.alerts': PortalAlertsContext
  /** At the end of the portal overview, after the recent invoices. */
  'portal.overview.sections': Record<never, never>
  /** On each wallet of the portal Credit page. */
  'portal.balance.wallet': PortalBalanceWalletContext
  /** Once on the portal Credit page, after the wallets: the dialogs a module opens. */
  'portal.balance.dialogs': PortalBalanceContext
}

/** Every slot, keyed to the item a module contributes; component slots take the component. */
export type Slots = {
  /** Sections of the sidebar's main navigation, each after the core entry it names. */
  'layout.navMain': NavMainSection
  /** Entries of the sidebar footer, after the core entries. */
  'layout.navSecondary': NavSecondaryItem
  /** Tabs of the settings page, placed in their section. */
  'settings.tabs': SettingsTab
  /** Tabs of the Security settings page, sorted with the core tabs by `order`. */
  'securitySettings.tabs': SecuritySettingsTab
  /** Columns of the customer list, after the name. */
  'customerList.columns': CustomerListColumn
  /** Tabs of the customer page, placed among the core tabs. */
  'customerDetail.tabs': DetailTab<CustomerDetailContext>
  /** Entries of the subscription's action menu, after Edit Billing Settings. */
  'subscriptionDetail.actions': DetailAction<SubscriptionDetailContext>
  /** Notices of the help page, picked by `?topic=<id>`. */
  'help.topics': HelpTopic
  /** Email kinds a module sends: the Emails view lists them only then. */
  'customerEmails.kinds': EmailKind
  /** Queries refreshed with the wallet queries whenever credit changes. */
  'wallets.queries': DescMethodUnary
  /** Queries refreshed with the portal's after any portal action. */
  'portal.queries': DescMethodUnary
  /** The live view of each wallet on the customer's Credit tab; the first one is used, without one the ledger figures show. */
  'customerWallet.live': CustomerWalletLive
  /** The live credit of a subscription; the first one replaces the core rows. */
  'subscriptionCredit.live': SubscriptionCreditLive
  /** Columns of the Wallets list, after Status. */
  'walletList.columns': WalletListColumn
  /** Entries of the Wallets list's status filter, after the core ones. */
  'walletList.filters': WalletListFilter
} & { [K in keyof ComponentSlots]: ComponentType<ComponentSlots[K]> }

export interface WebModule {
  id: string
  /** Top-level routes, outside the session guard. */
  rootRoutes?: RouteObject[]
  /** Children of the protected `/` route (signed-in user, no organization in the path). */
  protectedRoutes?: RouteObject[]
  /** Children of the `/:organizationSlug` route. */
  orgRoutes?: RouteObject[]
  /** Children of the `:tenantSlug` route. */
  tenantRoutes?: RouteObject[]
  /** Public customer-portal routes, outside the session guard. */
  portalRoutes?: RouteObject[]
  slots?: { [K in keyof Slots]?: Slots[K][] }
}

export interface Edition {
  id: string
  modules: WebModule[]
}

export const defineModule = (module: WebModule): WebModule => module
export const defineEdition = (edition: Edition): Edition => edition
