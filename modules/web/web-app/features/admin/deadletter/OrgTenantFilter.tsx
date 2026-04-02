import {
  Button,
  Command,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  cn,
} from '@md/ui'
import { CaretSortIcon, CheckIcon, Cross2Icon } from '@radix-ui/react-icons'
import { useEffect, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { searchOrganizations } from '@/rpc/admin/deadletter/v1/deadletter-DeadLetterService_connectquery'
import { OrganizationItem } from '@/rpc/admin/deadletter/v1/deadletter_pb'
import { useDebounceValue } from '@/hooks/useDebounce'

interface Props {
  organizationId: string | undefined
  tenantId: string | undefined
  onOrganizationChange: (orgId: string | undefined) => void
  onTenantChange: (tenantId: string | undefined) => void
}

export const OrgTenantFilterSelect = ({
  organizationId,
  tenantId,
  onOrganizationChange,
  onTenantChange,
}: Props) => {
  const [open, setOpen] = useState(false)
  const [search, setSearch] = useState('')
  const debouncedSearch = useDebounceValue(search, 300)

  const orgsQuery = useQuery(searchOrganizations, {
    query: debouncedSearch,
    limit: 10,
  })

  const orgs = orgsQuery.data?.organizations ?? []
  const selectedOrg = orgs.find(o => o.id === organizationId)

  // Keep a stable reference to the selected org's tenants
  const [selectedOrgData, setSelectedOrgData] = useState<OrganizationItem | undefined>()

  useEffect(() => {
    if (selectedOrg) {
      setSelectedOrgData(selectedOrg)
    }
  }, [selectedOrg])

  const tenants = selectedOrgData?.tenants ?? []

  const orgLabel = selectedOrgData
    ? `${selectedOrgData.tradeName} (${selectedOrgData.slug})`
    : undefined

  return (
    <div className="flex items-center gap-2">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            variant="outline"
            role="combobox"
            aria-expanded={open}
            className="w-[260px] justify-between font-normal"
          >
            <span className="truncate">{orgLabel ?? 'All organizations'}</span>
            <CaretSortIcon className="ml-2 h-4 w-4 shrink-0 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-[260px] p-0">
          <Command shouldFilter={false}>
            <CommandInput
              placeholder="Search organizations..."
              value={search}
              onValueChange={setSearch}
            />
            <CommandList>
              <CommandEmpty>
                {orgsQuery.isLoading ? 'Searching...' : 'No organizations found'}
              </CommandEmpty>
              {orgs.map(org => (
                <CommandItem
                  key={org.id}
                  value={org.id}
                  onSelect={() => {
                    if (organizationId === org.id) {
                      onOrganizationChange(undefined)
                      onTenantChange(undefined)
                      setSelectedOrgData(undefined)
                    } else {
                      onOrganizationChange(org.id)
                      onTenantChange(undefined)
                      setSelectedOrgData(org)
                    }
                    setOpen(false)
                  }}
                >
                  <div className="flex flex-col">
                    <span className="text-sm">{org.tradeName}</span>
                    <span className="text-xs text-muted-foreground">{org.slug}</span>
                  </div>
                  <CheckIcon
                    className={cn(
                      'ml-auto h-4 w-4',
                      organizationId === org.id ? 'opacity-100' : 'opacity-0'
                    )}
                  />
                </CommandItem>
              ))}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>

      {organizationId && (
        <Button
          variant="ghost"
          size="sm"
          className="h-9 px-2"
          onClick={() => {
            onOrganizationChange(undefined)
            onTenantChange(undefined)
            setSelectedOrgData(undefined)
          }}
        >
          <Cross2Icon className="h-4 w-4" />
        </Button>
      )}

      {organizationId && tenants.length > 0 && (
        <Select
          value={tenantId ?? 'all'}
          onValueChange={v => onTenantChange(v === 'all' ? undefined : v)}
        >
          <SelectTrigger className="w-[180px]">
            <SelectValue placeholder="All tenants" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All tenants</SelectItem>
            {tenants.map(t => (
              <SelectItem key={t.id} value={t.id}>
                {t.name} ({t.slug})
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
    </div>
  )
}
