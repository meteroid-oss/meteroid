import { Button, Tabs, TabsContent, TabsList, TabsTrigger } from '@md/ui'
import { Maximize2Icon, XIcon } from 'lucide-react'
import { useState } from 'react'

import { placeAfter, slotItems } from '@/edition'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useQueryState } from '@/hooks/useQueryState'

import { DeliveryView } from './DeliveryView'
import { EmailKindsView } from './EmailKindsView'
import { PaymentRemindersSwitch } from './PaymentRemindersSwitch'
import { SenderView } from './SenderView'
import { SentEmailsView } from './SentEmailsView'

const tabTriggerCls =
  'rounded-none border-b-2 border-transparent bg-transparent px-0 pb-2.5 text-muted-foreground shadow-none data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none'

const CORE_VIEWS = [
  { value: 'emails', label: 'Emails' },
  { value: 'general', label: 'General' },
  { value: 'activity', label: 'Activity' },
]

export const CustomerEmailsTab = () => {
  const [requestedView, setView] = useQueryState('view', 'emails', { replace: true })
  const [fullscreen, setFullscreen] = useState(false)

  const moduleViews = slotItems('customerEmails.views')
  const tabList = placeAfter<{ value: string; label: string; after?: string }>(
    CORE_VIEWS,
    moduleViews,
    v => v.value
  )
  const view = tabList.some(v => v.value === requestedView) ? requestedView : 'emails'
  const customization = useEntitlement(entitlementCodes.EMAIL_CUSTOMIZATION)

  const views = (
    <Tabs value={view} onValueChange={setView} className="w-full">
      <TabsList className="h-auto w-full justify-start gap-6 rounded-none border-b border-border bg-transparent p-0">
        {tabList.map(v => (
          <TabsTrigger key={v.value} value={v.value} className={tabTriggerCls}>
            {v.label}
          </TabsTrigger>
        ))}
      </TabsList>
      <TabsContent value="emails" className="mt-5">
        <EmailKindsView onOpenView={setView} />
      </TabsContent>
      <TabsContent value="general" className="mt-5">
        <SenderView />
      </TabsContent>
      {moduleViews.map(({ value, Content }) => (
        <TabsContent key={value} value={value} className="mt-5">
          <Content />
        </TabsContent>
      ))}
      <TabsContent value="activity" className="mt-5">
        <SentEmailsView />
      </TabsContent>
    </Tabs>
  )

  if (!customization) {
    return (
      <div className="flex flex-col gap-6">
        <DeliveryView />
        <SettingsSection
          title="Customer emails"
          description="Your customers receive every default email, sent in your entity's legal name."
          action={<InvoicingEntitySelect />}
        >
          <PaymentRemindersSwitch />
        </SettingsSection>
      </div>
    )
  }

  if (fullscreen) {
    return (
      <div className="fixed inset-0 z-50 flex flex-col bg-background">
        <div className="flex h-12 shrink-0 items-center justify-between gap-4 border-b border-border px-4">
          <h1 className="text-sm font-semibold">Customer emails</h1>
          <div className="flex items-center gap-2">
            <InvoicingEntitySelect />
            <Button variant="ghost" size="sm" hasIcon onClick={() => setFullscreen(false)}>
              <XIcon size={14} /> Exit
            </Button>
          </div>
        </div>
        <div className="flex-1 overflow-y-auto">
          <div className="mx-auto flex w-full max-w-screen-xl flex-col gap-4 p-6">
            <DeliveryView />
            {views}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6">
      <DeliveryView />
      <SettingsSection
        title="Customer emails"
        description="What your customers receive, who it comes from, and when reminders go out."
        action={
          <div className="flex items-center gap-2">
            <InvoicingEntitySelect />
            <Button variant="ghost" size="sm" hasIcon onClick={() => setFullscreen(true)}>
              <Maximize2Icon size={14} /> Expand
            </Button>
          </div>
        }
      >
        {views}
      </SettingsSection>
    </div>
  )
}
