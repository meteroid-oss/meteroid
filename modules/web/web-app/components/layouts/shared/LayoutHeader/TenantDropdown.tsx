import {
  Button,
  Command,
  CommandEmpty,
  CommandItem,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  Flex,
  useSidebar,
} from '@md/ui'
import { CommandList } from 'cmdk'
import { ChevronsUpDownIcon, PlusIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'

import { useOrganizationSlug } from '@/hooks/useOrganization'
import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

const getColor = (environment: TenantEnvironmentEnum | undefined) => {
  switch (environment) {
    case TenantEnvironmentEnum.PRODUCTION:
      return 'bg-cyan-600'
    default:
      return 'bg-yellow-500'
  }
}

export const TenantDropdown = () => {
  const { tenant } = useTenant()
  const org = useOrganizationSlug()
  const { state } = useSidebar()

  const tenants = useQuery(listTenants).data?.tenants ?? []

  const [open, setOpen] = useState(false)

  useEffect(() => {
    setOpen(false)
  }, [tenant])

  const tenantColor = (
    <span className={`rounded-full p-1 h-4 w-4 ${getColor(tenant?.environment)}`} />
  )

  return state === 'collapsed' ? (
    <div className={`h-4 w-4 rounded-full ${getColor(tenant?.environment)} text-center`} />
  ) : (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger className="w-full">
        <Flex
          align="center"
          justify="between"
          className="h-8 px-4 py-2 rounded-full border border-[#FFFFFF15] tenant-popover-bg w-full hover:bg-sidebar-accent"
        >
          <Flex align="center" className="gap-2">
            {tenantColor}
            <span className="max-w-36 overflow-hidden text-nowrap text-xs" title={tenant?.name}>
              {tenant?.name}
            </span>
          </Flex>
          <ChevronsUpDownIcon size="13" className="text-muted-foreground" />
        </Flex>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-[246px] p-0">
        <Command>
          <CommandEmpty>No tenant found.</CommandEmpty>
          <CommandList>
            {tenants
              .sort((a, b) => a.name.localeCompare(b.name))
              .map(tenant => (
                <Link key={tenant.id} to={`/${org}/${tenant.slug}`}>
                  <CommandItem key={tenant.id} className="flex flex-row space-x-2 items-center ">
                    <span className={`rounded-full p-1 ${getColor(tenant?.environment)}`} />
                    <span>{tenant.name}</span>
                  </CommandItem>
                </Link>
              ))}
          </CommandList>
          <CommandItem>
            <Link to={`/${org}/tenants/new`} className="w-full text-xs">
              <Button size="content" variant="ghost" hasIcon className="text-xs">
                <PlusIcon size="12" /> New tenant
              </Button>
            </Link>
          </CommandItem>
        </Command>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
