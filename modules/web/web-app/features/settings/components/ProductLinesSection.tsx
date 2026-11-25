import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Input } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CheckIcon, PencilIcon, XIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import {
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  createProductFamily,
  listProductFamilies,
  updateProductFamily,
} from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

import type { ProductFamily } from '@/rpc/api/productfamilies/v1/models_pb'

const useRefreshProductFamilies = () => {
  const queryClient = useQueryClient()
  return () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: listProductFamilies, cardinality: undefined }),
    })
}

/** The product lines of the workspace: each groups its own products and plans. */
export const ProductLinesSection = () => {
  const families = useQuery(listProductFamilies).data?.productFamilies ?? []
  const canManage = usePermissions().has('catalog:manage')
  const refresh = useRefreshProductFamilies()
  const [draft, setDraft] = useState('')

  const create = useMutation(createProductFamily, {
    onSuccess: async data => {
      await refresh()
      setDraft('')
      toast.success(`${data.productFamily?.name ?? 'Product line'} added`)
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The product line could not be added')),
  })

  const name = draft.trim()
  const canAdd = !!name && !create.isPending
  const add = () => canAdd && create.mutate({ name })

  return (
    <SettingsSection
      title="Product lines"
      description="A product line groups the plans its customers move between, e.g. Free, Pro and Enterprise of one product. Most workspaces need one."
    >
      <SettingsRowGroup>
        {families.map(family => (
          <ProductLineRow key={family.id} family={family} canManage={canManage} />
        ))}
        {canManage && (
          <SettingsRow
            title={
              <Input
                aria-label="New product line name"
                value={draft}
                placeholder="New product line"
                className="h-8 w-72 text-sm"
                onChange={e => setDraft(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter') {
                    e.preventDefault()
                    add()
                  }
                }}
              />
            }
          >
            <Button type="button" size="sm" disabled={!canAdd} onClick={add}>
              Add product line
            </Button>
          </SettingsRow>
        )}
      </SettingsRowGroup>
    </SettingsSection>
  )
}

const ProductLineRow = ({ family, canManage }: { family: ProductFamily; canManage: boolean }) => {
  const refresh = useRefreshProductFamilies()
  const [editing, setEditing] = useState(false)
  const [name, setName] = useState(family.name)

  const update = useMutation(updateProductFamily, {
    onSuccess: async () => {
      await refresh()
      setEditing(false)
      toast.success('Product line renamed')
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The product line could not be renamed')),
  })

  const next = name.trim()
  const canSave = !!next && next !== family.name && !update.isPending
  const save = () => canSave && update.mutate({ id: family.id, name: next })
  const cancel = () => {
    setName(family.name)
    setEditing(false)
  }

  if (editing) {
    return (
      <SettingsRow
        title={
          <Input
            autoFocus
            aria-label={`Name of ${family.name}`}
            value={name}
            className="h-8 w-72 text-sm"
            onChange={e => setName(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter') {
                e.preventDefault()
                save()
              } else if (e.key === 'Escape') {
                e.preventDefault()
                e.stopPropagation()
                cancel()
              }
            }}
          />
        }
      >
        <Button
          type="button"
          variant="ghost"
          size="sm"
          aria-label="Cancel renaming"
          onClick={cancel}
        >
          <XIcon size={14} />
        </Button>
        <Button type="button" size="sm" hasIcon disabled={!canSave} onClick={save}>
          <CheckIcon size={14} /> Save
        </Button>
      </SettingsRow>
    )
  }

  return (
    <SettingsRow
      title={family.name}
      description={<span className="font-mono">{family.localId}</span>}
    >
      {canManage && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          aria-label={`Rename ${family.name}`}
          onClick={() => {
            setName(family.name)
            setEditing(true)
          }}
        >
          <PencilIcon size={14} />
        </Button>
      )}
    </SettingsRow>
  )
}
