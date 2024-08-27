import {
  Button,
  Command,
  CommandEmpty,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@md/ui'
import { ChevronsUpDown, PlusIcon } from 'lucide-react'
import { Link } from 'react-router-dom'

import { useQuery } from '@/lib/connectrpc'
import { useTypedParams } from '@/lib/utils/params'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

import type { FunctionComponent } from 'react'

const FamilyPicker: FunctionComponent = () => {
  const familiesQuery = useQuery(listProductFamilies)
  const families = familiesQuery.data?.productFamilies ?? []

  // TODO query params ?
  const { familyExternalId } = useTypedParams<{ familyExternalId: string }>()
  const selected = families.find(fam => fam.externalId === familyExternalId) || families[0]

  return (
    <Popover>
      <PopoverTrigger>
        <Button variant="special" className=" rounded-full ">
          <div className="flex flex-row space-x-2 items-center ">
            <span>
              <span className="text-xs text-muted-foreground">Product line: </span>
              <span>{selected?.name}</span>
            </span>

            <ChevronsUpDown size="10" />
          </div>
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[200px] p-0">
        <Command>
          <CommandEmpty>No product family found.</CommandEmpty>
          <CommandList>
            {families
              .sort((a, b) => a.name.localeCompare(b.name))
              .map(family => (
                <Link key={family.id} to={family.externalId}>
                  <CommandItem key={family.id}>{family.name}</CommandItem>
                </Link>
              ))}
          </CommandList>
          <CommandItem>
            <Link to="/tenants/new" className="w-full">
              <Button size="content" variant="ghost" hasIcon>
                <PlusIcon size="12" /> New product family
              </Button>
            </Link>
          </CommandItem>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

export default FamilyPicker
