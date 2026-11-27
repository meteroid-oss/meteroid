import {
  Badge,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { ColumnDef, Row } from '@tanstack/react-table'
import { CountryFlag } from '@ui/components'
import { Eye, FolderSyncIcon, MoreVerticalIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Link, useNavigate } from 'react-router'

import { Gate } from '@/components/Gate'
import { InitialAvatar } from '@/components/icons/AppIcons'
import { slotItems } from '@/edition'
import { MemberAvatar } from '@/features/accounts/components/MemberAvatar'
import { MemberPicker } from '@/features/accounts/components/MemberPicker'
import { useAssignOwner } from '@/features/accounts/hooks/useAssignOwner'
import {
  IntegrationType,
  SyncCustomerModal,
} from '@/features/settings/integrations/SyncCustomerModal'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { lifecycleStateLabel, lifecycleStateVariant } from '@/lib/lifecycle'
import { formatCurrency } from '@/lib/utils/numbers'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import { CustomerBrief } from '@/rpc/api/customers/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

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
  const isHubspotConnected = connectorsData.some(c => c.provider === ConnectorProviderEnum.HUBSPOT)
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
          <MoreVerticalIcon
            size={16}
            className="cursor-pointer text-faint-foreground hover:text-foreground transition-colors"
          />
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

export const useCustomersColumns = ({ rows }: { rows: CustomerBrief[] }) => {
  const isExpress = useIsExpressOrganization()
  const moduleColumns = slotItems('customerList.columns').filter(column => column.isVisible(rows))
  const moduleColumnIds = moduleColumns.map(column => column.id).join(',')
  return useMemo<ColumnDef<CustomerBrief>[]>(
    () => [
      {
        id: 'name',
        header: 'Name',
        enableSorting: true,
        cell: ({ row }) => (
          <Link to={`${row.original.id}`} className="flex items-center gap-2 font-medium">
            <InitialAvatar name={row.original.name} />
            <span className="truncate">{row.original.name}</span>
          </Link>
        ),
      },
      {
        id: 'alias',
        header: 'Alias',
        className: 'hidden 2xl:table-cell',
        accessorFn: cell => cell.alias,
        // Secondary to the name, like the country: muted.
        cell: ({ row }) => <span className="text-muted-foreground">{row.original.alias}</span>,
      },
      {
        id: 'mrr',
        header: 'MRR',
        enableSorting: true,
        cell: ({ row }) =>
          row.original.mrrCents > 0n ? (
            <span className="tabular-nums">
              {formatCurrency(row.original.mrrCents, row.original.currency)}
            </span>
          ) : (
            <span className="text-muted-foreground">—</span>
          ),
      },
      {
        id: 'lifecycle',
        header: 'Status',
        enableSorting: false,
        cell: ({ row }) =>
          row.original.lifecycleState ? (
            <Badge variant={lifecycleStateVariant(row.original.lifecycleState)}>
              {lifecycleStateLabel(row.original.lifecycleState)}
            </Badge>
          ) : null,
      },
      {
        id: 'email',
        header: 'Email',
        className: 'hidden lg:table-cell max-w-[16rem] truncate',
        accessorFn: cell => cell.billingEmail,
      },
      {
        id: 'country',
        header: 'Country',
        className: 'hidden md:table-cell text-right',
        // Secondary to the name: muted, as the other supporting columns; to the right.
        cell: ({ row }) => (
          <span className="flex justify-end text-muted-foreground">
            <CountryFlag name={row.original.country} />
          </span>
        ),
        enableSorting: false,
      },
      ...(isExpress
        ? []
        : [
            {
              id: 'owner',
              header: 'Owner',
              enableSorting: false,
              className: 'w-20',
              // Picked in the cell itself: not a link to the customer.
              meta: { skipLink: true },
              cell: ({ row }: { row: Row<CustomerBrief> }) => <OwnerCell customer={row.original} />,
            } as ColumnDef<CustomerBrief>,
          ]),
      ...moduleColumns.map(
        ({ id, header, Cell }) =>
          ({
            id,
            header,
            enableSorting: false,
            cell: ({ row }) => <Cell customer={row.original} />,
          }) satisfies ColumnDef<CustomerBrief>
      ),
      {
        id: 'created_at',
        header: 'Created',
        className: 'hidden xl:table-cell whitespace-nowrap',
        enableSorting: true,
        cell: ({ row }) => (
          <span className="text-muted-foreground">
            {parseAndFormatDate(row.original.createdAt)}
          </span>
        ),
      },
      {
        id: 'actions',
        accessorKey: 'id',
        header: '',
        className: 'w-2',
        enableSorting: false,
        cell: ({ row }) => (
          <Gate perm="customers:manage">
            <CustomerRowActions customerId={row.original.id} customerName={row.original.name} />
          </Gate>
        ),
      },
    ],
    // The visible module columns are identified by their ids.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [moduleColumnIds, isExpress]
  )
}

/**
 * The account owner: their initials (the name in a tooltip, as a name would crowd the row), or a
 * dashed circle when nobody owns the account. Who may manage customers picks the owner here.
 */
const OwnerCell = ({ customer }: { customer: CustomerBrief }) => {
  const canAssign = usePermissions().has('customers:manage')
  const assign = useAssignOwner()
  const ownerId = customer.owner?.userId

  if (!canAssign) {
    return ownerId ? (
      <MemberAvatar userId={ownerId} />
    ) : (
      <span
        aria-label="No owner"
        role="img"
        className="block h-5 w-5 rounded-full border border-dashed border-foreground/25"
      />
    )
  }

  return (
    <MemberPicker
      compact
      // The column sits on the right: the menu opens from the avatar's right edge, leftwards.
      align="end"
      value={ownerId}
      label={`Owner of ${customer.name}`}
      disabled={assign.isPending}
      onChange={ownerUserId => assign.mutate({ customerIds: [customer.id], ownerUserId })}
    />
  )
}
