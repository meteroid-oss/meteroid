# Klant-e-mails — vaste teksten en de vooraf ingevulde standaardherinneringen.
# $company = merknaam van de entiteit, $number = documentnummer, $amount = opgemaakt bedrag, $legal_name = juridische naam van de entiteit.

## Invoice ready (kind invoice_ready)
invoice-ready-subject = Uw factuur van { $company } staat klaar
invoice-ready-heading = Uw factuur staat klaar

## Receipt (kind invoice_paid)
invoice-paid-subject = Uw betalingsbewijs van { $company }
invoice-paid-heading = Bedankt voor uw betaling!

## Quote (kind quote_ready)
quote-ready-subject = Offerte { $number } van { $company }
quote-ready-heading = U heeft een offerte ontvangen

## Credit note (kind credit_note_issued)
credit-note-issued-subject = Uw creditnota { $number } van { $company }
credit-note-issued-heading = Er is een creditnota uitgegeven

## Payment failed (kind payment_failed)
payment-failed-subject = De betaling voor factuur { $number } kon niet worden verwerkt
payment-failed-heading = Uw betaling kon niet worden verwerkt
payment-failed-intro = Controleer uw betaalmethode en probeer de betaling opnieuw via de onderstaande link.

## Authentication required (kind authentication_required, 3-D Secure)
authentication-required-subject = Bevestig uw betaling voor factuur { $number }
authentication-required-heading = Uw bank vraagt u deze betaling te bevestigen
authentication-required-intro = Uw bank vraagt om een bevestiging voordat { $amount } wordt afgeschreven. Dit duurt slechts enkele seconden.

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Factuurnummer
invoice-date = Factuurdatum
amount-due = Te betalen bedrag
amount-paid = Betaald bedrag
credit-note-number = Creditnotanummer
amount-credited = Gecrediteerd bedrag
quote-number = Offertenummer
valid-until = Geldig tot
# $date = a long localized date, under the headline amount.
due-on = Te betalen vóór { $date }

## Status badges, next to the headline amount
status-paid = Betaald
status-payment-failed = Betaling mislukt
status-action-required = Actie vereist

## Summary headline ($plan = plan name; the plain line when the invoice has none)
invoice-ready-plan-period = Deze factuur dekt uw abonnement { $plan } van { $start } tot { $end }.
invoice-ready-plan = Deze factuur betreft uw abonnement { $plan }.
invoice-paid-plan-period = Deze betaling dekt uw abonnement { $plan } van { $start } tot { $end }.
invoice-paid-plan = Deze betaling betreft uw abonnement { $plan }.

## Buttons
view-and-pay = Bekijken en betalen
pay-invoice = Factuur betalen
confirm-payment = Betaling bevestigen
update-payment-method = Betaalmethode bijwerken en betalen
view-credit-note = Creditnota bekijken
view-and-sign-quote = Offerte bekijken en ondertekenen

## Layout
already-paid-disregard = Heeft u al betaald, dan kunt u deze e-mail als niet verzonden beschouwen.
powered-by = Mogelijk gemaakt door Meteroid
test-mode-notice = Testmail: verzonden vanuit een niet-productieomgeving. Er wordt geen echte betaling uitgevoerd.

## Seeded default reminder sequence: steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
reminder-default-before-subject = Factuur { "{{invoice_number}}" } vervalt op { "{{due_date}}" }
reminder-default-before-body =
    Beste { "{{customer_name}}" },

    Ter herinnering: factuur { "{{invoice_number}}" } van { "{{amount_due}}" } vervalt op { "{{due_date}}" }.

reminder-default-due-subject = Factuur { "{{invoice_number}}" } vervalt vandaag
reminder-default-due-body =
    Beste { "{{customer_name}}" },

    Factuur { "{{invoice_number}}" } van { "{{amount_due}}" } vervalt vandaag. Heeft u al betaald, dan danken wij u en kunt u deze e-mail negeren.

reminder-default-overdue-subject = Factuur { "{{invoice_number}}" } is { "{{days_overdue}}" } dagen over de vervaldatum
reminder-default-overdue-body =
    Beste { "{{customer_name}}" },

    Wij hebben nog geen betaling ontvangen voor factuur { "{{invoice_number}}" } ({ "{{amount_due}}" }), die op { "{{due_date}}" } verviel.

reminder-default-final-subject = Laatste herinnering: factuur { "{{invoice_number}}" }
reminder-default-final-body =
    Beste { "{{customer_name}}" },

    Factuur { "{{invoice_number}}" } is inmiddels { "{{days_overdue}}" } dagen over de vervaldatum. Betaal { "{{amount_due}}" } binnen 7 dagen om een onderbreking van de dienstverlening te voorkomen.

