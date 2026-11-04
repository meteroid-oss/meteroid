# Email ai clienti — testi fissi e contenuto predefinito dei solleciti.
# $company = nome commerciale dell'entità, $number = numero del documento, $amount = importo formattato, $legal_name = ragione sociale dell'entità.

## Invoice ready (kind invoice_ready)
invoice-ready-subject = La Sua fattura { $company } è disponibile
invoice-ready-heading = La Sua fattura è disponibile

## Receipt (kind invoice_paid)
invoice-paid-subject = La Sua ricevuta { $company }
invoice-paid-heading = Grazie per il Suo pagamento!

## Quote (kind quote_ready)
quote-ready-subject = Preventivo { $number } da { $company }
quote-ready-heading = Ha ricevuto un preventivo

## Credit note (kind credit_note_issued)
credit-note-issued-subject = La Sua nota di credito { $number } da { $company }
credit-note-issued-heading = È stata emessa una nota di credito

## Payment failed (kind payment_failed)
payment-failed-subject = Impossibile elaborare il pagamento della fattura { $number }
payment-failed-heading = Non è stato possibile elaborare il Suo pagamento
payment-failed-intro = La preghiamo di verificare il metodo di pagamento e di riprovare tramite il link qui sotto.

## Authentication required (kind authentication_required, 3-D Secure)
authentication-required-subject = Confermi il pagamento della fattura { $number }
authentication-required-heading = La Sua banca richiede la conferma di questo pagamento
authentication-required-intro = La Sua banca richiede una conferma prima di addebitare { $amount }. Bastano pochi secondi.

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Numero fattura
invoice-date = Data di emissione
amount-due = Importo dovuto
amount-paid = Importo pagato
credit-note-number = Numero nota di credito
amount-credited = Importo accreditato
quote-number = Numero preventivo
valid-until = Valido fino al
# $date = a long localized date, under the headline amount.
due-on = Scade il { $date }

## Status badges, next to the headline amount
status-paid = Pagato
status-payment-failed = Pagamento non riuscito
status-action-required = Azione richiesta

## Summary headline ($plan = plan name; the plain line when the invoice has none)
invoice-ready-plan-period = Questa fattura copre il Suo abbonamento { $plan } dal { $start } al { $end }.
invoice-ready-plan = Questa fattura riguarda il Suo abbonamento { $plan }.
invoice-paid-plan-period = Questo pagamento copre il Suo abbonamento { $plan } dal { $start } al { $end }.
invoice-paid-plan = Questo pagamento riguarda il Suo abbonamento { $plan }.

## Buttons
view-and-pay = Visualizza e paga
pay-invoice = Paga la fattura
confirm-payment = Conferma il pagamento
update-payment-method = Aggiorna il metodo di pagamento e paga
view-credit-note = Visualizza la nota di credito
view-and-sign-quote = Visualizza e firma il preventivo

## Layout
already-paid-disregard = Se ha già effettuato il pagamento, La preghiamo di ignorare questa email.
powered-by = Powered by Meteroid
test-mode-notice = Email di prova: inviata da un ambiente non di produzione. Non verrà addebitato alcun pagamento reale.

## Seeded default reminder sequence (D30): steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
reminder-default-before-subject = La fattura { "{{invoice_number}}" } scade il { "{{due_date}}" }
reminder-default-before-body =
    Gentile { "{{customer_name}}" },

    Le ricordiamo che la fattura { "{{invoice_number}}" } di { "{{amount_due}}" } scade il { "{{due_date}}" }.

reminder-default-due-subject = La fattura { "{{invoice_number}}" } scade oggi
reminder-default-due-body =
    Gentile { "{{customer_name}}" },

    la fattura { "{{invoice_number}}" } di { "{{amount_due}}" } scade oggi. Se ha già effettuato il pagamento, La ringraziamo e La preghiamo di ignorare questa email.

reminder-default-overdue-subject = La fattura { "{{invoice_number}}" } è scaduta da { "{{days_overdue}}" } giorni
reminder-default-overdue-body =
    Gentile { "{{customer_name}}" },

    non abbiamo ricevuto il pagamento della fattura { "{{invoice_number}}" } ({ "{{amount_due}}" }), scaduta il { "{{due_date}}" }.

reminder-default-final-subject = Ultimo sollecito: fattura { "{{invoice_number}}" }
reminder-default-final-body =
    Gentile { "{{customer_name}}" },

    la fattura { "{{invoice_number}}" } è ormai scaduta da { "{{days_overdue}}" } giorni. La preghiamo di saldare { "{{amount_due}}" } entro 7 giorni per evitare qualsiasi interruzione del servizio.

