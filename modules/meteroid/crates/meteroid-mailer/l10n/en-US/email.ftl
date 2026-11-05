# Customer emails — fixed strings and the seeded default reminder copy.
# $company = entity brand name, $number = document number, $amount = formatted money, $legal_name = entity legal name.

## Invoice ready (kind invoice_ready)
invoice-ready-subject = Your { $company } invoice is ready
invoice-ready-heading = Your invoice is ready

## Receipt (kind invoice_paid)
invoice-paid-subject = Your { $company } receipt
invoice-paid-heading = Thank you for your payment!

## Quote (kind quote_ready)
quote-ready-subject = Quote { $number } from { $company }
quote-ready-heading = You've received a quote

## Credit note (kind credit_note_issued)
credit-note-issued-subject = Your { $company } credit note { $number }
credit-note-issued-heading = A credit note was issued

## Payment failed (kind payment_failed)
payment-failed-subject = Payment for invoice { $number } could not be processed
payment-failed-heading = Your payment could not be processed
payment-failed-intro = Please check your payment method and retry the payment using the link below.

## Authentication required (kind authentication_required, 3-D Secure)
authentication-required-subject = Please confirm your payment for invoice { $number }
authentication-required-heading = Your bank needs you to confirm this payment
authentication-required-intro = Your bank asked for a confirmation before charging { $amount }. It only takes a few seconds.

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Invoice number
invoice-date = Invoice date
amount-due = Amount due
amount-paid = Amount paid
credit-note-number = Credit note number
amount-credited = Amount credited
quote-number = Quote number
valid-until = Valid until
# $date = a long localized date, under the headline amount.
due-on = Due { $date }

## Status badges, next to the headline amount
status-paid = Paid
status-payment-failed = Payment failed
status-action-required = Action required

## Summary headline ($plan = plan name; the plain line when the invoice has none)
invoice-ready-plan-period = This invoice covers your { $plan } subscription from { $start } to { $end }.
invoice-ready-plan = This invoice is for your { $plan } subscription.
invoice-paid-plan-period = This payment covers your { $plan } subscription from { $start } to { $end }.
invoice-paid-plan = This payment is for your { $plan } subscription.

## Buttons
view-and-pay = View and pay
pay-invoice = Pay invoice
confirm-payment = Confirm payment
update-payment-method = Update payment method and pay
view-credit-note = View credit note
view-and-sign-quote = View and sign quote

## Layout
already-paid-disregard = If you have already paid, please disregard this email.
powered-by = Powered by Meteroid
test-mode-notice = Test email: sent from a non-production environment. No real payment will be taken.

## Seeded default reminder sequence: steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
reminder-default-before-subject = Invoice { "{{invoice_number}}" } is due on { "{{due_date}}" }
reminder-default-before-body =
    Hi { "{{customer_name}}" },

    A quick heads-up: invoice { "{{invoice_number}}" } for { "{{amount_due}}" } is due on { "{{due_date}}" }.

reminder-default-due-subject = Invoice { "{{invoice_number}}" } is due today
reminder-default-due-body =
    Hi { "{{customer_name}}" },

    Invoice { "{{invoice_number}}" } for { "{{amount_due}}" } is due today. If you already paid, thank you and please ignore this email.

reminder-default-overdue-subject = Invoice { "{{invoice_number}}" } is { "{{days_overdue}}" } days overdue
reminder-default-overdue-body =
    Hi { "{{customer_name}}" },

    We haven't received payment for invoice { "{{invoice_number}}" } ({ "{{amount_due}}" }), which was due on { "{{due_date}}" }.

reminder-default-final-subject = Final notice: invoice { "{{invoice_number}}" }
reminder-default-final-body =
    Hi { "{{customer_name}}" },

    Invoice { "{{invoice_number}}" } is now { "{{days_overdue}}" } days overdue. Please pay { "{{amount_due}}" } within 7 days to avoid any interruption of service.

## Prepaid balance (kinds wallet_balance_low, auto_top_up_failed). $threshold and $amount are formatted money.
wallet-balance-low-subject = Your { $company } prepaid balance is running low
wallet-balance-low-heading = Your prepaid balance is running low
wallet-balance-low-intro = Your available balance is below { $threshold }. Add credit to keep your usage covered.
auto-top-up-failed-subject = Your { $company } automatic top-up could not be charged
auto-top-up-failed-heading = Your automatic top-up could not be charged
auto-top-up-failed-intro = We could not charge { $amount } to add credit to your balance. Please top up manually and check your payment method.
auto-top-up-suspended-subject = Automatic top-ups of your { $company } balance are paused
auto-top-up-suspended-heading = Automatic top-ups are paused
auto-top-up-suspended-intro = Your automatic top-up of { $amount } is paused after a payment problem. Review your payment method and top-up settings to resume it.
wallet-balance-low-intro-contact = Your available balance is below { $threshold }. Contact { $company } to add credit.
auto-top-up-failed-intro-contact = We could not charge { $amount } to add credit to your balance. Please check your payment method, or contact { $company } to add credit.
auto-top-up-suspended-intro-contact = Your automatic top-up of { $amount } is paused after a payment problem. Contact { $company } to resume it.
auto-top-up-reversed-intro = Automatic top-ups are paused because { $amount } of a top-up payment was refunded or disputed. Review your payment method and top-up settings to resume them.
auto-top-up-reversed-intro-contact = Automatic top-ups are paused because { $amount } of a top-up payment was refunded or disputed. Contact { $company } to resume them.
available-balance = Available balance
alert-threshold = Alert threshold
top-up-amount = Top-up amount
reversed-payment-amount = Refunded or disputed
status-low-balance = Low balance
status-paused = Paused

## Budget (kinds budget_threshold, budget_exceeded). $level is a percentage, $budget formatted money.
budget-threshold-subject = Your { $company } spend reached { $level } of your budget
budget-threshold-heading = Your spend reached { $level } of your budget
budget-threshold-intro = Your spend this period reached { $level } of your { $budget } budget. It includes usage not invoiced yet.
budget-exceeded-subject = Your { $company } spend reached your budget
budget-exceeded-heading = Your spend reached your budget
budget-exceeded-intro = Your spend this period reached your { $budget } budget. It includes usage not invoiced yet.
budget-notify-only = This budget only notifies you. It does not limit your access.
budget-stops-usage-ahead = Once your spend reaches this budget, access to the usage it counts may be limited until the next billing period. Usage recorded before then is still charged.
budget-stops-usage-now = Your spend reached this budget: access to the usage it counts may be limited until the next billing period starts or the budget is raised. Usage recorded before then is still charged.
budget-stops-usage-ahead-open = Once your spend reaches this budget, access to the usage it counts may be limited until the budget is raised. Usage recorded before then is still charged.
budget-stops-usage-now-open = Your spend reached this budget: access to the usage it counts may be limited until the budget is raised. Usage recorded before then is still charged.
budget-spend = Spend this period
budget-amount = Budget
notification-level = Notification level
period-start = Period start
period-end = Period end
status-budget-alert = Budget alert
status-budget-reached = Budget reached
