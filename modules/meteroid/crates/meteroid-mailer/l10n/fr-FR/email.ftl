# Emails clients — chaînes fixes et contenu par défaut des relances.
# $company = nom commercial de l'entité, $number = numéro du document, $amount = montant formaté, $legal_name = raison sociale de l'entité.

## Invoice ready (kind invoice_ready)
invoice-ready-subject = Votre facture { $company } est disponible
invoice-ready-heading = Votre facture est disponible

## Receipt (kind invoice_paid)
invoice-paid-subject = Votre reçu { $company }
invoice-paid-heading = Merci pour votre paiement !

## Quote (kind quote_ready)
quote-ready-subject = Devis { $number } de { $company }
quote-ready-heading = Vous avez reçu un devis

## Credit note (kind credit_note_issued)
credit-note-issued-subject = Votre avoir { $number } de { $company }
credit-note-issued-heading = Un avoir a été émis

## Payment failed (kind payment_failed)
payment-failed-subject = Le paiement de la facture { $number } n'a pas pu être traité
payment-failed-heading = Votre paiement n'a pas abouti
payment-failed-intro = Veuillez vérifier votre moyen de paiement et réessayer le paiement via le lien ci-dessous.

## Authentication required (kind authentication_required, 3-D Secure)
authentication-required-subject = Confirmez votre paiement pour la facture { $number }
authentication-required-heading = Votre banque demande une confirmation
authentication-required-intro = Votre banque demande une confirmation avant de prélever { $amount }. Cela ne prend que quelques secondes.

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Numéro de facture
invoice-date = Date d'émission
amount-due = Montant dû
amount-paid = Montant payé
credit-note-number = Numéro d'avoir
amount-credited = Montant crédité
quote-number = Numéro de devis
valid-until = Valable jusqu'au
# $date = a long localized date, under the headline amount.
due-on = Échéance le { $date }

## Status badges, next to the headline amount
status-paid = Payé
status-payment-failed = Échec du paiement
status-action-required = Action requise

## Summary headline ($plan = plan name; the plain line when the invoice has none)
invoice-ready-plan-period = Cette facture couvre votre abonnement { $plan } du { $start } au { $end }.
invoice-ready-plan = Cette facture concerne votre abonnement { $plan }.
invoice-paid-plan-period = Ce paiement couvre votre abonnement { $plan } du { $start } au { $end }.
invoice-paid-plan = Ce paiement concerne votre abonnement { $plan }.

## Buttons
view-and-pay = Voir et payer
pay-invoice = Payer la facture
confirm-payment = Confirmer le paiement
update-payment-method = Mettre à jour le moyen de paiement
view-credit-note = Voir l'avoir
view-and-sign-quote = Voir et signer le devis

## Layout
already-paid-disregard = Si vous avez déjà réglé cette facture, veuillez ignorer cet email.
powered-by = Propulsé par Meteroid
test-mode-notice = Email de test : envoyé depuis un environnement hors production. Aucun paiement réel ne sera prélevé.

## Seeded default reminder sequence: steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
reminder-default-before-subject = La facture { "{{invoice_number}}" } arrive à échéance le { "{{due_date}}" }
reminder-default-before-body =
    Bonjour { "{{customer_name}}" },

    Petit rappel : la facture { "{{invoice_number}}" } de { "{{amount_due}}" } arrive à échéance le { "{{due_date}}" }.

reminder-default-due-subject = La facture { "{{invoice_number}}" } arrive à échéance aujourd'hui
reminder-default-due-body =
    Bonjour { "{{customer_name}}" },

    La facture { "{{invoice_number}}" } de { "{{amount_due}}" } arrive à échéance aujourd'hui. Si vous l'avez déjà réglée, merci d'ignorer cet email.

reminder-default-overdue-subject = Facture { "{{invoice_number}}" } en retard de { "{{days_overdue}}" } jours
reminder-default-overdue-body =
    Bonjour { "{{customer_name}}" },

    Nous n'avons pas reçu le paiement de la facture { "{{invoice_number}}" } ({ "{{amount_due}}" }), échue le { "{{due_date}}" }.

reminder-default-final-subject = Dernier rappel : facture { "{{invoice_number}}" }
reminder-default-final-body =
    Bonjour { "{{customer_name}}" },

    La facture { "{{invoice_number}}" } est en retard de { "{{days_overdue}}" } jours. Merci de régler { "{{amount_due}}" } sous 7 jours pour éviter toute interruption de service.

