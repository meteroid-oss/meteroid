import type { Transport } from '@connectrpc/connect'

// Holder for the single shared connect transport. It is built and registered
// once by `App` (where the interceptors live), and read here by router loaders
// — which run outside React and so have no access to `TransportProvider`.
//
// This module deliberately imports nothing from the app: the transport is built
// with interceptors that reference the router, and loaders live in the router
// tree, so a holder keeps the data layer free of that import cycle.
let instance: Transport | undefined

export const setTransport = (transport: Transport) => {
  instance = transport
}

export const getTransport = (): Transport => {
  if (!instance) {
    throw new Error('Transport accessed before it was initialized in App')
  }
  return instance
}
