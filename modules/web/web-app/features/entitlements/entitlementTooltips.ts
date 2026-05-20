export type EntitlementAction = 'override' | 'pin' | 'unpin' | 'pinAll' | 'enable' | 'disable'

export function entitlementTooltip(entity: string, action: EntitlementAction): string {
  switch (action) {
    case 'override':
      return `Override inherited value`
    case 'pin':
      return `Lock inherited value`
    case 'unpin':
      return 'Remove local override'
    case 'pinAll':
      return `Lock all entitlements on this ${entity} at their current values.`
    case 'enable':
      return `Enable on this ${entity}`
    case 'disable':
      return `Disable on this ${entity}`
  }
}
