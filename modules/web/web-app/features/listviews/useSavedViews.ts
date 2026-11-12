import { create } from '@bufbuild/protobuf'
import { OnChangeFn, SortingState, VisibilityState } from '@tanstack/react-table'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { toast } from 'sonner'

import { ColumnState } from '@/components/table/StandardTable'
import { hiddenState, useTablePreferences } from '@/hooks/useTablePreferences'
import {
  EntityListType,
  EntityListView,
  EntityListViewSchema,
} from '@/rpc/api/listviews/v1/models_pb'

import { AppFilter } from '../filters/filterTypes'
import { FILTERS_PARAM, FILTERS_SERDE } from '../filters/useEntityFilters'

import { useListViews } from './useListViews'
import {
  EMPTY_VIEW_CONFIG,
  ViewConfig,
  parseViewConfig,
  sameViewState,
  serializeViewConfig,
} from './viewConfig'

/** URL parameter holding the active view id; `all` = explicitly no view (skips the default). */
export const VIEW_PARAM = 'view'
const ALL_VIEWS = 'all'

/**
 * A view every member has, defined by the page: it can be selected and pinned as one's default,
 * not renamed, shared or deleted. Its id is reserved (`builtin:…`) so no saved view collides.
 */
export interface ViewPreset {
  id: `builtin:${string}`
  name: string
  config: ViewConfig
}

// A pinned preset is a per-browser preference: the server default only points at saved views.
const presetDefaultKey = (listType: EntityListType) => `listviews:default-preset:${listType}`

const readPresetDefault = (listType: EntityListType): string | undefined => {
  try {
    return localStorage.getItem(presetDefaultKey(listType)) ?? undefined
  } catch {
    return undefined
  }
}

const writePresetDefault = (listType: EntityListType, id: string | undefined) => {
  try {
    if (id) localStorage.setItem(presetDefaultKey(listType), id)
    else localStorage.removeItem(presetDefaultKey(listType))
  } catch {
    // Storage unavailable: the preference just doesn't persist.
  }
}

// Whether the list had a saved default view when last loaded, so landing on it can skip waiting
// for the views when there's none. '1' | '0'; absent = unknown.
const hasDefaultKey = (listType: EntityListType) => `listviews:has-default:${listType}`

const readHasDefault = (listType: EntityListType): boolean | undefined => {
  try {
    const value = localStorage.getItem(hasDefaultKey(listType))
    return value === null ? undefined : value === '1'
  } catch {
    return undefined
  }
}

const writeHasDefault = (listType: EntityListType, hasDefault: boolean) => {
  try {
    localStorage.setItem(hasDefaultKey(listType), hasDefault ? '1' : '0')
  } catch {
    // Storage unavailable: landings keep waiting for the views.
  }
}

export interface SavedViewsOptions {
  listType: EntityListType
  /** URL parameters holding the page's quick filters (captured raw into a view). */
  quickKeys: string[]
  /** Current advanced filters (URL-synced under {@link FILTERS_PARAM}). */
  filters: AppFilter[]
  sorting: SortingState
  setSorting: (sorting: SortingState) => void
  defaultSorting: SortingState
  /** The list table's `tableKey` and its default-hidden columns. */
  tableKey: string
  defaultHidden: string[]
  /**
   * Former quick-filter URL params that are now filter fields: converted to filters when found in
   * the URL or in a saved view's `quick`, so old links and views keep working.
   */
  legacyQuick?: Record<string, (raw: string) => AppFilter>
  /** Built-in views, listed right after "All". */
  presets?: ViewPreset[]
}

export type SavedViews = ReturnType<typeof useSavedViews>

/**
 * Saved-view controller for a list page. The page keeps owning its state (URL params + sort); this
 * hook applies a view onto that state in one URL write, tracks whether the current state still
 * matches the active view, and owns column visibility while a view is active (draft until saved).
 */
