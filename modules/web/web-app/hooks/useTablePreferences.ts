import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { OnChangeFn, PaginationState, VisibilityState } from '@tanstack/react-table'
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { useSearchParams } from 'react-router-dom'

import { useQuery } from '@/lib/connectrpc'
import { me, updateUserPreferences } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export type TableDensity = 'comfortable' | 'compact'

interface StoredPrefs {
  columnVisibility?: VisibilityState
  /** Whether the list's metric chart is shown (only meaningful on lists that have one). */
  showChart?: boolean
}

interface UseTablePreferencesOptions {
  defaultHidden?: string[]
}

// Table prefs live under a namespaced top-level key in the shared per-user
// preferences bag — persisted via UpdateUserPreferences (shallow-merged per
// key, so tables never clobber each other) and read back from `me`.
// localStorage mirrors the value for instant first paint / offline.
const bagKey = (tableKey: string) => `table:${tableKey}`
const storageKey = (tableKey: string) => `meteroid.tableprefs.v1.${tableKey}`

const read = (tableKey: string): StoredPrefs => {
  if (typeof window === 'undefined') return {}
  try {
    const raw = window.localStorage.getItem(storageKey(tableKey))
    return raw ? (JSON.parse(raw) as StoredPrefs) : {}
  } catch {
    return {}
  }
}

const write = (tableKey: string, prefs: StoredPrefs) => {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(storageKey(tableKey), JSON.stringify(prefs))
  } catch {
    // ignore quota / serialization errors — preferences are non-critical
  }
}

export const hiddenState = (ids: string[]): VisibilityState =>
  ids.reduce<VisibilityState>((acc, id) => ({ ...acc, [id]: false }), {})

// One shared store per table key, so every hook instance for a table (e.g. the table itself and
// the page's saved-view controller) sees the same prefs and writes never clobber each other.
interface PrefsStore {
  prefs: StoredPrefs
  listeners: Set<() => void>
  serverApplied: boolean
}

const stores = new Map<string, PrefsStore>()

const getStore = (tableKey: string): PrefsStore => {
  let store = stores.get(tableKey)
  if (!store) {
    store = { prefs: read(tableKey), listeners: new Set(), serverApplied: false }
    stores.set(tableKey, store)
  }
  return store
}

const setStorePrefs = (tableKey: string, next: StoredPrefs) => {
  const store = getStore(tableKey)
  store.prefs = next
  write(tableKey, next)
  store.listeners.forEach(l => l())
}

/**
 * Per-table column visibility, persisted per user. Pass a stable `tableKey`
 * unique to each list (e.g. "invoices"). `defaultHidden` columns are hidden
 * until the user explicitly shows them. Row height is a separate, global
 * preference — see {@link useTableDensity}.
 */
