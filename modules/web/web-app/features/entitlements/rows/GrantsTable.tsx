import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  cn,
} from '@md/ui'
import { MoreVerticalIcon } from 'lucide-react'
import { Fragment, type ReactNode } from 'react'

import { GroupHeaderRow } from '@/components/table/GroupHeaderRow'
import { FeatureStatusBadge } from '@/features/entitlements/features/FeatureStatusBadge'
import { type ProductRefLike, productRows } from '@/features/entitlements/productGroups'
import { FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'

import type { Feature } from '@/rpc/api/entitlements/v1/models_pb'
import type { LucideIcon } from 'lucide-react'

export type GrantAction = {
  label: string
  icon: LucideIcon
  onSelect: () => void
  destructive?: boolean
}

export type GrantsTableRow = {
  key: string
  featureId: string
  feature?: Feature
  /** Shown when the feature isn't loaded. */
  featureName?: string
  /** Groups the row while the feature isn't loaded; else the feature's. */
  product?: ProductRefLike
  value: ReactNode
  when: ReactNode
  actions: GrantAction[]
}

const productOfRow = (row: GrantsTableRow) => row.product ?? row.feature?.product

/** Rows of the same feature are consecutive; the feature shows on the first. Grouped by product. */
export const GrantsTable = ({
  rows: input,
  whenLabel = 'Only when',
}: {
  rows: GrantsTableRow[]
  whenLabel?: string
}) => {
  const { rows, headerAt } = productRows(input, productOfRow, { countBy: r => r.featureId })
  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <Table>
        <TableHeader className="border-t-0">
          <TableRow>
            <TableHead className="pl-3">Feature</TableHead>
            <TableHead>Value</TableHead>
            <TableHead>{whenLabel}</TableHead>
            <TableHead className="w-px">
              <span className="sr-only">Actions</span>
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row, i) => {
            const first = rows[i - 1]?.featureId !== row.featureId
            const continues = rows[i + 1]?.featureId === row.featureId
            const header = headerAt(i)
            return (
              <Fragment key={row.key}>
                {header && <GroupHeaderRow colSpan={4}>{header}</GroupHeaderRow>}
                <TableRow className={cn(continues && 'border-b-0')}>
                  <TableCell className="py-2.5 pl-3 align-top">
                    {first && <FeatureCell feature={row.feature} name={row.featureName ?? row.featureId} />}
                  </TableCell>
                  <TableCell className="py-2.5 align-top">{row.value}</TableCell>
                  <TableCell className="py-2.5 align-top">{row.when}</TableCell>
                  <TableCell className="w-px py-1.5 align-top">
                    {row.actions.length > 0 && <RowActions actions={row.actions} />}
                  </TableCell>
                </TableRow>
              </Fragment>
            )
          })}
        </TableBody>
      </Table>
    </div>
  )
}

const FeatureCell = ({ feature, name }: { feature?: Feature; name: string }) => (
  <div className="flex flex-col gap-0.5">
    <div className="flex flex-wrap items-center gap-2">
      <span className="text-sm font-medium">{feature?.name || name}</span>
      {feature?.status === FeatureStatus.DISABLED && <FeatureStatusBadge status={feature.status} />}
    </div>
    {feature?.code && <span className="font-mono text-xs text-muted-foreground">{feature.code}</span>}
  </div>
)

const RowActions = ({ actions }: { actions: GrantAction[] }) => {
  const regular = actions.filter(a => !a.destructive)
  const destructive = actions.filter(a => a.destructive)
  const item = (a: GrantAction) => (
    <DropdownMenuItem
      key={a.label}
      className={cn(a.destructive && 'text-destructive focus:text-destructive')}
      onClick={a.onSelect}
    >
      <a.icon size={16} className="mr-2" />
      {a.label}
    </DropdownMenuItem>
  )
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" className="h-8 w-8" aria-label="Actions">
          <MoreVerticalIcon size={16} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {regular.map(item)}
        {destructive.length > 0 && (
          <Fragment>
            {regular.length > 0 && <DropdownMenuSeparator />}
            {destructive.map(item)}
          </Fragment>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