export function useSavedViews({
  listType,
  quickKeys,
  filters,
  sorting,
  setSorting,
  defaultSorting,
  tableKey,
  defaultHidden,
  legacyQuick,
  presets,
}: SavedViewsOptions) {
  const api = useListViews(listType)
  const presetViews = useMemo(
    () =>
      (presets ?? []).map(p =>
        create(EntityListViewSchema, {
          id: p.id,
          name: p.name,
          entityType: listType,
          config: serializeViewConfig(p.config),
          shared: true,
        })
      ),
    [presets, listType]
  )
  const views = useMemo(() => [...presetViews, ...api.views], [presetViews, api.views])
  const isPreset = useCallback(
    (view: EntityListView) => presetViews.some(p => p.id === view.id),
    [presetViews]
  )
  const [presetDefault, setPresetDefault] = useState(() => readPresetDefault(listType))
  // A saved default wins over a pinned preset; an unknown preset id is ignored.
  const defaultViewId = api.defaultViewId ?? presetViews.find(p => p.id === presetDefault)?.id
  const [searchParams, setSearchParams] = useSearchParams()

  // Stored configs may predate a quick filter becoming a filter field: read them as filters.
  const normalize = useCallback(
    (config: ViewConfig): ViewConfig => {
      if (!legacyQuick) return config
      const quick = { ...config.quick }
      const converted: AppFilter[] = []
      for (const [key, toFilter] of Object.entries(legacyQuick)) {
        if (quick[key]) converted.push(toFilter(quick[key]))
        delete quick[key]
      }
      return converted.length
        ? { ...config, quick, filters: [...config.filters, ...converted] }
        : config
    },
    [legacyQuick]
  )

  const viewParam = searchParams.get(VIEW_PARAM)
  const activeView = views.find(v => v.id === viewParam)
  const activeConfig = useMemo(
    () => (activeView ? normalize(parseViewConfig(activeView.config)) : undefined),
    [activeView, normalize]
  )

  // Columns: the user's own preferences, unless a view is active — then the view's columns, with
  // unsaved toggles kept as a draft (so browsing a shared view never rewrites your preferences).
  const prefs = useTablePreferences(tableKey, { defaultHidden })
  const [draftColumns, setDraftColumns] = useState<VisibilityState | undefined>()
  useEffect(() => {
    setDraftColumns(undefined)
  }, [viewParam])

  const defaultHiddenKey = defaultHidden.join(',')
  const columnVisibility = useMemo<VisibilityState>(() => {
    if (!activeView) return prefs.columnVisibility
    return {
      ...hiddenState(defaultHidden),
      ...(draftColumns ?? activeConfig?.columns ?? prefs.columnVisibility),
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeView, draftColumns, activeConfig, prefs.columnVisibility, defaultHiddenKey])

  const onColumnsChange: OnChangeFn<VisibilityState> = useCallback(
    updater => {
      if (!activeView) return prefs.setColumnVisibility(updater)
      setDraftColumns(typeof updater === 'function' ? updater(columnVisibility) : updater)
    },
    [activeView, prefs, columnVisibility]
  )

  const columnState: ColumnState = useMemo(
    () => ({
      visibility: columnVisibility,
      onChange: onColumnsChange,
      onReset: () => (activeView ? setDraftColumns(hiddenState(defaultHidden)) : prefs.reset()),
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [columnVisibility, onColumnsChange, activeView, prefs.reset, defaultHiddenKey]
  )

  const quickKeysKey = quickKeys.join(',')
  const current: ViewConfig = useMemo(() => {
    const quick: Record<string, string> = {}
    for (const key of quickKeys) {
      const value = searchParams.get(key)
      if (value) quick[key] = value
    }
    return { v: 1, filters, quick, sort: sorting, columns: columnVisibility }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams, quickKeysKey, filters, sorting, columnVisibility])

  const compareOpts = { defaultSorting, defaultHidden }
  const isDirty = !!activeConfig && !sameViewState(current, activeConfig, compareOpts)
  /** "All" with anything narrowed or re-sorted: worth offering to save as a view. */
  const isCustomized = !activeView && !sameViewState(current, EMPTY_VIEW_CONFIG, compareOpts)

  /** Write a config onto the page state: every URL param in one update, then sort. */
  const applyConfig = useCallback(
    (config: ViewConfig, viewValue: string | null, replace = false) => {
      const { filters } = normalize(config)
      setSearchParams(
        prev => {
          const next = new URLSearchParams(prev)
          for (const key of [...quickKeys, ...Object.keys(legacyQuick ?? {})]) next.delete(key)
          for (const [key, value] of Object.entries(config.quick)) {
            if (quickKeys.includes(key) && value) next.set(key, value)
          }
          const serializedFilters = FILTERS_SERDE.serialize(filters)
          if (serializedFilters) next.set(FILTERS_PARAM, serializedFilters)
          else next.delete(FILTERS_PARAM)
          if (viewValue) next.set(VIEW_PARAM, viewValue)
          else next.delete(VIEW_PARAM)
          return next
        },
        { replace }
      )
      setSorting(config.sort.length ? config.sort : defaultSorting)
      setDraftColumns(undefined)
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [setSearchParams, quickKeysKey, setSorting, defaultSorting, normalize]
  )

  const selectView = useCallback(
    (view: EntityListView | null) => {
      if (view) applyConfig(parseViewConfig(view.config), view.id)
      // "All": a sentinel only matters when a default would otherwise re-apply on reload.
      else applyConfig(EMPTY_VIEW_CONFIG, defaultViewId ? ALL_VIEWS : null)
    },
    [applyConfig, defaultViewId]
  )

  // On landing, resolve which view to open before the list first queries (`ready` gates it, so
  // there's no flash of unfiltered rows):
  // - a bare `?view=<id>` link (no other list state) opens that view's saved state;
  // - the bare list opens the user's default view, if any;
  // - anything else (filters in the URL, `?view=all`) is left as the URL says.
  const [bootstrapped, setBootstrapped] = useState(false)
  const bootstrapping = useRef(false)
  // Landing that can't open a view: the list may query right away, without the views. A stale
  // "no default" only costs one re-query when the default turns up and gets applied below.
  const [landsWithoutView] = useState(() => {
    const urlHasState = [...searchParams.keys()].some(k => k !== VIEW_PARAM && k !== 'page')
    if (urlHasState || viewParam === ALL_VIEWS) return true
    if (viewParam) return false
    return !presetDefault && readHasDefault(listType) === false
  })
  useEffect(() => {
    if (api.query.isSuccess) writeHasDefault(listType, !!api.defaultViewId)
  }, [api.query.isSuccess, api.defaultViewId, listType])
  useEffect(() => {
    if (bootstrapped || bootstrapping.current) return
    if (api.query.isError) return setBootstrapped(true)
    if (!api.query.isSuccess) return
    bootstrapping.current = true
    const otherParams = [...searchParams.keys()].filter(k => k !== VIEW_PARAM && k !== 'page')
    if (otherParams.length === 0) {
      const target = viewParam
        ? views.find(v => v.id === viewParam)
        : views.find(v => v.id === defaultViewId)
      if (target) applyConfig(parseViewConfig(target.config), target.id, true)
    }
    setBootstrapped(true)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api.query.isSuccess, api.query.isError])

  // Legacy quick-filter params in the URL (old links) become filters, once.
  useEffect(() => {
    if (!legacyQuick) return
    const found = Object.keys(legacyQuick).filter(key => searchParams.get(key))
    if (found.length === 0) return
    setSearchParams(
      prev => {
        const next = new URLSearchParams(prev)
        const converted = found.map(key => legacyQuick[key](next.get(key) as string))
        found.forEach(key => next.delete(key))
        const merged = [...FILTERS_SERDE.deserialize(next.get(FILTERS_PARAM) ?? ''), ...converted]
        next.set(FILTERS_PARAM, FILTERS_SERDE.serialize(merged))
        return next
      },
      { replace: true }
    )
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams])

  // A `?view=` that doesn't resolve (deleted, made private, bad link) falls back to "All" once
  // views have loaded, keeping whatever state the URL carries.
  useEffect(() => {
    if (!api.query.isSuccess || !viewParam || viewParam === ALL_VIEWS || activeView) return
    setSearchParams(
      prev => {
        const next = new URLSearchParams(prev)
        next.delete(VIEW_PARAM)
        return next
      },
      { replace: true }
    )
    toast.info('That view is no longer available')
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api.query.isSuccess, viewParam, activeView])

  const snapshot = (): ViewConfig => ({ ...current, columns: columnVisibility })

  const createFromCurrent = (name: string, shared: boolean) =>
    api.create
      .mutateAsync({
        entityType: listType,
        name,
        shared,
        config: serializeViewConfig(snapshot()),
      })
      .then(res => {
        if (res.view) {
          setSearchParams(prev => {
            const next = new URLSearchParams(prev)
            next.set(VIEW_PARAM, res.view!.id)
            return next
          })
        }
        return res.view
      })

  const saveActive = () => {
    if (!activeView || isPreset(activeView)) return Promise.resolve()
    return api.update
      .mutateAsync({ id: activeView.id, config: serializeViewConfig(snapshot()) })
      .then(() => setDraftColumns(undefined))
  }

  const revert = () => {
    if (activeView) selectView(activeView)
  }

  const rename = (view: EntityListView, name: string) =>
    api.update.mutateAsync({ id: view.id, name })

  const setShared = (view: EntityListView, shared: boolean) =>
    api.update.mutateAsync({ id: view.id, shared })

  const duplicate = (view: EntityListView) =>
    api.duplicate.mutateAsync({ id: view.id }).then(res => {
      if (res.view) selectView(res.view)
      return res.view
    })

  const remove = (view: EntityListView) =>
    api.remove.mutateAsync({ id: view.id }).then(() => {
      if (view.id === activeView?.id) selectView(null)
    })

  const setDefault = async (view: EntityListView | null) => {
    const preset = view && isPreset(view) ? view.id : undefined
    writePresetDefault(listType, preset)
    setPresetDefault(preset)
    if (preset && !api.defaultViewId) return
    await api.setDefault.mutateAsync({
      entityType: listType,
      viewId: preset ? undefined : view?.id,
    })
  }

  return {
    views,
    defaultViewId,
    isPreset,
    activeView,
    isDirty,
    isCustomized,
    isLoading: api.query.isLoading,
    /** False until the default view has been resolved (gate the list query on it). */
    ready: bootstrapped || landsWithoutView,
    isSaving: api.create.isPending || api.update.isPending,
    columnState,
    selectView,
    createFromCurrent,
    saveActive,
    revert,
    rename,
    setShared,
    duplicate,
    remove,
    setDefault,
  }
}
