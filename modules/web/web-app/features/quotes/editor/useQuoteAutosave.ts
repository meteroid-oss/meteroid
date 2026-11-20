import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useRef, useState } from 'react'

import {
  DRAFT_PAYLOAD_VERSION,
  decodeSnapshot,
  encodeSnapshot,
} from '@/features/quotes/editor/draftPayload'
import {
  localDraftKey,
  readLocalDraft,
  removeLocalDraft,
  useLocalDraftScope,
  writeLocalDraft,
} from '@/features/quotes/editor/localDraft'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteQuoteDraft,
  getQuoteDraft,
  listQuoteDrafts,
  saveQuoteDraft,
} from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { parseUtc } from '@/utils/date'

import type { LocalDraft } from '@/features/quotes/editor/localDraft'
import type { QuoteFormSeed } from '@/features/quotes/quoteFormSeed'
import type { QuoteDraft } from '@/rpc/api/quotes/v1/models_pb'

export type AutosaveStatus = 'idle' | 'saving' | 'saved' | 'local' | 'conflict'

export interface RestoreOffer {
  snapshot: QuoteFormSeed
  savedAt: Date
  source: 'server' | 'device'
}

interface UseQuoteAutosaveOptions {
  /** The quote being edited: its one shared draft is found by it. */
  quoteId?: string
  /** The server draft a new quote was resumed from. */
  initialDraft?: QuoteDraft
  getSnapshot: () => QuoteFormSeed
  getLabel: () => string
  onDraftCreated?: (draftId: string) => void
  /** Saves nothing and offers nothing, e.g. while editing a template. */
  disabled?: boolean
}

const DEBOUNCE_MS = 1500

/** A device copy the server never received, newer than the server's, else the server draft when offered. */
function pickOffer(
  server: QuoteDraft | undefined,
  local: LocalDraft | undefined,
  offerServer: boolean
): RestoreOffer | undefined {
  const serverAt = server ? parseUtc(server.updatedAt).getTime() : 0
  if (local && !local.synced && local.savedAt > serverAt) {
    const snapshot = decodeSnapshot(local.payload)
    if (snapshot) return { snapshot, savedAt: new Date(local.savedAt), source: 'device' }
  }
  if (offerServer && server) {
    const snapshot = decodeSnapshot(server.payload)
    if (snapshot) return { snapshot, savedAt: parseUtc(server.updatedAt), source: 'server' }
  }
  return undefined
}

