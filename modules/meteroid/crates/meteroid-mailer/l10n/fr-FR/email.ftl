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

## Quote signature code (one-time code before signing)
quote-signature-code-subject = Votre code pour signer le devis { $number }
quote-signature-code-heading = Votre code de signature
quote-signature-code-intro = Saisissez ce code pour signer le devis { $number } de { $company }. Il expire dans { $minutes } minutes.
quote-signature-code-ignore = Si vous n'avez pas essayé de signer ce devis, ignorez cet email.

## Quote discussion (a message or a change request)
quote-message-subject = Nouveau message sur le devis { $number }
quote-changes-requested-subject = Modifications demandées sur le devis { $number }
quote-message-heading = { $author } a envoyé un message à propos du devis { $number }
quote-changes-requested-heading = { $author } demande des modifications du devis { $number }
view-quote = Voir le devis
quote-customer = Client

## Quote accepted (kind quote_accepted), and its outcome to the team
quote-accepted-subject = Le devis { $number } de { $company } est accepté
quote-accepted-heading = Votre devis est accepté
quote-seller = De
quote-accepted-on = Accepté le
view-signed-quote = Voir le devis signé
quote-outcome-accepted-subject = { $customer } a accepté le devis { $number }
quote-outcome-declined-subject = { $customer } a refusé le devis { $number }

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

## Moyen de paiement (type payment_method_requested : la facturation a changé de prestataire)
payment-method-requested-subject = Action requise : ajoutez un moyen de paiement pour votre abonnement { $company }
payment-method-requested-heading = Ajoutez un moyen de paiement pour conserver votre abonnement
payment-method-requested-intro = { $company } facture désormais votre abonnement directement, et non plus via { $provider }. Votre offre et votre prix restent les mêmes.
payment-method-requested-by = Merci d'ajouter un moyen de paiement avant le { $date } pour que votre prochain paiement aboutisse.
payment-method-requested-anytime = Merci d'ajouter un moyen de paiement pour que votre prochain paiement aboutisse.
payment-method-requested-hedge = Si vous en avez déjà ajouté un, vous pouvez ignorer cet e-mail.
payment-method-requested-heading-by = Ajoutez un moyen de paiement avant le { $date }
payment-method-moving-intro = { $company } transfère la facturation de votre abonnement de { $provider } vers { $company } directement le { $date }. Votre offre et votre prix restent les mêmes.
payment-method-moving-kept-cards = { $provider } ne peut pas transférer les cartes enregistrées : nous avons besoin que vous en ajoutiez une. Vos factures seront émises par { $company }, qui calcule aussi la taxe.
payment-method-moving-no-reuse = Nous ne pouvons pas réutiliser la carte enregistrée chez { $provider } : nous avons besoin que vous en confirmiez une.
payment-method-moving-paypal = { $provider } ne peut pas transférer les paiements PayPal : nous avons besoin que vous ajoutiez un moyen de paiement.
payment-method-moving-invoice = Vous payez aujourd'hui par facture. Ajoutez un moyen de paiement pour que vos paiements soient prélevés automatiquement.
payment-method-link-hedge = Ce lien vous connecte à votre portail de facturation { $company }. S'il a expiré, vous pouvez en demander un nouveau depuis la page. Si vous avez déjà ajouté un moyen de paiement, vous pouvez ignorer cet e-mail.
payment-method-stop = Ne plus recevoir ces rappels
add-payment-method = Ajouter un moyen de paiement

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Numéro de facture
invoice-date = Date d'émission
amount-due = Montant dû
amount-paid = Montant payé
amount-carried = Reporté
credit-note-number = Numéro d'avoir
amount-credited = Montant crédité
quote-number = Numéro de devis
valid-until = Valable jusqu'au
# $date = a long localized date, under the headline amount.
due-on = Échéance le { $date }
collected-with-next-invoice = Prélevé avec votre prochaine facture

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
view-invoice = Voir la facture
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

## Résiliation (type subscription_cancelled). $plan est le nom de l'offre.
subscription-cancelled-subject = La résiliation de votre abonnement { $company } est confirmée
subscription-cancelled-heading = Résiliation confirmée
subscription-cancelled-intro = Votre abonnement { $plan } reste actif jusqu'au dernier jour de service indiqué ci-dessous.
cancellation-received = Résiliation reçue le
# $date = une date longue localisée, $time = HH:MM en UTC.
cancellation-received-at = { $date } à { $time } UTC
last-day-of-service = Dernier jour de service

## Carte qui expire (type payment_method_expiring). $method par ex. "Visa •••• 4242", $date une date longue localisée.
payment-method-expiring-subject = Votre moyen de paiement pour { $company } expire bientôt
payment-method-expiring-heading = Votre carte expire bientôt
payment-method-expiring-intro = Votre { $method } expire le { $date }. Mettez-la à jour pour que votre abonnement continue d'être renouvelé sans interruption.
payment-method-label = Moyen de paiement
payment-method-expires = Expire le
update-card = Mettre à jour le moyen de paiement

## Renewal reminder (kind renewal_reminder). $plan is the plan name, $date a long localized date.
renewal-reminder-renewing-subject = Votre abonnement { $company } est bientôt renouvelé
renewal-reminder-renewing-heading = Votre abonnement est bientôt renouvelé
renewal-reminder-renewing-intro = Votre abonnement { $plan } est renouvelé automatiquement le { $date }.
renewal-reminder-ending-subject = Votre abonnement { $company } prend bientôt fin
renewal-reminder-ending-heading = Votre abonnement prend bientôt fin
renewal-reminder-ending-intro = Votre abonnement { $plan } prend fin le { $date }. Contactez-nous pour le prolonger.
renewal-reminder-renews-on = Renouvelé le
renewal-reminder-ends-on = Prend fin le
renewal-reminder-next-term-ends = Fin de la prochaine période
renewal-reminder-price-change = Évolution du prix
# $percent = the uplift, as a decimal.
renewal-reminder-uplift = +{ $percent } %
renewal-reminder-opt-out-by = Résiliation possible jusqu'au
