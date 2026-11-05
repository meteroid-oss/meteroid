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

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Rechnungsnummer
invoice-date = Rechnungsdatum
amount-due = Fälliger Betrag
amount-paid = Gezahlter Betrag
credit-note-number = Nummer der Rechnungskorrektur
amount-credited = Gutgeschriebener Betrag
quote-number = Angebotsnummer
valid-until = Gültig bis
# $date = a long localized date, under the headline amount.
due-on = Fällig am { $date }

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