export const useTablePreferences = (
  tableKey: string,
  { defaultHidden = [] }: UseTablePreferencesOptions = {}
) => {
  const enabled = tableKey !== ''
  const queryClient = useQueryClient()
  const store = getStore(tableKey)

  const subscribe = useCallback(
    (listener: () => void) => {
      store.listeners.add(listener)
      return () => {
        store.listeners.delete(listener)
      }
    },
    [store]
  )
  const prefs = useSyncExternalStore(subscribe, () => store.prefs)

  const defaultHiddenKey = defaultHidden.join(',')
  const columnVisibility = useMemo(
    () => ({ ...hiddenState(defaultHidden), ...(prefs.columnVisibility ?? {}) }),
    // defaultHidden identity is not stable across renders; its content is
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [prefs.columnVisibility, defaultHiddenKey]
  )

  const meQuery = useQuery(me, undefined, { enabled })
  const updatePreferences = useMutation(updateUserPreferences, {
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: me, cardinality: 'finite' }),
      })
    },
  })

  // Adopt the server copy once per table (e.g. set on another device), then let
  // local state drive — every change writes through to localStorage and server.
  useEffect(() => {
    if (store.serverApplied || !meQuery.data) return
    store.serverApplied = true
    const raw = meQuery.data.user?.preferences
    if (!raw) return
    try {
      const bag = JSON.parse(raw) as Record<string, StoredPrefs | undefined>
      const remote = bag[bagKey(tableKey)]
      if (!remote) return
      setStorePrefs(tableKey, {
        columnVisibility: remote.columnVisibility ?? {},
        showChart: remote.showChart ?? store.prefs.showChart ?? true,
      })
    } catch {
      // ignore malformed preferences bag
    }
  }, [meQuery.data, tableKey, store])

  const persist = useCallback(
    (next: StoredPrefs) => {
      setStorePrefs(tableKey, next)
      if (enabled) {
        updatePreferences.mutate({ preferences: JSON.stringify({ [bagKey(tableKey)]: next }) })
      }
    },
    // mutation identity is stable enough; keep deps minimal to avoid churn
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tableKey, enabled]
  )

  const setColumnVisibility: OnChangeFn<VisibilityState> = useCallback(
    updater => {
      const current = { ...hiddenState(defaultHidden), ...(store.prefs.columnVisibility ?? {}) }
      const next = typeof updater === 'function' ? updater(current) : updater
      persist({ ...store.prefs, columnVisibility: next })
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [persist, store, defaultHiddenKey]
  )

  const setShowChart = useCallback(
    (showChart: boolean) => persist({ ...store.prefs, showChart }),
    [persist, store]
  )

  const reset = useCallback(
    () => persist({ columnVisibility: hiddenState(defaultHidden), showChart: true }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [persist, defaultHiddenKey]
  )

  return {
    columnVisibility,
    setColumnVisibility,
    showChart: prefs.showChart ?? true,
    setShowChart,
    reset,
  }
}

// Older entries were stored as bare strings rather than JSON.
const parseStored = (raw: string): unknown => {
  try {
    return JSON.parse(raw)
  } catch {
    return raw
  }
}

// Global (not per-table) preferences: one top-level key each in the shared per-user
// bag, mirrored to localStorage for instant paint.
const useGlobalTablePref = <T>(
  bagKey: string,
  storageKey: string,
  parse: (raw: unknown) => T | undefined,
  fallback: T
) => {
  const queryClient = useQueryClient()
  const [value, setValueState] = useState<T>(() => {
    if (typeof window === 'undefined') return fallback
    try {
      const raw = window.localStorage.getItem(storageKey)
      return raw === null ? fallback : (parse(parseStored(raw)) ?? fallback)
    } catch {
      return fallback
    }
  })

  const writeLocal = (next: T) => {
    if (typeof window === 'undefined') return
    try {
      window.localStorage.setItem(storageKey, JSON.stringify(next))
    } catch {
      // ignore quota errors — preference is non-critical
    }
  }

  const meQuery = useQuery(me)
  const updatePreferences = useMutation(updateUserPreferences, {
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: me, cardinality: 'finite' }),
      })
    },
  })

  const serverApplied = useRef(false)
  useEffect(() => {
    if (serverApplied.current || !meQuery.data) return
    serverApplied.current = true
    const raw = meQuery.data.user?.preferences
    if (!raw) return
    try {
      const remote = parse((JSON.parse(raw) as Record<string, unknown>)[bagKey])
      if (remote !== undefined) {
        setValueState(remote)
        writeLocal(remote)
      }
    } catch {
      // ignore malformed preferences bag
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [meQuery.data])

  const setValue = useCallback(
    (next: T) => {
      setValueState(next)
      writeLocal(next)
      updatePreferences.mutate({ preferences: JSON.stringify({ [bagKey]: next }) })
    },
    // mutation identity is stable enough; keep deps minimal to avoid churn
    // eslint-disable-next-line react-hooks/exhaustive-deps
    []
  )

  return [value, setValue] as const
}

const parseDensity = (raw: unknown): TableDensity | undefined =>
  raw === 'compact' || raw === 'comfortable' ? raw : undefined

/**
 * Global row-height (density) preference shared by every table, persisted per
 * user. Changing it in one table's view-options menu affects all tables.
 */
export const useTableDensity = () => {
  const [density, setDensity] = useGlobalTablePref(
    'tableDensity',
    'meteroid.tabledensity.v1',
    parseDensity,
    'compact'
  )
  return { density, setDensity }
}

export const PAGE_SIZE_OPTIONS = [20, 50, 100, 200]
const DEFAULT_PAGE_SIZE = PAGE_SIZE_OPTIONS[0]

const parsePageSize = (raw: unknown): number | undefined =>
  typeof raw === 'number' && PAGE_SIZE_OPTIONS.includes(raw) ? raw : undefined

const PAGE_PARAM = 'page'

const parsePage = (raw: string | null) => {
  const page = Number(raw)
  return Number.isInteger(page) && page > 1 ? page - 1 : 0
}

/**
 * Pagination state for list screens. The page size is a global per-user
 * preference; the current page lives in the URL (`?page=3`) so Back returns to it.
 */
export const useListPagination = () => {
  const [pageSize, setPageSize] = useGlobalTablePref(
    'tablePageSize',
    'meteroid.tablepagesize.v1',
    parsePageSize,
    DEFAULT_PAGE_SIZE
  )
  const [searchParams, setSearchParams] = useSearchParams()
  const pageIndex = parsePage(searchParams.get(PAGE_PARAM))
  const pagination = useMemo(() => ({ pageIndex, pageSize }), [pageIndex, pageSize])
  const latest = useRef(pagination)
  latest.current = pagination

  const setPageIndex = useCallback(
    (index: number) =>
      setSearchParams(
        prev => {
          const next = new URLSearchParams(prev)
          if (index > 0) next.set(PAGE_PARAM, String(index + 1))
          else next.delete(PAGE_PARAM)
          return next
        },
        { replace: true }
      ),
    [setSearchParams]
  )

  const setPagination: OnChangeFn<PaginationState> = useCallback(
    updater => {
      const current = latest.current
      const next = typeof updater === 'function' ? updater(current) : updater
      if (next.pageSize !== current.pageSize) {
        setPageSize(next.pageSize)
        setPageIndex(0)
      } else if (next.pageIndex !== current.pageIndex) {
        setPageIndex(next.pageIndex)
      }
    },
    [setPageSize, setPageIndex]
  )

  return [pagination, setPagination] as const
}

/** Back to the first page when `deps` change, but not on mount, so a `?page=` link survives. */
export const useResetPageOnChange = (
  setPagination: OnChangeFn<PaginationState>,
  deps: unknown[]
) => {
  const key = JSON.stringify(deps)
  const previous = useRef(key)
  useEffect(() => {
    if (previous.current === key) return
    previous.current = key
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [key, setPagination])
}
