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

## Quote signature code (one-time code before signing)
quote-signature-code-subject = Uw code om offerte { $number } te ondertekenen
quote-signature-code-heading = Uw ondertekeningscode
quote-signature-code-intro = Voer deze code in om offerte { $number } van { $company } te ondertekenen. De code verloopt over { $minutes } minuten.
quote-signature-code-ignore = Heeft u niet geprobeerd deze offerte te ondertekenen, negeer dan deze e-mail.

## Quote discussion (a message or a change request)
quote-message-subject = Nieuw bericht over offerte { $number }
quote-changes-requested-subject = Wijzigingen gevraagd voor offerte { $number }
quote-message-heading = { $author } heeft een bericht gestuurd over offerte { $number }
quote-changes-requested-heading = { $author } vraagt wijzigingen in offerte { $number }
view-quote = Offerte bekijken
quote-customer = Klant

## Quote accepted (kind quote_accepted), and its outcome to the team
quote-accepted-subject = Offerte { $number } van { $company } is geaccepteerd
quote-accepted-heading = Uw offerte is geaccepteerd
quote-seller = Van
quote-accepted-on = Geaccepteerd op
view-signed-quote = Ondertekende offerte bekijken
quote-outcome-accepted-subject = { $customer } heeft offerte { $number } geaccepteerd
quote-outcome-declined-subject = { $customer } heeft offerte { $number } geweigerd

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

## Betaalmethode (soort payment_method_requested: facturatie verhuisd van een andere aanbieder)
payment-method-requested-subject = Actie vereist: voeg een betaalmethode toe voor uw { $company }-abonnement
payment-method-requested-heading = Voeg een betaalmethode toe om uw abonnement te behouden
payment-method-requested-intro = { $company } factureert uw abonnement nu rechtstreeks in plaats van via { $provider }. Uw abonnement en prijs blijven hetzelfde.
payment-method-requested-by = Voeg vóór { $date } een betaalmethode toe zodat uw volgende betaling slaagt.
payment-method-requested-anytime = Voeg een betaalmethode toe zodat uw volgende betaling slaagt.
payment-method-requested-hedge = Als u er al een hebt toegevoegd, kunt u deze e-mail negeren.
payment-method-requested-heading-by = Voeg vóór { $date } een betaalmethode toe
payment-method-moving-intro = { $company } verplaatst de facturering van uw abonnement op { $date } van { $provider } rechtstreeks naar { $company }. Uw abonnement en prijs blijven hetzelfde.
payment-method-moving-kept-cards = { $provider } kan opgeslagen kaarten niet overzetten, daarom vragen we u er een toe te voegen. Uw facturen komen dan van { $company }, dat ook de belasting berekent.
payment-method-moving-no-reuse = We kunnen de bij { $provider } opgeslagen kaart niet hergebruiken, daarom vragen we u er een te bevestigen.
payment-method-moving-paypal = { $provider } kan PayPal-betalingen niet overzetten, daarom vragen we u een betaalmethode toe te voegen.
payment-method-moving-invoice = U betaalt nu per factuur. Voeg een betaalmethode toe zodat uw betalingen automatisch worden afgeschreven.
payment-method-link-hedge = Met deze link logt u in op uw factuurportaal van { $company }. Is de link verlopen, dan kunt u op de pagina een nieuwe aanvragen. Hebt u al een betaalmethode toegevoegd, dan kunt u deze e-mail negeren.
payment-method-stop = Deze herinneringen niet meer ontvangen
add-payment-method = Betaalmethode toevoegen

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Factuurnummer
invoice-date = Factuurdatum
amount-due = Te betalen bedrag
amount-paid = Betaald bedrag
amount-carried = Overgedragen
credit-note-number = Creditnotanummer
amount-credited = Gecrediteerd bedrag
quote-number = Offertenummer
valid-until = Geldig tot
# $date = a long localized date, under the headline amount.
due-on = Te betalen vóór { $date }
collected-with-next-invoice = Geïnd met uw volgende factuur

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
view-invoice = Factuur bekijken
pay-invoice = Factuur betalen
confirm-payment = Betaling bevestigen
update-payment-method = Betaalmethode bijwerken en betalen
view-credit-note = Creditnota bekijken
view-and-sign-quote = Offerte bekijken en ondertekenen

## Layout
already-paid-disregard = Heeft u al betaald, dan kunt u deze e-mail als niet verzonden beschouwen.
powered-by = Mogelijk gemaakt door Meteroid
test-mode-notice = Testmodus: verzonden vanuit een niet-productieomgeving.

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

## Opzegging (soort subscription_cancelled). $plan is de naam van het abonnement.
subscription-cancelled-subject = Uw opzegging bij { $company } is bevestigd
subscription-cancelled-heading = Opzegging bevestigd
subscription-cancelled-intro = Uw abonnement { $plan } blijft actief tot de laatste dag van de dienst hieronder.
cancellation-received = Opzegging ontvangen op
# $date = een lange gelokaliseerde datum, $time = HH:MM in UTC.
cancellation-received-at = { $date } om { $time } UTC
last-day-of-service = Laatste dag van de dienstverlening

## Verlopende kaart (type payment_method_expiring). $method bijv. "Visa •••• 4242", $date een lange gelokaliseerde datum.
payment-method-expiring-subject = Je betaalmethode voor { $company } verloopt binnenkort
payment-method-expiring-heading = Je kaart verloopt binnenkort
payment-method-expiring-intro = Je { $method } verloopt op { $date }. Werk hem bij zodat je abonnement zonder onderbreking wordt verlengd.
payment-method-label = Betaalmethode
payment-method-expires = Verloopt
update-card = Betaalmethode bijwerken

