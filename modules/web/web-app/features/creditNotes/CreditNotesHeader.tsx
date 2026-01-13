import { SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@md/ui'
import { RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import PageHeading from '@/components/PageHeading/PageHeading'
import { FilterDropdown } from '@/features/creditNotes/FilterDropdown'
import { CreditNotesSearch } from '@/features/creditNotes/types'

type CreditNotesHeaderProps = {
  count: number
  isLoading: boolean
  refetch: () => void
  setSearch: (search: CreditNotesSearch) => void
  search: CreditNotesSearch
}

export const CreditNotesHeader: FunctionComponent<CreditNotesHeaderProps> = ({
  count,
  isLoading,
  refetch,
  setSearch,
  search,
}) => {
  return (
    <div className="flex flex-col gap-8">
      <div className="flex flex-row items-center justify-between">
        <PageHeading count={count}>Credit Notes</PageHeading>
      </div>
      <div className="flex flex-row items-center gap-2">
        <InputWithIcon
          placeholder="Search by customer"
          icon={<SearchIcon size={16} />}
          width="fit-content"
          value={search.text}
          onChange={e => setSearch({ ...search, text: e.target.value })}
        />
        <Button variant="secondary" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
        </Button>
        <FilterDropdown
          status={search.status}
          setStatus={value => setSearch({ ...search, status: value })}
        />
      </div>
    </div>
  )
}
