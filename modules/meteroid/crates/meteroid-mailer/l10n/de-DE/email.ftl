# Kunden-E-Mails — feste Texte und die vorbelegten Standard-Erinnerungen.
# $company = Markenname der Einheit, $number = Belegnummer, $amount = formatierter Betrag, $legal_name = Firmenname der Einheit.

## Invoice ready (kind invoice_ready)
invoice-ready-subject = Ihre Rechnung von { $company } ist verfügbar
invoice-ready-heading = Ihre Rechnung ist verfügbar

## Receipt (kind invoice_paid)
invoice-paid-subject = Ihre Zahlungsbestätigung von { $company }
invoice-paid-heading = Vielen Dank für Ihre Zahlung!

## Quote (kind quote_ready)
quote-ready-subject = Angebot { $number } von { $company }
quote-ready-heading = Sie haben ein Angebot erhalten

## Quote signature code (one-time code before signing)
quote-signature-code-subject = Ihr Code zum Unterzeichnen des Angebots { $number }
quote-signature-code-heading = Ihr Signaturcode
quote-signature-code-intro = Geben Sie diesen Code ein, um das Angebot { $number } von { $company } zu unterzeichnen. Er läuft in { $minutes } Minuten ab.
quote-signature-code-ignore = Wenn Sie nicht versucht haben, dieses Angebot zu unterzeichnen, ignorieren Sie diese E-Mail.

## Quote discussion (a message or a change request)
quote-message-subject = Neue Nachricht zum Angebot { $number }
quote-changes-requested-subject = Änderungen am Angebot { $number } angefragt
quote-message-heading = { $author } hat eine Nachricht zum Angebot { $number } gesendet
quote-changes-requested-heading = { $author } bittet um Änderungen am Angebot { $number }
view-quote = Angebot ansehen
quote-customer = Kunde

## Quote accepted (kind quote_accepted), and its outcome to the team
quote-accepted-subject = Angebot { $number } von { $company } ist angenommen
quote-accepted-heading = Ihr Angebot ist angenommen
quote-seller = Von
quote-accepted-on = Angenommen am
view-signed-quote = Unterzeichnetes Angebot ansehen
quote-outcome-accepted-subject = { $customer } hat das Angebot { $number } angenommen
quote-outcome-declined-subject = { $customer } hat das Angebot { $number } abgelehnt

## Credit note (kind credit_note_issued)
credit-note-issued-subject = Ihre Rechnungskorrektur { $number } von { $company }
credit-note-issued-heading = Eine Rechnungskorrektur wurde ausgestellt

## Payment failed (kind payment_failed)
payment-failed-subject = Die Zahlung für Rechnung { $number } konnte nicht verarbeitet werden
payment-failed-heading = Ihre Zahlung konnte nicht verarbeitet werden
payment-failed-intro = Bitte überprüfen Sie Ihre Zahlungsmethode und wiederholen Sie die Zahlung über den untenstehenden Link.

## Authentication required (kind authentication_required, 3-D Secure)
authentication-required-subject = Bitte bestätigen Sie Ihre Zahlung für Rechnung { $number }
authentication-required-heading = Ihre Bank bittet Sie, diese Zahlung zu bestätigen
authentication-required-intro = Ihre Bank verlangt eine Bestätigung, bevor { $amount } abgebucht werden. Das dauert nur wenige Sekunden.