## Renewal reminder (kind renewal_reminder). $plan is the plan name, $date a long localized date.
renewal-reminder-renewing-subject = Je { $company }-abonnement wordt binnenkort verlengd
renewal-reminder-renewing-heading = Je abonnement wordt binnenkort verlengd
renewal-reminder-renewing-intro = Je abonnement { $plan } wordt op { $date } automatisch verlengd.
renewal-reminder-ending-subject = Je { $company }-abonnement loopt binnenkort af
renewal-reminder-ending-heading = Je abonnement loopt binnenkort af
renewal-reminder-ending-intro = Je abonnement { $plan } loopt af op { $date }. Neem contact met ons op om het voort te zetten.
renewal-reminder-renews-on = Verlenging op
renewal-reminder-ends-on = Loopt af op
renewal-reminder-next-term-ends = Volgende termijn loopt af
renewal-reminder-price-change = Prijswijziging
# $percent = the uplift, as a decimal.
renewal-reminder-uplift = +{ $percent } %
renewal-reminder-opt-out-by = Opzeggen kan tot

## Proefperiodes (soorten trial_ending, trial_started, trial_ended). $plan en $next zijn namen van abonnementen, $date een lange datum
## (het einde van een proefperiode: de dag waarop de facturatie begint), $amount een opgemaakt bedrag, $interval bijv. "per maand".
trial-ending-subject = Je proefperiode bij { $company } eindigt op { $date }
trial-ending-heading = Je proefperiode eindigt binnenkort
trial-ending-charged-intro = Je proefperiode voor { $plan } eindigt op { $date }. Daarna wordt { $amount } afgeschreven van je opgeslagen betaalmethode.
trial-ending-charged-intro-no-amount = Je proefperiode voor { $plan } eindigt op { $date }. Daarna begint de facturatie via je opgeslagen betaalmethode.
trial-ending-invoiced-intro = Je proefperiode voor { $plan } eindigt op { $date }. Daarna ontvang je je eerste factuur, van { $amount }, met je gebruikelijke betalingstermijn.
trial-ending-invoiced-intro-no-amount = Je proefperiode voor { $plan } eindigt op { $date }. Daarna begint de facturatie, met je gebruikelijke betalingstermijn.
trial-ending-ends-intro = Je proefperiode voor { $plan } eindigt op { $date } en je abonnement eindigt dan ook. Er wordt niets in rekening gebracht.
trial-ending-switches-intro = Je proefperiode voor { $plan } eindigt op { $date }. Je abonnement gaat daarna over naar { $next }.
trial-ending-no-method-heading = Voeg een betaalmethode toe om je abonnement te behouden
trial-ending-no-method-intro = Je proefperiode voor { $plan } eindigt op { $date } en er is geen betaalmethode opgeslagen.
trial-ending-no-method-await = Zonder betaalmethode wordt je abonnement gepauzeerd tot de eerste factuur is betaald.
trial-ending-no-method-cancel = Zonder betaalmethode eindigt je abonnement op { $date }.
trial-ending-no-method-switch = Zonder betaalmethode gaat je abonnement op { $date } over naar { $next }.
trial-ending-no-method-ask = Voeg er vóór { $date } een toe om door te gaan met { $plan }.
trial-manage-before = Je kunt je abonnement vóór { $date } wijzigen of opzeggen in je factuurportaal.
trial-first-payment = Eerste betaling
trial-first-invoice = Eerste factuur
trial-charged-on = Afgeschreven op { $date }
trial-invoiced-on = Gefactureerd op { $date }
trial-plan = Abonnement
trial-starts-on = Begin proefperiode
trial-ends-on = Einde proefperiode
trial-ended-on = Proefperiode geëindigd op
trial-next-plan = Volgend abonnement
trial-started-subject = Je proefperiode bij { $company } is begonnen
trial-started-heading = Je proefperiode is begonnen
trial-started-intro = Je proefperiode voor { $plan } eindigt op { $date }. Daarna wordt { $amount } { $interval } afgeschreven van je opgeslagen betaalmethode tot je opzegt.
trial-started-intro-no-amount = Je proefperiode voor { $plan } eindigt op { $date }. Daarna begint de facturatie via je opgeslagen betaalmethode tot je opzegt.
trial-after-trial = Na je proefperiode
trial-charged-from = { $interval } vanaf { $date }
trial-per-month = per maand
trial-per-quarter = elke 3 maanden
trial-per-half-year = elke 6 maanden
trial-per-year = per jaar
trial-ended-subject = Je proefperiode bij { $company } is afgelopen
trial-ended-heading = Je proefperiode is afgelopen
trial-ended-cancelled-intro = Je proefperiode voor { $plan } is op { $date } afgelopen en je abonnement is daarmee beëindigd. Er is niets in rekening gebracht.
trial-ended-switched-intro = Je proefperiode is op { $date } afgelopen. Je abonnement loopt nu op { $plan }.
trial-ended-awaiting-payment-intro = Je proefperiode voor { $plan } is op { $date } afgelopen. Voeg een betaalmethode toe en betaal je eerste factuur om je abonnement voort te zetten.
manage-subscription = Abonnement beheren
view-plans = Abonnementen bekijken
