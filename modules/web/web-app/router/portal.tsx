import { redirect, RouteObject } from 'react-router-dom'

import { PortalCheckout } from '@/pages/portal/checkout'
import { PortalCheckoutSuccess } from '@/pages/portal/checkout-success'
import { PortalCustomer } from '@/pages/portal/customer'
import { PortalInvoicePayment } from '@/pages/portal/invoice-payment'
import { PortalQuote } from '@/pages/portal/quote'
import { PortalSubscription } from '@/pages/portal/subscription'

// TODO temporary, standardize
const redirectToCheckout = ({
  params,
  request,
}: {
  params: { subscriptionId?: string }
  request: Request
}) => {
  const url = new URL(request.url)
  const destination = params.subscriptionId
    ? `/checkout/${params.subscriptionId}${url.search}`
    : `/checkout${url.search}`
  return redirect(destination)
}

export const portalRoutes: RouteObject = {
  children: [
    {
      path: 'checkout',
      handle: { title: 'Checkout' },
      children: [
        {
          index: true,
          element: <PortalCheckout />,
        },
        {
          path: ':subscriptionId',
          element: <PortalCheckout />,
        },
        {
          path: 'success',
          element: <PortalCheckoutSuccess />,
          handle: { title: 'Checkout success' },
        },
      ],
    },
    {
      path: 'portal/checkout',
      children: [
        {
          index: true,
          loader: redirectToCheckout,
        },
        {
          path: ':subscriptionId',
          loader: redirectToCheckout,
        },
      ],
    },
    {
      path: 'portal/invoice-payment',
      handle: { title: 'Pay invoice' },
      children: [
        {
          index: true,
          element: <PortalInvoicePayment />,
        },
        {
          path: ':invoiceId',
          element: <PortalInvoicePayment />,
        },
        {
          path: 'success',
          element: <PortalCheckoutSuccess />,
          handle: { title: 'Payment success' },
        },
      ],
    },
    {
      path: 'portal/quote',
      handle: { title: 'Quote' },
      children: [
        {
          index: true,
          element: <PortalQuote />,
        },
      ],
    },
    {
      path: 'portal/customer',
      element: <PortalCustomer />,
      handle: { title: 'Customer portal' },
    },
    {
      path: 'portal/subscription/:subscriptionId',
      element: <PortalSubscription />,
      handle: { title: 'Subscription' },
    },
  ],
}