## Zahlungsmethode (Art payment_method_requested: Abrechnung von einem anderen Anbieter umgezogen)
payment-method-requested-subject = Handlungsbedarf: Fügen Sie eine Zahlungsmethode für Ihr { $company }-Abonnement hinzu
payment-method-requested-heading = Fügen Sie eine Zahlungsmethode hinzu, um Ihr Abonnement zu behalten
payment-method-requested-intro = { $company } rechnet Ihr Abonnement jetzt direkt ab statt über { $provider }. Ihr Tarif und Ihr Preis bleiben gleich.
payment-method-requested-by = Bitte fügen Sie vor dem { $date } eine Zahlungsmethode hinzu, damit Ihre nächste Zahlung erfolgen kann.
payment-method-requested-anytime = Bitte fügen Sie eine Zahlungsmethode hinzu, damit Ihre nächste Zahlung erfolgen kann.
payment-method-requested-hedge = Wenn Sie bereits eine hinzugefügt haben, können Sie diese E-Mail ignorieren.
payment-method-requested-heading-by = Fügen Sie vor dem { $date } eine Zahlungsmethode hinzu
payment-method-moving-intro = { $company } verlegt die Abrechnung Ihres Abonnements am { $date } von { $provider } direkt zu { $company }. Ihr Tarif und Ihr Preis bleiben gleich.
payment-method-moving-kept-cards = { $provider } kann gespeicherte Karten nicht übertragen, daher bitten wir Sie, eine hinzuzufügen. Ihre Rechnungen kommen dann von { $company }, das auch die Steuer berechnet.
payment-method-moving-no-reuse = Wir können die bei { $provider } gespeicherte Karte nicht weiterverwenden, daher bitten wir Sie, eine zu bestätigen.
payment-method-moving-paypal = { $provider } kann PayPal-Zahlungen nicht übertragen, daher bitten wir Sie, eine Zahlungsmethode hinzuzufügen.
payment-method-moving-invoice = Sie zahlen derzeit per Rechnung. Fügen Sie eine Zahlungsmethode hinzu, damit Ihre Zahlungen automatisch eingezogen werden.
payment-method-link-hedge = Mit diesem Link melden Sie sich in Ihrem Abrechnungsportal von { $company } an. Ist er abgelaufen, können Sie auf der Seite einen neuen anfordern. Wenn Sie bereits eine Zahlungsmethode hinzugefügt haben, können Sie diese E-Mail ignorieren.
payment-method-stop = Diese Erinnerungen abbestellen
add-payment-method = Zahlungsmethode hinzufügen

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Rechnungsnummer
invoice-date = Rechnungsdatum
amount-due = Fälliger Betrag
amount-paid = Gezahlter Betrag
amount-carried = Übertragen
credit-note-number = Nummer der Rechnungskorrektur
amount-credited = Gutgeschriebener Betrag
quote-number = Angebotsnummer
valid-until = Gültig bis
# $date = a long localized date, under the headline amount.
due-on = Fällig am { $date }
collected-with-next-invoice = Wird mit Ihrer nächsten Rechnung eingezogen

## Status badges, next to the headline amount
status-paid = Bezahlt
status-payment-failed = Zahlung fehlgeschlagen
status-action-required = Aktion erforderlich

## Summary headline ($plan = plan name; the plain line when the invoice has none)
invoice-ready-plan-period = Diese Rechnung umfasst Ihr Abonnement { $plan } vom { $start } bis { $end }.
invoice-ready-plan = Diese Rechnung betrifft Ihr Abonnement { $plan }.
invoice-paid-plan-period = Diese Zahlung umfasst Ihr Abonnement { $plan } vom { $start } bis { $end }.
invoice-paid-plan = Diese Zahlung betrifft Ihr Abonnement { $plan }.

## Buttons
view-and-pay = Ansehen und bezahlen
view-invoice = Rechnung ansehen
pay-invoice = Rechnung bezahlen
confirm-payment = Zahlung bestätigen
update-payment-method = Zahlungsmethode aktualisieren und bezahlen
view-credit-note = Rechnungskorrektur ansehen
view-and-sign-quote = Angebot ansehen und unterzeichnen

## Layout
already-paid-disregard = Falls Sie bereits bezahlt haben, betrachten Sie diese E-Mail bitte als gegenstandslos.
powered-by = Bereitgestellt von Meteroid
test-mode-notice = Test-E-Mail: aus einer Nicht-Produktionsumgebung gesendet. Es wird keine echte Zahlung eingezogen.

## Seeded default reminder sequence: steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
reminder-default-before-subject = Rechnung { "{{invoice_number}}" } ist am { "{{due_date}}" } fällig
reminder-default-before-body =
    Guten Tag { "{{customer_name}}" },

    zur Erinnerung: Rechnung { "{{invoice_number}}" } über { "{{amount_due}}" } ist am { "{{due_date}}" } fällig.

reminder-default-due-subject = Rechnung { "{{invoice_number}}" } ist heute fällig
reminder-default-due-body =
    Guten Tag { "{{customer_name}}" },

    Rechnung { "{{invoice_number}}" } über { "{{amount_due}}" } ist heute fällig. Falls Sie bereits bezahlt haben, danken wir Ihnen und bitten Sie, diese E-Mail zu ignorieren.

