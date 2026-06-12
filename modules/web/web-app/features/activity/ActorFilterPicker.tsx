import {
  Input,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Skeleton,
} from '@md/ui'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { ActorType } from '@/rpc/api/activity/v1/activity_pb'
import { listApiTokens } from '@/rpc/api/apitokens/v1/apitokens-ApiTokensService_connectquery'
import { listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { listUsers } from '@/rpc/api/users/v1/users-UsersService_connectquery'

type Picked = { id: string; label: string }

type Props = {
  actorType: ActorType
  value?: Picked
  onChange: (picked: Picked | undefined) => void
}

export const ActorFilterPicker = ({ actorType, value, onChange }: Props) => {
  const [open, setOpen] = useState(false)
  const [search, setSearch] = useState('')

  if (!isSupported(actorType)) return null

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="flex items-center justify-between gap-2 h-9 rounded-md border border-input bg-input px-3 text-sm"
        >
          <span className={value ? 'text-foreground' : 'text-muted-foreground'}>
            {value ? value.label : `Any ${actorTypeLabel(actorType)}`}
          </span>
          {value && (
            <span
              role="button"
              tabIndex={0}
              className="text-muted-foreground hover:text-foreground"
              onClick={e => {
                e.stopPropagation()
                onChange(undefined)
              }}
            >
              ×
            </span>
          )}
        </button>
      </PopoverTrigger>
      <PopoverContent className="p-2 w-72" align="start">
        <Input
          autoFocus
          placeholder="Search…"
          value={search}
          onChange={e => setSearch(e.target.value)}
          className="mb-2"
        />
        <Results
          actorType={actorType}
          search={search}
          onPick={picked => {
            onChange(picked)
            setOpen(false)
            setSearch('')
          }}
        />
      </PopoverContent>
    </Popover>
  )
}

function isSupported(actorType: ActorType): boolean {
  return (
    actorType === ActorType.USER ||
    actorType === ActorType.API_TOKEN ||
    actorType === ActorType.CUSTOMER
  )
}

function actorTypeLabel(actorType: ActorType): string {
  switch (actorType) {
    case ActorType.USER:
      return 'user'
    case ActorType.API_TOKEN:
      return 'API token'
    case ActorType.CUSTOMER:
      return 'customer'
    default:
      return 'actor'
  }
}

const Results = ({
  actorType,
  search,
  onPick,
}: {
  actorType: ActorType
  search: string
  onPick: (p: Picked) => void
}) => {
  switch (actorType) {
    case ActorType.USER:
      return <UserResults search={search} onPick={onPick} />
    case ActorType.API_TOKEN:
      return <ApiTokenResults search={search} onPick={onPick} />
    case ActorType.CUSTOMER:
      return <CustomerResults search={search} onPick={onPick} />
    default:
      return null
  }
}

const ItemList = ({
  isLoading,
  items,
  onPick,
}: {
  isLoading: boolean
  items: Picked[]
  onPick: (p: Picked) => void
}) => {
  if (isLoading) {
    return (
      <div className="space-y-1 py-1">
        <Skeleton height={12} width={200} />
        <Skeleton height={12} width={160} />
        <Skeleton height={12} width={180} />
      </div>
    )
  }
  if (items.length === 0) {
    return <p className="text-xs text-muted-foreground py-2 text-center">No matches</p>
  }
  return (
    <ul className="max-h-60 overflow-y-auto">
      {items.map(it => (
        <li key={it.id}>
          <button
            type="button"
            className="w-full text-left px-2 py-1.5 text-sm rounded hover:bg-accent"
            onClick={() => onPick(it)}
          >
            {it.label}
          </button>
        </li>
      ))}
    </ul>
  )
}

// User/ApiToken pass the raw UUID; the backend accepts either form.

const UserResults = ({ search, onPick }: { search: string; onPick: (p: Picked) => void }) => {
  const q = useQuery(listUsers)
  const users = q.data?.users ?? []
  const term = search.toLowerCase()
  const items: Picked[] = users
    .filter(u => {
      if (!term) return true
      const name = displayName(u.firstName, u.lastName, u.email).toLowerCase()
      return name.includes(term) || u.email.toLowerCase().includes(term)
    })
    .map(u => {
      const name = displayName(u.firstName, u.lastName, u.email)
      return { id: u.id, label: name === u.email ? u.email : `${name} · ${u.email}` }
    })
  return <ItemList isLoading={q.isLoading} items={items} onPick={onPick} />
}

const ApiTokenResults = ({ search, onPick }: { search: string; onPick: (p: Picked) => void }) => {
  const q = useQuery(listApiTokens)
  const tokens = q.data?.apiTokens ?? []
  const term = search.toLowerCase()
  const items: Picked[] = tokens
    .filter(t => !term || t.name.toLowerCase().includes(term) || t.hint.toLowerCase().includes(term))
    .map(t => ({ id: t.id, label: `${t.name} (${t.hint})` }))
  return <ItemList isLoading={q.isLoading} items={items} onPick={onPick} />
}

const CustomerResults = ({ search, onPick }: { search: string; onPick: (p: Picked) => void }) => {
  const q = useQuery(listCustomers, {
    pagination: { perPage: 20, page: 0 },
    search: search || undefined,
  })
  const items: Picked[] = (q.data?.customers ?? []).map(c => ({ id: c.id, label: c.name }))
  return <ItemList isLoading={q.isLoading} items={items} onPick={onPick} />
}

function displayName(first: string | undefined, last: string | undefined, email: string): string {
  const joined = `${first ?? ''} ${last ?? ''}`.trim()
  return joined || email
}
