import { Tabs, TabsContent, TabsList, TabsTrigger } from '@md/ui'
import { LockIcon } from 'lucide-react'

import { slotItems } from '@/edition'
import { PoliciesTab } from '@/features/settings/tabs/PoliciesTab'
import { useQueryState } from '@/hooks/useQueryState'

import type { SecuritySettingsTab } from '@/edition/types'

const notLocked = () => false

const coreTabs: SecuritySettingsTab[] = [
  {
    value: 'policies',
    label: 'Two-factor authentication',
    order: 10,
    useVisible: () => true,
    useLocked: notLocked,
    Content: PoliciesTab,
  },
]

let allTabs: SecuritySettingsTab[] | undefined

// Fixed once built, so each render calls the same hooks in the same order.
const securityTabs = () =>
  (allTabs ??= [...coreTabs, ...slotItems('securitySettings.tabs')].sort(
    (a, b) => a.order - b.order
  ))

// Bundles the account/organization security surfaces under one nav entry:
// personal & org MFA (Policies), SSO, custom roles, and the two audit trails.
export const SecuritySection = () => {
  const [sub, setSub] = useQueryState('sec', 'policies')

  const tabs = securityTabs()
    .map(tab => ({ tab, show: tab.useVisible(), locked: tab.useLocked() }))
    .filter(t => t.show)

  const active = tabs.find(t => t.tab.value === sub) ?? tabs[0]

  return (
    <Tabs value={active?.tab.value} onValueChange={setSub} className="flex flex-col gap-5">
      <TabsList className="self-start">
        {tabs.map(({ tab, locked }) => (
          <TabsTrigger key={tab.value} value={tab.value} className="gap-1.5">
            {tab.label}
            {locked && <LockIcon size={12} className="text-warning" />}
          </TabsTrigger>
        ))}
      </TabsList>
      {tabs.map(({ tab }) => (
        <TabsContent key={tab.value} value={tab.value} className="mt-0">
          <tab.Content />
        </TabsContent>
      ))}
    </Tabs>
  )
}
