import { PlusIcon, SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@ui/components'
import { FileUpIcon, RefreshCwIcon } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { Link } from 'react-router-dom'

import PageHeading from '@/components/PageHeading/PageHeading'
import { MultiFilter } from '@/features/TablePage'
import { SubscriptionsImportModal } from '@/features/subscriptions/modals/SubscriptionsImportModal'
import { SetQueryStateAction } from '@/hooks/useQueryState'

interface SubscriptionsProps {
  count: number
  isLoading: boolean
  refetch: () => void
  statusFilter: string[]
  setStatusFilter: (value: SetQueryStateAction<string[]>) => void
  onImportSuccess?: () => void
}

export const SubscriptionsHeader: FunctionComponent<SubscriptionsProps> = ({
  count,
  isLoading,
  refetch,
  statusFilter,
  setStatusFilter,
  onImportSuccess,
}) => {
  const [importVisible, setImportVisible] = useState(false)

  return (
    <>
      <div className="flex flex-col gap-8">
        <div className="flex flex-row items-center justify-between">
          <PageHeading count={count}>Subscriptions</PageHeading>
          <div className="flex flex-row gap-2">
            <Button variant="secondary" size="sm" onClick={() => setImportVisible(true)}>
              <FileUpIcon className="h-4 w-4 mr-2" />
              Import CSV
            </Button>
            <Link to="create">
              <Button variant="primary" hasIcon>
                <PlusIcon size={10} fill="white" /> New subscription
              </Button>
            </Link>
          </div>
        </div>
        <div className="flex flex-row items-center gap-2">
          <InputWithIcon
            placeholder="Search subscriptions"
            icon={<SearchIcon size={16} />}
            width="fit-content"
            disabled
          />
          <MultiFilter
            emptyLabel="All statuses"
            entries={[
              { label: 'Pending', value: 'pending' },
              { label: 'Trialing', value: 'trialing' },
              { label: 'Active', value: 'active' },
              { label: 'Canceled', value: 'canceled' },
              { label: 'Ended', value: 'ended' },
              { label: 'Trial Expired', value: 'trial_expired' },
              { label: 'Errored', value: 'errored' },
            ]}
            hook={[statusFilter, setStatusFilter]}
          />
          <Button variant="secondary" disabled={isLoading} onClick={refetch}>
            <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
          </Button>
        </div>
      </div>
      <SubscriptionsImportModal
        openState={[importVisible, setImportVisible]}
        onSuccess={onImportSuccess}
      />
    </>
  )
}
