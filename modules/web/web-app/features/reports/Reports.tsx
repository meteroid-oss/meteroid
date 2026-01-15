import { ScrollArea, Tabs, TabsContent, TabsList, TabsTrigger } from '@ui/components'
import { useState } from 'react'
import { Outlet } from 'react-router-dom'

import SidebarMenu from '@/components/SidebarMenu'
import { TenantPageLayout } from '@/components/layouts'

export const Reports = () => {
  return (
    <TenantPageLayout>
      <div className="flex">
        <ReportsNavigation />

        <main className="flex  flex-col flex-1 w-full max-w-screen-2xl pl-8 pr-2 mx-auto h-full overflow-x-hidden ">
          <Outlet />
        </main>
      </div>
    </TenantPageLayout>
  )
}

const enabledItems = [
  {
    label: 'Recurring Revenue',
    items: [
      { label: 'MRR', to: '.', end: true },
      { label: 'Revenue', to: 'revenue' },
    ],
  },
]

const comingSoonItems = [
  {
    label: 'Recurring Revenue',
    items: [
      { label: 'MRR Movements', to: '#', disabled: true },
      { label: 'Net MRR Movements', to: '#', disabled: true },
      { label: 'Annual Run Rate', to: '#', disabled: true },
      { label: 'Committed MRR Forecast', to: '#', disabled: true },
    ],
  },
  {
    label: 'Leads and Conversion',
    items: [
      { label: 'Leads', to: '#', disabled: true },
      { label: 'Free Trials', to: '#', disabled: true },
      { label: 'Trial-to-Paid Conversion Rate', to: '#', disabled: true },
      { label: 'Pipeline Funnel Analysis', to: '#', disabled: true },
      { label: 'Average Sales Cycle Length', to: '#', disabled: true },
    ],
  },
  {
    label: 'Subscribers',
    items: [
      { label: 'Subscribers', to: '#', disabled: true },
      { label: 'Average Revenue Per Account', to: '#', disabled: true },
      { label: 'Average Sale Price', to: '#', disabled: true },
      { label: 'Customer Lifetime Value', to: '#', disabled: true },
      { label: 'Subscriptions', to: '#', disabled: true },
      { label: 'Subscription Quantity', to: '#', disabled: true },
    ],
  },
  {
    label: 'Churn',
    items: [
      { label: 'Customer Churn Rate', to: '#', disabled: true },
      { label: 'Net MRR Churn Rate', to: '#', disabled: true },
      { label: 'Gross MRR Churn Rate', to: '#', disabled: true },
      { label: 'Quantity Churn Rate', to: '#', disabled: true },
    ],
  },
  {
    label: 'Retention',
    items: [
      { label: 'Net MRR Retention', to: '#', disabled: true },
      { label: 'Gross MRR Retention', to: '#', disabled: true },
    ],
  },
  {
    label: 'Transactions',
    items: [
      { label: 'Net Cash Flow', to: '#', disabled: true },
      { label: 'Gross Cash Flow', to: '#', disabled: true },
      { label: 'Non-Recurring Cash Flow', to: '#', disabled: true },
      { label: 'Successful Payments', to: '#', disabled: true },
      { label: 'Refunds', to: '#', disabled: true },
      { label: 'Failed Transactions', to: '#', disabled: true },
    ],
  },
]

export const ReportsNavigation = () => {
  const [value, setValue] = useState('general')

  return (
    <aside className="flex flex-col w-[250px] h-full relative text-sm ">
      <Tabs defaultValue="general" className="w-full h-full" value={value} onValueChange={setValue}>
        <div className="flex">
          <TabsList className="mx-auto">
            <div>
              <TabsTrigger value="general" className="text-xs ">
                Standard charts
              </TabsTrigger>
              <TabsTrigger value="saved" className="text-xs">
                Custom charts
              </TabsTrigger>
            </div>
          </TabsList>
        </div>
        <TabsContent value="general" className="h-full fixed overflow-hidden pb-10">
          <ScrollArea className="h-full">
            <SidebarMenu items={enabledItems} />
            <div className="relative mt-4">
              <div className="absolute -top-2 left-4 right-4 z-10">
                <div className=" rounded-md py-1.5 px-3 text-center">
                  <span className="text-xs font-medium text-muted-foreground">Coming soon</span>
                </div>
              </div>
              <div className="pointer-events-none select-none opacity-40 pt-6">
                <SidebarMenu items={comingSoonItems} />
              </div>
            </div>
            <div className="py-16"></div>
          </ScrollArea>
        </TabsContent>
        <TabsContent value="saved">
          <div className="p-4">No saved charts</div>
        </TabsContent>
      </Tabs>
    </aside>
  )
}
