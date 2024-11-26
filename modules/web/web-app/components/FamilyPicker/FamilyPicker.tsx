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

import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

import type { FunctionComponent } from 'react'

const FamilyPicker: FunctionComponent = () => {
  const familiesQuery = useQuery(listProductFamilies)
  const families = familiesQuery.data?.productFamilies ?? []

  // TODO query params ?
  const [familyLocalId, setFamilyLocalId] = useQueryState('line', families[0]?.localId)

  const selected = families.find(fam => fam.localId === familyLocalId) || families[0]

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="special" className=" rounded-full ">
          <div className="flex flex-row space-x-2 items-center ">
            <span>
              <span className="text-xs text-muted-foreground">Product line: </span>
              <span className="max-w-36 overflow-hidden text-nowrap text-xs" title={selected?.name}>
                {selected?.name}
              </span>
            </span>

            <ChevronsUpDown size="10" />
          </div>
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[200px] p-0">
        <Command>
          <CommandEmpty>No product line found.</CommandEmpty>
          <CommandList>
            {families
              .sort((a, b) => a.name.localeCompare(b.name))
              .map(family => (
                <CommandItem key={family.localId} onSelect={setFamilyLocalId}>
                  {family.name}
                </CommandItem>
              ))}
          </CommandList>
          <CommandItem>
            <Link to="./settings?tab=products" className="w-full">
              <Button size="content" variant="ghost" hasIcon className="text-xs">
                <PlusIcon size="12" /> Configure
              </Button>
            </Link>
          </CommandItem>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

export default FamilyPicker