## Prepaid tegoed (soorten wallet_balance_low, auto_top_up_failed). $threshold en $amount zijn opgemaakte bedragen.
wallet-balance-low-subject = Uw prepaid tegoed bij { $company } is laag
wallet-balance-low-heading = Uw prepaid tegoed is laag
wallet-balance-low-intro = Uw beschikbare tegoed is lager dan { $threshold }. Vul uw tegoed aan zodat uw gebruik gedekt blijft.
auto-top-up-failed-subject = Uw automatische opwaardering bij { $company } kon niet worden afgeschreven
auto-top-up-failed-heading = Uw automatische opwaardering kon niet worden afgeschreven
auto-top-up-failed-intro = We konden { $amount } niet afschrijven om uw tegoed aan te vullen. Vul uw tegoed handmatig aan en controleer uw betaalmethode.
auto-top-up-suspended-subject = Automatische opwaarderingen van uw tegoed bij { $company } zijn gepauzeerd
auto-top-up-suspended-heading = Automatische opwaarderingen zijn gepauzeerd
auto-top-up-suspended-intro = Uw automatische opwaardering van { $amount } is gepauzeerd na een betalingsprobleem. Controleer uw betaalmethode en opwaardeerinstellingen om deze te hervatten.
wallet-balance-low-intro-contact = Uw beschikbare tegoed is lager dan { $threshold }. Neem contact op met { $company } om tegoed toe te voegen.
auto-top-up-failed-intro-contact = We konden { $amount } niet afschrijven om uw tegoed aan te vullen. Controleer uw betaalmethode of neem contact op met { $company } om tegoed toe te voegen.
auto-top-up-suspended-intro-contact = Uw automatische opwaardering van { $amount } is gepauzeerd na een betalingsprobleem. Neem contact op met { $company } om deze te hervatten.
auto-top-up-reversed-intro = Automatische opwaarderingen zijn gepauzeerd omdat { $amount } van een opwaardeerbetaling is terugbetaald of betwist. Controleer uw betaalmethode en opwaardeerinstellingen om ze te hervatten.
auto-top-up-reversed-intro-contact = Automatische opwaarderingen zijn gepauzeerd omdat { $amount } van een opwaardeerbetaling is terugbetaald of betwist. Neem contact op met { $company } om ze te hervatten.
available-balance = Beschikbaar tegoed
alert-threshold = Waarschuwingsdrempel
top-up-amount = Opwaardeerbedrag
reversed-payment-amount = Terugbetaald of betwist
status-low-balance = Laag tegoed
status-paused = Gepauzeerd

## Budget (soorten budget_threshold, budget_exceeded). $level is een percentage, $budget een opgemaakt bedrag.
budget-threshold-subject = Uw uitgaven bij { $company } hebben { $level } van uw budget bereikt
budget-threshold-heading = Uw uitgaven hebben { $level } van uw budget bereikt
budget-threshold-intro = Uw uitgaven in deze periode hebben { $level } van uw budget van { $budget } bereikt. Ze omvatten gebruik dat nog niet is gefactureerd.
budget-exceeded-subject = Uw uitgaven bij { $company } hebben uw budget bereikt
budget-exceeded-heading = Uw uitgaven hebben uw budget bereikt
budget-exceeded-intro = Uw uitgaven in deze periode hebben uw budget van { $budget } bereikt. Ze omvatten gebruik dat nog niet is gefactureerd.
budget-notify-only = Dit budget waarschuwt u alleen. Het beperkt uw toegang niet.
budget-stops-usage-ahead = Zodra uw uitgaven dit budget bereiken, kan de toegang tot het gebruik dat ervoor telt beperkt worden tot de volgende factuurperiode. Gebruik dat tot dan is geregistreerd, wordt nog steeds gefactureerd.
budget-stops-usage-now = Uw uitgaven hebben dit budget bereikt: de toegang tot het gebruik dat ervoor telt kan beperkt worden tot de volgende factuurperiode begint of het budget wordt verhoogd. Gebruik dat tot dan is geregistreerd, wordt nog steeds gefactureerd.
budget-stops-usage-ahead-open = Zodra uw uitgaven dit budget bereiken, kan de toegang tot het gebruik dat ervoor telt beperkt worden tot het budget wordt verhoogd. Gebruik dat tot dan is geregistreerd, wordt nog steeds gefactureerd.
budget-stops-usage-now-open = Uw uitgaven hebben dit budget bereikt: de toegang tot het gebruik dat ervoor telt kan beperkt worden tot het budget wordt verhoogd. Gebruik dat tot dan is geregistreerd, wordt nog steeds gefactureerd.
budget-spend = Uitgaven in de periode
budget-amount = Budget
notification-level = Meldingsniveau
period-start = Begin van de periode
period-end = Einde van de periode
status-budget-alert = Budgetwaarschuwing
status-budget-reached = Budget bereikt
