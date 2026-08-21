import { loadStripe } from '@stripe/stripe-js/pure' // prevents calls to stripe until used

// react-stripe-js requires a *stable* `stripe` promise: a new promise every
// render re-initializes Stripe.js and can leave <Elements> bound to the wrong
// intent. Cache one promise per publishable key at module scope.
const stripePromiseCache = new Map<string, ReturnType<typeof loadStripe>>()

export const getStripePromise = (publishableKey: string): ReturnType<typeof loadStripe> => {
  let promise = stripePromiseCache.get(publishableKey)
  if (!promise) {
    promise = loadStripe(publishableKey)
    stripePromiseCache.set(publishableKey, promise)
  }
  return promise
}