## Saldo prepagato (tipi wallet_balance_low, auto_top_up_failed). $threshold e $amount sono importi formattati.
wallet-balance-low-subject = Il Suo saldo prepagato { $company } è basso
wallet-balance-low-heading = Il Suo saldo prepagato è basso
wallet-balance-low-intro = Il Suo saldo disponibile è inferiore a { $threshold }. Aggiunga credito per continuare a coprire i Suoi consumi.
auto-top-up-failed-subject = Non è stato possibile addebitare la Sua ricarica automatica { $company }
auto-top-up-failed-heading = Non è stato possibile addebitare la Sua ricarica automatica
auto-top-up-failed-intro = Non siamo riusciti ad addebitare { $amount } per ricaricare il Suo saldo. Ricarichi il saldo manualmente e verifichi il metodo di pagamento.
auto-top-up-suspended-subject = Le ricariche automatiche del Suo saldo { $company } sono sospese
auto-top-up-suspended-heading = Le ricariche automatiche sono sospese
auto-top-up-suspended-intro = La Sua ricarica automatica di { $amount } è sospesa a seguito di un problema di pagamento. Verifichi il metodo di pagamento e le impostazioni di ricarica per riattivarla.
wallet-balance-low-intro-contact = Il Suo saldo disponibile è inferiore a { $threshold }. Contatti { $company } per aggiungere credito.
auto-top-up-failed-intro-contact = Non siamo riusciti ad addebitare { $amount } per ricaricare il Suo saldo. Verifichi il metodo di pagamento o contatti { $company } per aggiungere credito.
auto-top-up-suspended-intro-contact = La Sua ricarica automatica di { $amount } è sospesa a seguito di un problema di pagamento. Contatti { $company } per riattivarla.
auto-top-up-reversed-intro = Le ricariche automatiche sono sospese perché { $amount } di un pagamento di ricarica sono stati rimborsati o contestati. Verifichi il metodo di pagamento e le impostazioni di ricarica per riattivarle.
auto-top-up-reversed-intro-contact = Le ricariche automatiche sono sospese perché { $amount } di un pagamento di ricarica sono stati rimborsati o contestati. Contatti { $company } per riattivarle.
available-balance = Saldo disponibile
alert-threshold = Soglia di avviso
top-up-amount = Importo della ricarica
reversed-payment-amount = Rimborsato o contestato
status-low-balance = Saldo basso
status-paused = Sospesa

## Budget (tipi budget_threshold, budget_exceeded). $level è una percentuale, $budget un importo formattato.
budget-threshold-subject = La Sua spesa { $company } ha raggiunto il { $level } del budget
budget-threshold-heading = La Sua spesa ha raggiunto il { $level } del budget
budget-threshold-intro = La Sua spesa di questo periodo ha raggiunto il { $level } del budget di { $budget }. Include i consumi non ancora fatturati.
budget-exceeded-subject = La Sua spesa { $company } ha raggiunto il budget
budget-exceeded-heading = La Sua spesa ha raggiunto il budget
budget-exceeded-intro = La Sua spesa di questo periodo ha raggiunto il budget di { $budget }. Include i consumi non ancora fatturati.
budget-notify-only = Questo budget La avvisa soltanto. Non limita il Suo accesso.
budget-stops-usage-ahead = Quando la Sua spesa raggiunge questo budget, l'accesso ai consumi che conteggia può essere limitato fino al prossimo periodo di fatturazione. I consumi registrati fino ad allora vengono comunque addebitati.
budget-stops-usage-now = La Sua spesa ha raggiunto questo budget: l'accesso ai consumi che conteggia può essere limitato fino all'inizio del prossimo periodo di fatturazione o finché il budget non viene aumentato. I consumi registrati fino ad allora vengono comunque addebitati.
budget-stops-usage-ahead-open = Quando la Sua spesa raggiunge questo budget, l'accesso ai consumi che conteggia può essere limitato finché il budget non viene aumentato. I consumi registrati fino ad allora vengono comunque addebitati.
budget-stops-usage-now-open = La Sua spesa ha raggiunto questo budget: l'accesso ai consumi che conteggia può essere limitato finché il budget non viene aumentato. I consumi registrati fino ad allora vengono comunque addebitati.
budget-spend = Spesa del periodo
budget-amount = Budget
notification-level = Livello di notifica
period-start = Inizio del periodo
period-end = Fine del periodo
status-budget-alert = Avviso budget
status-budget-reached = Budget raggiunto
