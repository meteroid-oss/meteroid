import { Tabs, TabsContent, TabsList, TabsTrigger } from '@md/ui'
import { FunctionComponent } from 'react'

import { CompanyTab } from '@/features/settings/tabs/CompanyTab'
import { InvoiceTab } from '@/features/settings/tabs/InvoiceTab'
import { UsersTab } from '@/features/settings/tabs/UsersTab'

export const TenantSettings: FunctionComponent = () => {
  return (
    <>
      <div className="  space-y-6 w-full h-full overflow-x-hidden">
        <Tabs defaultValue="merchant" className="w-full ">
          <TabsList className="w-full justify-start">
            <TabsTrigger value="merchant">Merchant</TabsTrigger>
            <TabsTrigger value="invoices">Invoices</TabsTrigger>
            <TabsTrigger value="integrations">Integrations</TabsTrigger>
            <TabsTrigger value="taxes">Taxes</TabsTrigger>
            <TabsTrigger value="users">Users</TabsTrigger>
          </TabsList>
          <TabsContent value="merchant">
            <CompanyTab />
          </TabsContent>
          <TabsContent value="invoices">
            <InvoiceTab />
          </TabsContent>
          <TabsContent value="integrations">Not implemented</TabsContent>
          <TabsContent value="taxes">Not implemented</TabsContent>
          <TabsContent value="users">
            <UsersTab />
          </TabsContent>
        </Tabs>
      </div>
    </>
  )
}
