import { Tabs, TabsContent, TabsList, TabsTrigger } from '@ui/components'
import { FunctionComponent } from 'react'

import { TenantDropdown } from '@/components/layouts/shared/LayoutHeader/TenantDropdown'
import { CompanyTab } from '@/features/settings/tabs/CompanyTab'
import { InvoiceTab } from '@/features/settings/tabs/InvoiceTab'
import { UsersTab } from '@/features/settings/tabs/UsersTab'

export const TenantSettings: FunctionComponent = () => {
  return (
    <>
      <div className=" px-6 py-3 space-y-6 bg-slate-100 dark:bg-inherit w-full h-full overflow-x-hidden">
        <div className="space-y-2">
          <div className="flex space-x-4 items-center pb-2">
            <h3>Tenant Settings</h3>
            <TenantDropdown />
          </div>

          <div className="border-b border-slate-400" />
        </div>
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
