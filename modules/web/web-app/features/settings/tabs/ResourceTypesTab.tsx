import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  cn,
} from '@md/ui'
import { Boxes, MoreVerticalIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { ResourceTypeSheet } from '@/features/resources/ResourceTypeSheet'
import { TypeDetailSheet } from '@/features/resources/TypeDetail'
import { attributeKeyLabel, humanize, useInvalidateResources, useResourceTypes } from '@/features/resources/hooks'
import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { useQuery } from '@/lib/connectrpc'
import { updateResourceType } from '@/rpc/api/resources/v1/resources-ResourceTypesService_connectquery'
import { listResources } from '@/rpc/api/resources/v1/resources-ResourcesService_connectquery'

import type { ResourceType } from '@/rpc/api/resources/v1/models_pb'

export const ResourceTypesTab = () => {
  const { all, isLoading } = useResourceTypes()
  const [editing, setEditing] = useState<{ type?: ResourceType }>()
  const [viewing, setViewing] = useState<ResourceType>()
  const invalidate = useInvalidateResources()
  const archive = useMutation(updateResourceType, {
    onSuccess: res => {
      toast.success(res.resourceType?.archived ? 'Resource type archived' : 'Resource type restored')
      return invalidate()
    },
    onError: err => toast.error(err.message),
  })

  const attributes = useMemo(
    () =>
      all
        .filter(t => !t.archived)
        .flatMap(t =>
          Object.entries(t.attributes)
            .sort(([a], [b]) => a.localeCompare(b))
            .map(([name, { values }]) => ({ type: t, name, values }))
        ),
    [all]
  )

  return (
    <div className="flex flex-col gap-6">
      <SettingsSection
        divided={false}
        title="Resource types"
        action={
          <Button size="sm" onClick={() => setEditing({})}>
            New resource type
          </Button>
        }
      >
        {isLoading ? (
          <Skeleton className="h-24 w-full" />
        ) : all.length === 0 ? (
          <div className="rounded-lg border border-border">
            <PanelEmptyState icon={Boxes} title="No resource types" />
          </div>
        ) : (
          <div className="overflow-hidden rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Type</TableHead>
                  <TableHead>Identified by</TableHead>
                  <TableHead>Attributes</TableHead>
                  <TableHead className="w-px whitespace-nowrap">Resources</TableHead>
                  <TableHead className="w-px" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {all.map(t => (
                  <TableRow key={t.id} className={cn(t.archived && 'opacity-60')}>
                    <TableCell>
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium">{t.name}</span>
                        {t.archived && (
                          <Badge variant="outline" size="sm">
                            Archived
                          </Badge>
                        )}
                      </div>
                      <span className="font-mono text-xs text-muted-foreground">{t.code}</span>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {t.keys.map(k => (
                          <Badge key={k} variant="outline" size="sm" className="font-mono font-normal">
                            {k}
                          </Badge>
                        ))}
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {Object.keys(t.attributes).length === 0 ? (
                          <span className="text-muted-foreground">—</span>
                        ) : (
                          Object.keys(t.attributes)
                            .sort()
                            .map(name => (
                              <Badge key={name} variant="outline" size="sm" className="font-normal">
                                {humanize(name)}
                              </Badge>
                            ))
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="w-px whitespace-nowrap">
                      <ResourceCount type={t} onOpen={() => setViewing(t)} />
                    </TableCell>
                    <TableCell className="w-px">
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button variant="ghost" size="icon" className="h-8 w-8" aria-label={`${t.name} actions`}>
                            <MoreVerticalIcon size={16} />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          {!t.archived && (
                            <DropdownMenuItem onClick={() => setEditing({ type: t })}>Edit</DropdownMenuItem>
                          )}
                          <DropdownMenuItem onClick={() => setViewing(t)}>View {t.pluralName.toLowerCase()}</DropdownMenuItem>
                          <DropdownMenuItem
                            disabled={archive.isPending}
                            onClick={() => archive.mutate({ idOrCode: t.id, archived: !t.archived })}
                          >
                            {t.archived ? 'Restore' : 'Archive'}
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </SettingsSection>

      {attributes.length > 0 && (
        <SettingsSection title="Attributes">
          <div className="overflow-hidden rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Attribute</TableHead>
                  <TableHead>Values</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {attributes.map(a => (
                  <TableRow key={`${a.type.id}.${a.name}`}>
                    <TableCell>
                      <div className="text-sm font-medium">{attributeKeyLabel(`${a.type.code}.${a.name}`, all)}</div>
                      <span className="font-mono text-xs text-muted-foreground">
                        {a.type.code}.{a.name}
                      </span>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {a.values.map(v => (
                          <Badge key={v} variant="outline" size="sm" className="font-normal">
                            {v}
                          </Badge>
                        ))}
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </SettingsSection>
      )}

      {editing && <ResourceTypeSheet {...editing} onClose={() => setEditing(undefined)} />}
      {viewing && <TypeDetailSheet type={viewing} onClose={() => setViewing(undefined)} />}
    </div>
  )
}

const ResourceCount = ({ type, onOpen }: { type: ResourceType; onOpen: () => void }) => {
  const query = useQuery(listResources, { resourceType: type.id, pagination: { page: 0, perPage: 1 } })
  if (query.isLoading) return <Skeleton className="h-4 w-8" />
  const total = query.data?.paginationMeta?.totalItems
  if (total === undefined) return <span className="text-muted-foreground">—</span>
  return (
    <button type="button" className="text-[13px] font-medium text-brand hover:underline" onClick={onOpen}>
      {total.toLocaleString()}
    </button>
  )
}
