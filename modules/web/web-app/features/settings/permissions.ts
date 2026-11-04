// Display metadata for the RBAC permission vocabulary. The codes and grouping
// are owned by the backend (`common_domain::rbac`); this only makes them
// human-readable. Unknown codes fall back to a title-cased label so a new
// backend permission still renders before this map is updated.

export const PERMISSION_GROUP_LABELS: Record<string, string> = {
  customers: 'Customers',
  billing: 'Billing',
  catalog: 'Product catalog',
  analytics: 'Analytics',
  events: 'Usage events',
  alerts: 'Alerts',
  targets: 'Targets',
  settings: 'Settings & integrations',
  members: 'Members',
  roles: 'Roles',
  organization: 'Organization',
}

// Order groups from day-to-day operations down to governance.
export const PERMISSION_GROUP_ORDER = [
  'customers',
  'billing',
  'catalog',
  'analytics',
  'events',
  'alerts',
  'targets',
  'settings',
  'members',
  'roles',
  'organization',
]

export const PERMISSION_LABELS: Record<string, { label: string; description?: string }> = {
  'customers:read': { label: 'View customers' },
  'customers:manage': { label: 'Manage customers', description: 'Create, edit and delete customers' },
  'billing:subscriptions:read': { label: 'View subscriptions' },
  'billing:subscriptions:manage': {
    label: 'Manage subscriptions',
    description: 'Create, edit and cancel subscriptions',
  },
  'billing:invoices:read': { label: 'View invoices', description: 'Invoices and credit notes' },
  'billing:invoices:manage': {
    label: 'Manage invoices',
    description: 'Issue, finalize and void invoices and credit notes',
  },
  'billing:quotes:read': { label: 'View quotes' },
  'billing:quotes:manage': { label: 'Manage quotes', description: 'Create, send and expire quotes' },
  'catalog:read': { label: 'View catalog', description: 'Plans, prices, products, add-ons, coupons, metrics' },
  'catalog:manage': { label: 'Manage catalog' },
  'analytics:read': { label: 'View analytics', description: 'Stats, segments, activity feed, saved views' },
  'events:read': { label: 'View usage events', description: 'Explore and inspect raw metering events' },
  'alerts:read': { label: 'View alerts' },
  'alerts:manage': { label: 'Manage alerts' },
  'targets:read': { label: 'View targets', description: 'Targets, the plan and its history' },
  'targets:manage': { label: 'Manage targets', description: 'Set and change targets and the plan' },
  'settings:read': { label: 'View settings', description: 'Invoicing entities, taxes, connectors, webhooks, tokens' },
  'settings:manage': { label: 'Manage settings' },
  'members:manage': { label: 'Manage members', description: 'Invite, remove and assign roles to members' },
  'roles:manage': { label: 'Manage roles', description: 'Create, edit and delete custom roles' },
  'organization:manage': {
    label: 'Manage organization',
    description: 'Organization settings, MFA policy, security & audit',
  },
}

const titleCase = (value: string) =>
  value.charAt(0).toUpperCase() + value.slice(1).replace(/[:_]/g, ' ')

// `x:manage` grants `x:read` (mirrors the backend `RolePermissions::contains`
// implication). Returns the manage code that implies this read, or null.
export const impliedByManage = (code: string): string | null =>
  code.endsWith(':read') ? `${code.slice(0, -':read'.length)}:manage` : null

export const permissionLabel = (code: string): string => PERMISSION_LABELS[code]?.label ?? titleCase(code)

export const permissionDescription = (code: string): string | undefined =>
  PERMISSION_LABELS[code]?.description

export const groupLabel = (group: string): string => PERMISSION_GROUP_LABELS[group] ?? titleCase(group)

export interface PermissionGroup {
  group: string
  label: string
  codes: string[]
}

/**
 * Bucket permission codes by their `group`, ordered by {@link PERMISSION_GROUP_ORDER}
 * with any unknown group appended alphabetically.
 */
export const groupPermissions = (permissions: { code: string; group: string }[]): PermissionGroup[] => {
  const byGroup = new Map<string, string[]>()
  for (const p of permissions) {
    const codes = byGroup.get(p.group) ?? []
    codes.push(p.code)
    byGroup.set(p.group, codes)
  }
  return [...byGroup.keys()]
    .sort((a, b) => {
      const ia = PERMISSION_GROUP_ORDER.indexOf(a)
      const ib = PERMISSION_GROUP_ORDER.indexOf(b)
      return (ia === -1 ? Infinity : ia) - (ib === -1 ? Infinity : ib) || a.localeCompare(b)
    })
    .map(group => ({ group, label: groupLabel(group), codes: byGroup.get(group) ?? [] }))
}
