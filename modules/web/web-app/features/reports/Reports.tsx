import { ScrollArea, Tabs, TabsContent, TabsList, TabsTrigger } from '@ui/components'
import { useState } from 'react'

import SidebarMenu from '@/components/SidebarMenu'
import { TenantPageLayout } from '@/components/layouts'
import { MrrReport } from '@/features/reports/charts/MrrReport'
export const Reports = () => {
  return (
    <TenantPageLayout title="Reports " displayTitle={true}>
      <div className="flex">
        <ReportsNavigation />

        <main className="flex  flex-col flex-1 w-full max-w-screen-2xl pl-8 pr-2 mx-auto h-full overflow-x-hidden ">
          <MrrReport />
        </main>
      </div>
    </TenantPageLayout>
  )
}

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
            <SidebarMenu
              items={[
                {
                  label: 'Recurring Revenue',
                  items: [
                    {
                      label: 'MRR',
                      to: '.',
                    },
                    {
                      label: 'MRR Movements',
                      to: 'mrr-movements',
                    },
                    {
                      label: 'Net MRR Movements',
                      to: 'net-mrr-movements',
                    },
                    {
                      label: 'Annual Run Rate',
                      to: 'annual-run-rate',
                    },
                    {
                      label: 'Committed MRR Forecast',
                      to: 'committed-mrr-forecast',
                    },
                  ],
                },
                {
                  label: 'Leads and Conversion',
                  items: [
                    {
                      label: 'Leads',
                      to: 'leads',
                    },
                    {
                      label: 'Free Trials',
                      to: 'free-trials',
                    },
                    {
                      label: 'Trial-to-Paid Conversion Rate',
                      to: 'trial-paid-conversion-rate',
                    },
                    {
                      label: 'Pipeline Funnel Analysis',
                      to: 'pipeline-funnel-analysis',
                    },
                    {
                      label: 'Average Sales Cycle Length',
                      to: 'average-sales-cycle-length',
                    },
                  ],
                },
                {
                  label: 'Subscribers',
                  items: [
                    {
                      label: 'Subscribers',
                      to: 'subscribers',
                    },
                    {
                      label: 'Average Revenue Per Account',
                      to: 'average-revenue-per-account',
                    },
                    {
                      label: 'Average Sale Price',
                      to: 'average-sale-price',
                    },
                    {
                      label: 'Customer Lifetime Value',
                      to: 'customer-lifetime-value',
                    },
                    {
                      label: 'Subscriptions',
                      to: 'subscriptions',
                    },
                    {
                      label: 'Subscription Quantity',
                      to: 'subscription-quantity',
                    },
                  ],
                },
                {
                  label: 'Churn',
                  items: [
                    {
                      label: 'Customer Churn Rate',
                      to: 'customer-churn-rate',
                    },
                    {
                      label: 'Net MRR Churn Rate',
                      to: 'net-mrr-churn-rate',
                    },
                    {
                      label: 'Gross MRR Churn Rate',
                      to: 'gross-mrr-churn-rate',
                    },
                    {
                      label: 'Quantity Churn Rate',
                      to: 'quantity-churn-rate',
                    },
                  ],
                },
                {
                  label: 'Retention',
                  items: [
                    {
                      label: 'Net MRR Retention',
                      to: 'net-mrr-retention',
                    },
                    {
                      label: 'Gross MRR Retention',
                      to: 'gross-mrr-retention',
                    },
                  ],
                },
                {
                  label: 'Transactions',
                  items: [
                    {
                      label: 'Net Cash Flow',
                      to: 'net-cash-flow',
                    },
                    {
                      label: 'Gross Cash Flow',
                      to: 'gross-cash-flow',
                    },
                    {
                      label: 'Non-Recurring Cash Flow',
                      to: 'non-recurring-cash-flow',
                    },
                    {
                      label: 'Successful Payments',
                      to: 'successful-payments',
                    },
                    {
                      label: 'Refunds',
                      to: 'refunds',
                    },
                    {
                      label: 'Failed Transactions',
                      to: 'failed-transactions',
                    },
                  ],
                },
              ]}
            />
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
