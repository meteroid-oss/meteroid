import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@ui2/components'
import { Check, ChevronDownIcon, PlusIcon } from 'lucide-react'
import { Link } from 'react-router-dom'

import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

export const TenantDropdown = () => {
  const tenants = useQuery(listTenants).data?.tenants ?? []

  const { tenant } = useTenant()

  return (
    <Popover>
      <PopoverTrigger>
        <Button variant="secondary" className=" rounded-full">
          <div className="flex flex-row space-x-2 items-center ">
            <span className=" rounded-full p-1 bg-cyan-600" />
            <span>{tenant?.name}</span>
            <ChevronDownIcon size="12" />
          </div>
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[200px] p-0">
        <Command>
          <CommandEmpty>No tenant found.</CommandEmpty>
          <CommandGroup>
            {tenants
              .sort((a, b) => a.name.localeCompare(b.name))
              .map(tenant => (
                <Link key={tenant.id} to={`/tenant/${tenant.slug}`}>
                  <CommandItem key={tenant.id}>{tenant.name}</CommandItem>
                </Link>
              ))}
          </CommandGroup>
          <CommandItem>
            <Link to="/tenants/new" className="w-full">
              <Button size="content" variant="ghost" hasIcon>
                <PlusIcon size="12" /> New tenant
              </Button>
            </Link>
          </CommandItem>
        </Command>
      </PopoverContent>
    </Popover>
    // <Dropdown
    //   side="bottom"
    //   align="start"
    //   overlay={
    //     <>
    //       {tenants
    //         .sort((a, b) => a.name.localeCompare(b.name))
    //         .map(tenant => (
    //           <Link key={tenant.id} to={`/tenant/${tenant.slug}`}>
    //             <Dropdown.Item>{tenant.name}</Dropdown.Item>
    //           </Link>
    //         ))}
    //       <Popover.Separator />
    //       <Link to="/tenants/new">
    //         <Dropdown.Item icon={<PlusIcon size="12" />}>New tenant</Dropdown.Item>
    //       </Link>
    //     </>
    //   }
    // >
    //   <Button as="span" type="text" size="small" className="border border-slate-700 rounded-full">
    //     <div className="flex flex-row space-x-2 items-center ">
    //       <span className=" rounded-full p-1 bg-cyan-600" />
    //       <span>{tenant?.name}</span>
    //       <ChevronDownIcon size="12" />
    //     </div>
    //   </Button>
    // </Dropdown>
  )
}
