import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { CountryFlag } from '@ui/components'
import { Eye, FolderSyncIcon, MoreVerticalIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'

import {
  IntegrationType,
  SyncCustomerModal,
} from '@/features/settings/integrations/SyncCustomerModal'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import { CustomerBrief } from '@/rpc/api/customers/v1/models_pb'

const CustomerRowActions = ({
  customerId,
  customerName,
}: {
  customerId: string
  customerName: string
}) => {
  const basePath = useBasePath()
  const navigate = useNavigate()

  const [showSyncHubspotModal, setShowSyncHubspotModal] = useState(false)
  const [showSyncPennylaneModal, setShowSyncPennylaneModal] = useState(false)

  const connectorsQuery = useQuery(listConnectors, {})
  const connectorsData = connectorsQuery.data?.connectors ?? []
  const isHubspotConnected = connectorsData.some(
    c => c.provider === ConnectorProviderEnum.HUBSPOT
  )
  const isPennylaneConnected = connectorsData.some(
    c => c.provider === ConnectorProviderEnum.PENNYLANE
  )

  return (
    <div onClick={e => e.stopPropagation()}>
      {showSyncHubspotModal && (
        <SyncCustomerModal
          name={customerName}
          id={customerId}
          integrationType={IntegrationType.Hubspot}
          onClose={() => setShowSyncHubspotModal(false)}
        />
      )}
      {showSyncPennylaneModal && (
        <SyncCustomerModal
          name={customerName}
          id={customerId}
          integrationType={IntegrationType.Pennylane}
          onClose={() => setShowSyncPennylaneModal(false)}
        />
      )}

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <MoreVerticalIcon size={16} className="cursor-pointer" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onClick={() => navigate(`${basePath}/customers/${customerId}`)}>
            <Eye size={16} className="mr-2" />
            View
          </DropdownMenuItem>
          <Tooltip>
            <TooltipTrigger asChild>
              <span>
                <DropdownMenuItem
                  disabled={!isHubspotConnected}
                  onClick={() => setShowSyncHubspotModal(true)}
                >
                  <FolderSyncIcon size={16} className="mr-2" />
                  Sync to Hubspot
                </DropdownMenuItem>
              </span>
            </TooltipTrigger>
            {!isHubspotConnected && (
              <TooltipContent>Hubspot integration not connected</TooltipContent>
            )}
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <span>
                <DropdownMenuItem
                  disabled={!isPennylaneConnected}
                  onClick={() => setShowSyncPennylaneModal(true)}
                >
                  <FolderSyncIcon size={16} className="mr-2" />
                  Sync to Pennylane
                </DropdownMenuItem>
              </span>
            </TooltipTrigger>
            {!isPennylaneConnected && (
              <TooltipContent>Pennylane integration not connected</TooltipContent>
            )}
          </Tooltip>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}

export const useCustomersColumns = () =>
  useMemo<ColumnDef<CustomerBrief>[]>(
    () => [
      {
        header: 'Name',
        cell: ({ row }) => <Link to={`${row.original.id}`}>{row.original.name}</Link>,
      },
      {
        header: 'Country',
        cell: ({ row }) => <CountryFlag name={row.original.country} />,
      },
      {
        header: 'Email',
        accessorFn: cell => cell.billingEmail,
      },
      {
        header: 'Alias',
        accessorFn: cell => cell.alias,
      },
      {
        header: 'Accrued',
        accessorFn: () => '-',
      },
      {
        accessorKey: 'id',
        header: '',
        className: 'w-2',
        cell: ({ row }) => (
          <CustomerRowActions
            customerId={row.original.id}
            customerName={row.original.name}
          />
        ),
      },
    ],
    []
  )
