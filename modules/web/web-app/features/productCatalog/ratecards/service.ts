import { createConnectQueryKey } from '@connectrpc/connect-query'

import { useMutation, useQuery } from '@/lib/connectrpc'
import {
  getRateCard,
  listRateCardChanges,
  setRateCardRates,
} from '@/rpc/api/ratecards/v1/ratecards-RateCardsService_connectquery'

import type { RateCard, RateCardEntry } from '@/rpc/api/ratecards/v1/models_pb'
import type {
  RateCardChange,
  RateCardKeyRemovalSchema,
  RateCardRateInputSchema,
} from '@/rpc/api/ratecards/v1/ratecards_pb'
import type { MessageInitShape } from '@bufbuild/protobuf'
import type { ConnectError } from '@connectrpc/connect'
import type { QueryKey } from '@tanstack/react-query'

export type RateCardRead = { card?: RateCard; rates: RateCardEntry[]; entries: RateCardEntry[] }

export type RatesBatch = {
  rateCardId: string
  effectiveFrom?: string
  rates: MessageInitShape<typeof RateCardRateInputSchema>[]
  remove: MessageInitShape<typeof RateCardKeyRemovalSchema>[]
}

export type RatesSaved = { inserted: number; removed: number; replaced: number; unchanged: number }

export type ChangesPage = { changes: RateCardChange[]; nextCursor?: string }

/**
 * The reads and writes a card's sheet makes. Core's rate cards by default; a module serving its
 * own cards (the Enterprise cost cards) passes its RPCs.
 */
export type RateCardService = {
  useCard: (id: string) => { data?: RateCardRead; isLoading: boolean; refetch: () => unknown }
  useSetRates: (options: {
    onSuccess: (saved: RatesSaved) => void
    onError: (error: ConnectError) => void
  }) => { mutate: (batch: RatesBatch) => void; isPending: boolean }
  useChanges: (input: { rateCardId: string; cursor?: string; limit: number }) => {
    data?: ChangesPage
    isLoading: boolean
    isFetching: boolean
  }
  changesKey: QueryKey
}

export const coreRateCards: RateCardService = {
  useCard: id => useQuery(getRateCard, { id }),
  useSetRates: options => useMutation(setRateCardRates, options),
  useChanges: input => useQuery(listRateCardChanges, input),
  changesKey: createConnectQueryKey({ schema: listRateCardChanges, cardinality: undefined }),
}
