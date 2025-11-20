import { RouteObject } from 'react-router-dom'

import { PortalCheckout } from '@/pages/portal/checkout'
import { PortalCheckoutSuccess } from '@/pages/portal/checkout-success'
import { PortalCustomer } from '@/pages/portal/customer'
import { PortalInvoicePayment } from '@/pages/portal/invoice-payment'
import { PortalQuote } from '@/pages/portal/quote'

export const portalRoutes: RouteObject = {
  children: [
    {
      path: 'checkout',
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
        },
      ],
    },
    {
      path: 'portal/invoice-payment',
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
        },
      ],
    },
    {
      path: 'quote',
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
    },
  ],
}
