/**
 * What the settings search finds inside each tab, beyond the tab's own name: its section titles,
 * rows and main fields. Each entry is written exactly as the page shows it, because choosing a
 * result opens the tab and scrolls to that text. Keep it in step with the tabs; an entry the
 * page no longer shows still opens the right tab, it just does not scroll.
 */
export interface SettingsSearchEntry {
  text: string
  /** The tab's own sub-tab (its TabsTrigger label) the entry lives in, when it is not the first. */
  subTab?: string
}

const entries = (...texts: string[]): SettingsSearchEntry[] => texts.map(text => ({ text }))
const inSubTab = (subTab: string, ...texts: string[]): SettingsSearchEntry[] =>
  texts.map(text => ({ text, subTab }))

export const SETTINGS_SEARCH_ENTRIES: Record<string, SettingsSearchEntry[]> = {
  general: entries(
    'Organization',
    'Company trade name',
    'Tenant name',
    'Tenant slug',
    'Reporting currency',
    'Customer emails',
    'Keep the last copy of re-sent events',
    'Currencies',
    'Danger zone',
    'Delete tenant',
    'Delete organization'
  ),
  merchant: entries(
    'Billing details',
    'Legal name',
    'Address line 1',
    'Address line 2',
    'Zip code',
    'State',
    'City',
    'VAT number / Tax ID',
    'National company registration number',
    'Invoice contact',
    'Contact name',
    'Phone',
    'Email',
    'Accounting',
    'Accounting currency'
  ),
  branding: entries(
    'Portal & invoice branding',
    'Logo',
    'Theme',
    'Roundness',
    'Email button color',
    'Customer-facing language',
    'Default language',
    'Additional languages'
  ),
  invoices: entries(
    'Invoice settings',
    'Invoice number pattern',
    'Credit note number pattern',
    'Grace period (hours)',
    'Net terms (days)',
    'Minimum charge',
    'Default invoice language',
    'Require billing information at checkout',
    'Structured e-invoicing',
    'Invoice footer'
  ),
  taxes: entries(
    'Tax calculation',
    'Method',
    'Require a valid tax ID for reverse charge',
    'Custom rates',
    'Overrides',
    'Product tax categories',
    'Default product tax category'
  ),
  payments: entries('Payment method routing'),
  emails: [
    ...entries('Payment reminders'),
    ...inSubTab(
      'General',
      'Sender',
      'Sender address',
      'Sender name',
      'Reply-to',
      'Bcc',
      'Languages',
      'Default language',
      'Additional languages',
      'Branding in emails',
      'Brand colour',
      'Footer'
    ),
    ...inSubTab('Activity', 'Activity'),
  ],
  'credit-units': entries('Credit units', 'Customer portal', 'Top-ups'),
  integrations: entries(
    'Payment Providers',
    'Stripe',
    'Adyen',
    'CRM',
    'HubSpot',
    'Accounting',
    'Pennylane',
    'Quickbooks'
  ),
  users: entries('Members', 'Pending invites', 'Danger zone', 'Leave organization'),
  security: entries('Two-factor authentication'),
}

/**
 * Finds the element showing `text` in the tab's content: the innermost one whose own text is
 * exactly it (a section title, a row title, a field label), or failing that one that starts
 * with it (a label with a dynamic end, such as "Minimum charge (EUR)").
 */
export const findSettingsText = (root: HTMLElement, text: string): HTMLElement | null => {
  const wanted = text.trim().toLowerCase()
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  let prefixMatch: HTMLElement | null = null
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const own = node.textContent?.trim().toLowerCase()
    const element = node.parentElement
    if (!own || !element || element.offsetParent === null) continue
    if (own === wanted) return element
    if (!prefixMatch && own.startsWith(wanted)) prefixMatch = element
  }
  return prefixMatch
}
