## Invoicing workers

Links to :

- invoice provider (Stripe Invoice, Netsuite, ...)
- payment providers (Stripe, Adyen, ...)

//

- webhook api

// invoicing
We want :

- draft invoices from the beginning of each billing period

- amount data to be updated regularly (+ optionaly on demand)

- at the end of a billing period, change the status to pending

- at the end of the grace period, recompute, change the status to final
  For each invoice in status pending or draft whose grace period has ended :

  - fetch again the subscription, customer data, organization data
  - compute the billing data with maximum precision
  - build the final invoice
  - mark as Finalized

- fetch finalized invoices with issued = false & send to stripe

  - send to stripe (later: meteroid as provider)
  - mark as issued = true, or if error save last_issue_error, last_issue_date, issue_retry_count (or if possible, audit log)

- internal grpc api
- asynchronous/scheduled process :
  - draft invoice creation when billing period starts
  - pre-finalize when billing period ends
  - finalize when grace period ends
  - invoice amount updates (usage is realtime, but the invoice amount is not. We update it on-demand when accessed, and on schedule for dashboard usage)
  - issuer