export function useQuoteAutosave({
  quoteId,
  initialDraft,
  getSnapshot,
  getLabel,
  onDraftCreated,
  disabled = false,
}: UseQuoteAutosaveOptions) {
  const scope = useLocalDraftScope()
  const queryClient = useQueryClient()
  const { mutateAsync: save } = useMutation(saveQuoteDraft)
  const { mutateAsync: remove } = useMutation(deleteQuoteDraft)

  const callbacks = useRef({ getSnapshot, getLabel, onDraftCreated })
  callbacks.current = { getSnapshot, getLabel, onDraftCreated }

  const draft = useRef<{ id?: string; version?: number }>({
    id: initialDraft?.id,
    version: initialDraft?.version,
  })
  const ready = useRef(!quoteId)
  const lastPayload = useRef<string>()
  const timer = useRef<ReturnType<typeof setTimeout>>()
  const inFlight = useRef<Promise<void>>()
  const again = useRef(false)
  const done = useRef(disabled)
  const failed = useRef(false)
  const pendingConflict = useRef<QuoteDraft>()

  const [status, setStatus] = useState<AutosaveStatus>('idle')
  const [conflict, setConflict] = useState<QuoteDraft>()
  const [offer, setOffer] = useState<RestoreOffer>()

  const localKey = useCallback(
    () =>
      localDraftKey(
        scope,
        quoteId ? { quoteId } : draft.current.id ? { draftId: draft.current.id } : 'new'
      ),
    [scope, quoteId]
  )

  const invalidateList = useCallback(
    () =>
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listQuoteDrafts, cardinality: undefined }),
      }),
    [queryClient]
  )

  const mirrorLocally = useCallback(() => {
    const payload = encodeSnapshot(callbacks.current.getSnapshot())
    if (payload !== lastPayload.current) {
      writeLocalDraft(localKey(), { payload, savedAt: Date.now(), synced: false })
    }
  }, [localKey])

  const flush = useCallback(async (): Promise<void> => {
    clearTimeout(timer.current)
    timer.current = undefined
    if (done.current) return
    if (!ready.current) {
      timer.current = setTimeout(() => void flush(), DEBOUNCE_MS)
      return
    }
    if (inFlight.current) {
      again.current = true
      return
    }

    const payload = encodeSnapshot(callbacks.current.getSnapshot())
    if (payload === lastPayload.current) return
    const savedAt = Date.now()
    writeLocalDraft(localKey(), { payload, savedAt, synced: false })
    if (pendingConflict.current) return

    const run = async () => {
      setStatus('saving')
      try {
        const response = await save({
          id: draft.current.id,
          quoteId,
          expectedVersion: draft.current.version,
          label: callbacks.current.getLabel(),
          payload,
          payloadVersion: DRAFT_PAYLOAD_VERSION,
        })
        failed.current = false
        if (response.conflict && response.draft) {
          pendingConflict.current = response.draft
          setConflict(response.draft)
          setStatus('conflict')
          return
        }
        if (!response.draft) return
        const created = !draft.current.id
        draft.current = { id: response.draft.id, version: response.draft.version }
        lastPayload.current = payload
        if (created && !quoteId && !done.current) {
          removeLocalDraft(localDraftKey(scope, 'new'))
          callbacks.current.onDraftCreated?.(response.draft.id)
          void invalidateList()
        }
        writeLocalDraft(localKey(), { payload, savedAt, synced: true })
        setStatus('saved')
      } catch {
        failed.current = true
        setStatus('local')
      }
    }

    inFlight.current = run().finally(() => {
      inFlight.current = undefined
      if (again.current) {
        again.current = false
        void flush()
      }
    })
    return inFlight.current
  }, [localKey, save, quoteId, scope, invalidateList])

  const touch = useCallback(() => {
    if (done.current) return
    clearTimeout(timer.current)
    timer.current = setTimeout(() => void flush(), DEBOUNCE_MS)
  }, [flush])

  useEffect(() => {
    lastPayload.current = encodeSnapshot(callbacks.current.getSnapshot())
  }, [])

  const existingQuery = useQuery(
    getQuoteDraft,
    quoteId ? { target: { case: 'quoteId', value: quoteId } } : skipToken,
    { staleTime: 0, refetchOnWindowFocus: false }
  )

  useEffect(() => {
    if (!quoteId || ready.current || existingQuery.isPending) return
    const server = existingQuery.data?.draft
    if (server) draft.current = { id: server.id, version: server.version }
    ready.current = true
    setOffer(pickOffer(server, readLocalDraft(localKey()), true))
  }, [quoteId, existingQuery.isPending, existingQuery.data, localKey])

  useEffect(() => {
    if (quoteId || disabled) return
    setOffer(pickOffer(initialDraft, readLocalDraft(localKey()), false))
    // Once, for the draft the editor opened with.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const onOnline = () => {
      if (failed.current) void flush()
    }
    const onLeave = () => {
      if (timer.current) mirrorLocally()
    }
    window.addEventListener('online', onOnline)
    window.addEventListener('pagehide', onLeave)
    window.addEventListener('beforeunload', onLeave)
    return () => {
      window.removeEventListener('online', onOnline)
      window.removeEventListener('pagehide', onLeave)
      window.removeEventListener('beforeunload', onLeave)
      if (timer.current) void flush()
    }
  }, [flush, mirrorLocally])

  const resolveConflict = (draftToFollow: QuoteDraft) => {
    draft.current = { id: draftToFollow.id, version: draftToFollow.version }
    pendingConflict.current = undefined
    lastPayload.current = undefined
    setConflict(undefined)
  }

  /** Overwrites the other version with this editor's. */
  const keepMine = () => {
    if (!pendingConflict.current) return
    resolveConflict(pendingConflict.current)
    void flush()
  }

  /** The other version, for the editor to load; later saves build on it. */
  const takeTheirs = (): QuoteFormSeed | undefined => {
    const theirs = pendingConflict.current
    if (!theirs) return undefined
    resolveConflict(theirs)
    setStatus('saved')
    return decodeSnapshot(theirs.payload)
  }

  const acceptOffer = (): QuoteFormSeed | undefined => {
    const snapshot = offer?.snapshot
    setOffer(undefined)
    return snapshot
  }

  const discardOffer = async () => {
    setOffer(undefined)
    removeLocalDraft(localKey())
    if (quoteId && draft.current.id) {
      const id = draft.current.id
      draft.current = {}
      lastPayload.current = undefined
      await remove({ id }).catch(() => undefined)
    }
  }

  /** The quote was saved: its draft is no longer needed. */
  const complete = async () => {
    done.current = true
    clearTimeout(timer.current)
    timer.current = undefined
    await inFlight.current
    removeLocalDraft(localKey())
    removeLocalDraft(localDraftKey(scope, 'new'))
    if (draft.current.id) {
      await remove({ id: draft.current.id }).catch(() => undefined)
      void invalidateList()
    }
  }

  return {
    status,
    conflict,
    offer,
    touch,
    keepMine,
    takeTheirs,
    acceptOffer,
    discardOffer,
    complete,
  }
}

export type QuoteAutosave = ReturnType<typeof useQuoteAutosave>