## Solde prépayé (types wallet_balance_low, auto_top_up_failed). $threshold et $amount sont des montants formatés.
wallet-balance-low-subject = Votre solde prépayé { $company } est bas
wallet-balance-low-heading = Votre solde prépayé est bas
wallet-balance-low-intro = Votre solde disponible est inférieur à { $threshold }. Ajoutez du crédit pour continuer à couvrir votre consommation.
auto-top-up-failed-subject = Votre recharge automatique { $company } n'a pas pu être prélevée
auto-top-up-failed-heading = Votre recharge automatique n'a pas pu être prélevée
auto-top-up-failed-intro = Nous n'avons pas pu prélever { $amount } pour recharger votre solde. Veuillez recharger votre solde manuellement et vérifier votre moyen de paiement.
auto-top-up-suspended-subject = Les recharges automatiques de votre solde { $company } sont suspendues
auto-top-up-suspended-heading = Les recharges automatiques sont suspendues
auto-top-up-suspended-intro = Votre recharge automatique de { $amount } est suspendue à la suite d'un incident de paiement. Vérifiez votre moyen de paiement et vos paramètres de recharge pour la réactiver.
wallet-balance-low-intro-contact = Votre solde disponible est inférieur à { $threshold }. Contactez { $company } pour ajouter du crédit.
auto-top-up-failed-intro-contact = Nous n'avons pas pu prélever { $amount } pour recharger votre solde. Veuillez vérifier votre moyen de paiement, ou contacter { $company } pour ajouter du crédit.
auto-top-up-suspended-intro-contact = Votre recharge automatique de { $amount } est suspendue à la suite d'un incident de paiement. Contactez { $company } pour la réactiver.
auto-top-up-reversed-intro = Les recharges automatiques sont suspendues car { $amount } d'un paiement de recharge ont été remboursés ou contestés. Vérifiez votre moyen de paiement et vos paramètres de recharge pour les réactiver.
auto-top-up-reversed-intro-contact = Les recharges automatiques sont suspendues car { $amount } d'un paiement de recharge ont été remboursés ou contestés. Contactez { $company } pour les réactiver.
available-balance = Solde disponible
alert-threshold = Seuil d'alerte
top-up-amount = Montant de la recharge
reversed-payment-amount = Remboursé ou contesté
status-low-balance = Solde bas
status-paused = Suspendue

## Budget (types budget_threshold, budget_exceeded). $level est un pourcentage, $budget un montant formaté.
budget-threshold-subject = Votre consommation { $company } a atteint { $level } de votre budget
budget-threshold-heading = Votre consommation a atteint { $level } de votre budget
budget-threshold-intro = Votre consommation de cette période a atteint { $level } de votre budget de { $budget }. Elle inclut l'usage pas encore facturé.
budget-exceeded-subject = Votre consommation { $company } a atteint votre budget
budget-exceeded-heading = Votre consommation a atteint votre budget
budget-exceeded-intro = Votre consommation de cette période a atteint votre budget de { $budget }. Elle inclut l'usage pas encore facturé.
budget-notify-only = Ce budget ne fait que vous avertir. Il ne limite pas votre accès.
budget-stops-usage-ahead = Une fois votre consommation arrivée à ce budget, l'accès à l'usage qu'il compte peut être limité jusqu'à la prochaine période de facturation. L'usage enregistré d'ici là reste facturé.
budget-stops-usage-now = Votre consommation a atteint ce budget : l'accès à l'usage qu'il compte peut être limité jusqu'au début de la prochaine période de facturation ou jusqu'à ce que le budget soit relevé. L'usage enregistré d'ici là reste facturé.
budget-stops-usage-ahead-open = Une fois votre consommation arrivée à ce budget, l'accès à l'usage qu'il compte peut être limité jusqu'à ce que le budget soit relevé. L'usage enregistré d'ici là reste facturé.
budget-stops-usage-now-open = Votre consommation a atteint ce budget : l'accès à l'usage qu'il compte peut être limité jusqu'à ce que le budget soit relevé. L'usage enregistré d'ici là reste facturé.
budget-spend = Consommation de la période
budget-amount = Budget
notification-level = Niveau de notification
period-start = Début de la période
period-end = Fin de la période
status-budget-alert = Alerte budget
status-budget-reached = Budget atteint
