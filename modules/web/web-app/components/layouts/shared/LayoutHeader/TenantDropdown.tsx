import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@md/ui'
import { ChevronsUpDownIcon, PlusIcon } from 'lucide-react'
import { Link } from 'react-router-dom'

import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'
import React, { useEffect } from 'react'

export const TenantDropdown = () => {
  const tenants = useQuery(listTenants).data?.tenants ?? []

  const { tenant } = useTenant()

  const [open, setOpen] = React.useState(false)

  useEffect(() => {
    setOpen(false)
  }, [tenant])

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger>
        <Button variant="special" className=" rounded-full">
          <div className="flex flex-row space-x-2 items-center ">
            <span className="text-xs text-muted-foreground">Tenant: </span>
            <span className=" rounded-full p-1 bg-cyan-600" />
            <span>{tenant?.name}</span>
            <ChevronsUpDownIcon size="10" />
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
  )
}