reminder-default-overdue-subject = Rechnung { "{{invoice_number}}" } ist seit { "{{days_overdue}}" } Tagen überfällig
reminder-default-overdue-body =
    Guten Tag { "{{customer_name}}" },

    für Rechnung { "{{invoice_number}}" } ({ "{{amount_due}}" }), fällig am { "{{due_date}}" }, ist bei uns noch keine Zahlung eingegangen.

reminder-default-final-subject = Letzte Mahnung: Rechnung { "{{invoice_number}}" }
reminder-default-final-body =
    Guten Tag { "{{customer_name}}" },

    Rechnung { "{{invoice_number}}" } ist inzwischen seit { "{{days_overdue}}" } Tagen überfällig. Bitte begleichen Sie { "{{amount_due}}" } innerhalb von 7 Tagen, um eine Unterbrechung des Dienstes zu vermeiden.

## Prepaid-Guthaben (Arten wallet_balance_low, auto_top_up_failed). $threshold und $amount sind formatierte Beträge.
wallet-balance-low-subject = Ihr Prepaid-Guthaben bei { $company } ist niedrig
wallet-balance-low-heading = Ihr Prepaid-Guthaben ist niedrig
wallet-balance-low-intro = Ihr verfügbares Guthaben liegt unter { $threshold }. Laden Sie Guthaben auf, damit Ihre Nutzung weiterhin gedeckt ist.
auto-top-up-failed-subject = Ihre automatische Aufladung bei { $company } konnte nicht abgebucht werden
auto-top-up-failed-heading = Ihre automatische Aufladung konnte nicht abgebucht werden
auto-top-up-failed-intro = Wir konnten { $amount } für die Aufladung Ihres Guthabens nicht abbuchen. Bitte laden Sie Ihr Guthaben manuell auf und überprüfen Sie Ihre Zahlungsmethode.
auto-top-up-suspended-subject = Automatische Aufladungen Ihres Guthabens bei { $company } sind pausiert
auto-top-up-suspended-heading = Automatische Aufladungen sind pausiert
auto-top-up-suspended-intro = Ihre automatische Aufladung von { $amount } ist nach einem Zahlungsproblem pausiert. Überprüfen Sie Ihre Zahlungsmethode und Ihre Aufladeeinstellungen, um sie fortzusetzen.
wallet-balance-low-intro-contact = Ihr verfügbares Guthaben liegt unter { $threshold }. Wenden Sie sich an { $company }, um Guthaben aufzuladen.
auto-top-up-failed-intro-contact = Wir konnten { $amount } für die Aufladung Ihres Guthabens nicht abbuchen. Bitte überprüfen Sie Ihre Zahlungsmethode oder wenden Sie sich an { $company }, um Guthaben aufzuladen.
auto-top-up-suspended-intro-contact = Ihre automatische Aufladung von { $amount } ist nach einem Zahlungsproblem pausiert. Wenden Sie sich an { $company }, um sie fortzusetzen.
auto-top-up-reversed-intro = Automatische Aufladungen sind pausiert, weil { $amount } einer Aufladezahlung erstattet oder angefochten wurden. Überprüfen Sie Ihre Zahlungsmethode und Ihre Aufladeeinstellungen, um sie fortzusetzen.
auto-top-up-reversed-intro-contact = Automatische Aufladungen sind pausiert, weil { $amount } einer Aufladezahlung erstattet oder angefochten wurden. Wenden Sie sich an { $company }, um sie fortzusetzen.
available-balance = Verfügbares Guthaben
alert-threshold = Warnschwelle
top-up-amount = Aufladebetrag
reversed-payment-amount = Erstattet oder angefochten
status-low-balance = Niedriges Guthaben
status-paused = Pausiert

