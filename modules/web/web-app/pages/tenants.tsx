import { BadgeAlt, ButtonAlt } from '@ui/components'
import { ChevronRightIcon } from 'lucide-react'
import { Link } from 'react-router-dom'

import { FooterAccountDropdown } from '@/components/layouts/TenantLayout/NavigationBar/components/Footer/Footer'
import Header from '@/components/layouts/TenantLayout/NavigationBar/components/Header'
import { useQuery } from '@/lib/connectrpc'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

export const Tenants: React.FC = () => {
  const tenants = useQuery(listTenants)

  return (
    <div className="flex h-full">
      <nav>
        <Header />
        <FooterAccountDropdown />
      </nav>
      <div className="p-6 space-y-6 w-full">
        <div className="space-y-2">
          <h3 className="">Tenants</h3>
          <div className="border-b border-slate-400" />
        </div>
        <div className="">
          <ButtonAlt>New tenant</ButtonAlt>
        </div>
        <div>
          {tenants.data?.tenants?.map(tenant => {
            return (
              <Link key={tenant.id} to={`/tenant/${tenant.slug}`}>
                <div
                  key={tenant.id}
                  className="w-96 border border-slate-700 rounded-lg py-5 px-5 flex justify-between cursor-pointer"
                >
                  <div>
                    <div className="font-bold pb-6">{tenant.name}</div>
                    <div className="flex space-x-2">
                      <BadgeAlt>Sandbox</BadgeAlt>
                      <BadgeAlt color="blue">{tenant.currency}</BadgeAlt>
                    </div>
                  </div>
                  <div>
                    <ChevronRightIcon size={20} />
                  </div>
                </div>
              </Link>
            )
          })}
        </div>
      </div>
    </div>
  )
}
