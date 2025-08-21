import { Tabs, TabsContent, TabsList, TabsTrigger } from '@md/ui'
import { FunctionComponent } from 'react'

import { CompanyTab } from '@/features/settings/tabs/CompanyTab'
import { GeneralTab } from '@/features/settings/tabs/GeneralTab'
import { IntegrationsTab } from '@/features/settings/tabs/IntegrationsTab'
import { InvoiceTab } from '@/features/settings/tabs/InvoiceTab'
import { PaymentMethodsTab } from '@/features/settings/tabs/PaymentsTab'
import { TaxesTab } from '@/features/settings/tabs/TaxesTab'
import { UsersTab } from '@/features/settings/tabs/UsersTab'
import { useQueryState } from '@/hooks/useQueryState'

export const TenantSettings: FunctionComponent = () => {
  const [tab, setTab] = useQueryState('tab', 'general')

  return (
    <>
      <div className="  space-y-6 w-full h-full overflow-x-hidden">
        <Tabs defaultValue={tab} onValueChange={setTab} className="w-full ">
          <TabsList className="w-full justify-start">
            <TabsTrigger value="general">General</TabsTrigger>
            <TabsTrigger value="merchant">Merchant</TabsTrigger>
            <TabsTrigger value="invoices">Invoices</TabsTrigger>
            <TabsTrigger value="integrations">Integrations</TabsTrigger>
            <TabsTrigger value="payments">Payment methods</TabsTrigger>
            <TabsTrigger value="taxes">Taxes</TabsTrigger>
            <TabsTrigger value="users">Users</TabsTrigger>
          </TabsList>
          <TabsContent value="general">
            <GeneralTab />
          </TabsContent>
          <TabsContent value="merchant">
            <CompanyTab />
          </TabsContent>
          <TabsContent value="invoices">
            <InvoiceTab />
          </TabsContent>
          <TabsContent value="integrations">
            <IntegrationsTab />
          </TabsContent>
          <TabsContent value="payments">
            <PaymentMethodsTab />
          </TabsContent>
          <TabsContent value="taxes">
            <TaxesTab />
          </TabsContent>
          <TabsContent value="users">
            <UsersTab />
          </TabsContent>
        </Tabs>
      </div>
    </>
  )
}