## Budget (Arten budget_threshold, budget_exceeded). $level ist ein Prozentwert, $budget ein formatierter Betrag.
budget-threshold-subject = Ihre Ausgaben bei { $company } haben { $level } Ihres Budgets erreicht
budget-threshold-heading = Ihre Ausgaben haben { $level } Ihres Budgets erreicht
budget-threshold-intro = Ihre Ausgaben in diesem Zeitraum haben { $level } Ihres Budgets von { $budget } erreicht. Sie enthalten noch nicht abgerechnete Nutzung.
budget-exceeded-subject = Ihre Ausgaben bei { $company } haben Ihr Budget erreicht
budget-exceeded-heading = Ihre Ausgaben haben Ihr Budget erreicht
budget-exceeded-intro = Ihre Ausgaben in diesem Zeitraum haben Ihr Budget von { $budget } erreicht. Sie enthalten noch nicht abgerechnete Nutzung.
budget-notify-only = Dieses Budget benachrichtigt Sie nur. Es schränkt Ihren Zugriff nicht ein.
budget-stops-usage-ahead = Sobald Ihre Ausgaben dieses Budget erreichen, kann der Zugriff auf die darauf angerechnete Nutzung bis zum nächsten Abrechnungszeitraum eingeschränkt werden. Bis dahin erfasste Nutzung wird weiterhin berechnet.
budget-stops-usage-now = Ihre Ausgaben haben dieses Budget erreicht: Der Zugriff auf die darauf angerechnete Nutzung kann eingeschränkt werden, bis der nächste Abrechnungszeitraum beginnt oder das Budget erhöht wird. Bis dahin erfasste Nutzung wird weiterhin berechnet.
budget-stops-usage-ahead-open = Sobald Ihre Ausgaben dieses Budget erreichen, kann der Zugriff auf die darauf angerechnete Nutzung eingeschränkt werden, bis das Budget erhöht wird. Bis dahin erfasste Nutzung wird weiterhin berechnet.
budget-stops-usage-now-open = Ihre Ausgaben haben dieses Budget erreicht: Der Zugriff auf die darauf angerechnete Nutzung kann eingeschränkt werden, bis das Budget erhöht wird. Bis dahin erfasste Nutzung wird weiterhin berechnet.
budget-spend = Ausgaben im Zeitraum
budget-amount = Budget
notification-level = Benachrichtigungsstufe
period-start = Beginn des Zeitraums
period-end = Ende des Zeitraums
status-budget-alert = Budgetwarnung
status-budget-reached = Budget erreicht

## Kündigung (Art subscription_cancelled). $plan ist der Name des Tarifs.
subscription-cancelled-subject = Bestätigung Ihrer Kündigung bei { $company }
subscription-cancelled-heading = Kündigung bestätigt
subscription-cancelled-intro = Ihr Abonnement { $plan } bleibt bis zum unten genannten letzten Leistungstag aktiv.
cancellation-received = Kündigung eingegangen am
# $date = ein langes lokalisiertes Datum, $time = HH:MM in UTC.
cancellation-received-at = { $date } um { $time } Uhr (UTC)
last-day-of-service = Letzter Leistungstag

## Ablaufende Karte (Typ payment_method_expiring). $method z. B. "Visa •••• 4242", $date ein langes lokalisiertes Datum.
payment-method-expiring-subject = Ihre Zahlungsmethode für { $company } läuft bald ab
payment-method-expiring-heading = Ihre Karte läuft bald ab
payment-method-expiring-intro = Ihre { $method } läuft am { $date } ab. Aktualisieren Sie sie, damit Ihr Abonnement ohne Unterbrechung verlängert wird.
payment-method-label = Zahlungsmethode
payment-method-expires = Gültig bis
update-card = Zahlungsmethode aktualisieren

## Renewal reminder (kind renewal_reminder). $plan is the plan name, $date a long localized date.
renewal-reminder-renewing-subject = Ihr { $company }-Abonnement verlängert sich bald
renewal-reminder-renewing-heading = Ihr Abonnement verlängert sich bald
renewal-reminder-renewing-intro = Ihr Abonnement { $plan } verlängert sich am { $date } automatisch.
renewal-reminder-ending-subject = Ihr { $company }-Abonnement endet bald
renewal-reminder-ending-heading = Ihr Abonnement endet bald
renewal-reminder-ending-intro = Ihr Abonnement { $plan } endet am { $date }. Kontaktieren Sie uns, um es fortzuführen.
renewal-reminder-renews-on = Verlängerung am
renewal-reminder-ends-on = Endet am
renewal-reminder-next-term-ends = Nächste Laufzeit endet
renewal-reminder-price-change = Preisänderung
# $percent = the uplift, as a decimal.
renewal-reminder-uplift = +{ $percent } %
renewal-reminder-opt-out-by = Kündigung möglich bis
