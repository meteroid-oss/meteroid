import { Navigate, RouteObject } from 'react-router-dom'

import { StandardOnly } from '@/components/StandardOnly'
import { NotImplemented } from '@/features/NotImplemented'
import { BillingOutlet } from '@/pages/tenants/billing'
import { CreditNote, CreditNotes } from '@/pages/tenants/creditnote'
import { Invoice, Invoices } from '@/pages/tenants/invoice'
import { InvoiceCreate } from '@/pages/tenants/invoice/invoiceCreate'
import { CreateQuote, Quote, Quotes } from '@/pages/tenants/quotes'
import { Subscriptions } from '@/pages/tenants/subscription'
import { ChangePlanWizard } from '@/pages/tenants/subscription/changePlan/ChangePlanWizard'
import { Subscription } from '@/pages/tenants/subscription/subscription'
import { SubscriptionCreate } from '@/pages/tenants/subscription/subscriptionCreate'

export const billingRoutes: RouteObject = {
  element: <BillingOutlet />,
  children: [
    {
      index: true,
      element: <Navigate to="subscriptions" replace />,
    },
    {
      path: 'invoices',
      children: [
        { index: true, element: <Invoices />, handle: { title: 'Invoices' } },
        { path: ':invoiceId', element: <Invoice />, handle: { title: 'Invoice' } },
        {
          element: <StandardOnly />,
          children: [
            { path: 'create', element: <InvoiceCreate />, handle: { title: 'New invoice' } },
          ],
        },
      ],
    },
    {
      path: 'subscriptions',
      children: [
        { index: true, element: <Subscriptions />, handle: { title: 'Subscriptions' } },
        { path: ':subscriptionId', element: <Subscription />, handle: { title: 'Subscription' } },
        {
          element: <StandardOnly />,
          children: [
            {
              path: 'create',
              element: <SubscriptionCreate />,
              handle: { title: 'New subscription' },
            },
            {
              path: ':subscriptionId/change-plan',
              element: <ChangePlanWizard />,
              handle: { title: 'Change plan' },
            },
          ],
        },
      ],
    },
    {
      element: <StandardOnly />,
      children: [
        {
          path: 'quotes',
          children: [
            { index: true, element: <Quotes />, handle: { title: 'Quotes' } },
            { path: 'create', element: <CreateQuote />, handle: { title: 'New quote' } },
            { path: ':quoteId', element: <Quote />, handle: { title: 'Quote' } },
          ],
        },
        {
          path: 'credit-notes',
          children: [
            { index: true, element: <CreditNotes />, handle: { title: 'Credit notes' } },
            {
              path: ':creditNoteId',
              element: <CreditNote />,
              handle: { title: 'Credit note' },
            },
          ],
        },
      ],
    },
    { path: '*', element: <NotImplemented /> },
  ],
}
