# Correos a clientes — textos fijos y el contenido predeterminado de los recordatorios.
# $company = nombre comercial de la entidad, $number = número del documento, $amount = importe formateado, $legal_name = razón social de la entidad.

## Invoice ready (kind invoice_ready)
invoice-ready-subject = Su factura de { $company } está disponible
invoice-ready-heading = Su factura está disponible

## Receipt (kind invoice_paid)
invoice-paid-subject = Su recibo de { $company }
invoice-paid-heading = ¡Gracias por su pago!

## Quote (kind quote_ready)
quote-ready-subject = Presupuesto { $number } de { $company }
quote-ready-heading = Ha recibido un presupuesto

## Quote signature code (one-time code before signing)
quote-signature-code-subject = Su código para firmar el presupuesto { $number }
quote-signature-code-heading = Su código de firma
quote-signature-code-intro = Introduzca este código para firmar el presupuesto { $number } de { $company }. Caduca en { $minutes } minutos.
quote-signature-code-ignore = Si no ha intentado firmar este presupuesto, ignore este correo.

## Quote discussion (a message or a change request)
quote-message-subject = Nuevo mensaje sobre el presupuesto { $number }
quote-changes-requested-subject = Cambios solicitados en el presupuesto { $number }
quote-message-heading = { $author } ha enviado un mensaje sobre el presupuesto { $number }
quote-changes-requested-heading = { $author } solicita cambios en el presupuesto { $number }
view-quote = Ver presupuesto
quote-customer = Cliente

## Quote accepted (kind quote_accepted), and its outcome to the team
quote-accepted-subject = El presupuesto { $number } de { $company } está aceptado
quote-accepted-heading = Su presupuesto está aceptado
quote-seller = De
quote-accepted-on = Aceptado el
view-signed-quote = Ver presupuesto firmado
quote-outcome-accepted-subject = { $customer } aceptó el presupuesto { $number }
quote-outcome-declined-subject = { $customer } rechazó el presupuesto { $number }

## Credit note (kind credit_note_issued)
credit-note-issued-subject = Su factura rectificativa { $number } de { $company }
credit-note-issued-heading = Se ha emitido una factura rectificativa

## Payment failed (kind payment_failed)
payment-failed-subject = No se ha podido procesar el pago de la factura { $number }
payment-failed-heading = No se ha podido procesar su pago
payment-failed-intro = Compruebe su método de pago y vuelva a intentar el pago mediante el enlace siguiente.

## Authentication required (kind authentication_required, 3-D Secure)
authentication-required-subject = Confirme su pago de la factura { $number }
authentication-required-heading = Su banco necesita que confirme este pago
authentication-required-intro = Su banco solicita una confirmación antes de cobrar { $amount }. Solo le llevará unos segundos.

## Método de pago (tipo payment_method_requested: la facturación cambió de proveedor)
payment-method-requested-subject = Acción necesaria: añada un método de pago para su suscripción a { $company }
payment-method-requested-heading = Añada un método de pago para conservar su suscripción
payment-method-requested-intro = { $company } factura ahora su suscripción directamente en lugar de { $provider }. Su plan y su precio no cambian.
payment-method-requested-by = Añada un método de pago antes del { $date } para que su próximo pago se realice correctamente.
payment-method-requested-anytime = Añada un método de pago para que su próximo pago se realice correctamente.
payment-method-requested-hedge = Si ya ha añadido uno, puede ignorar este correo.
payment-method-requested-heading-by = Añada un método de pago antes del { $date }
payment-method-moving-intro = { $company } traslada la facturación de su suscripción de { $provider } a { $company } directamente el { $date }. Su plan y su precio no cambian.
payment-method-moving-kept-cards = { $provider } no puede transferir las tarjetas guardadas, así que necesitamos que añada una. Sus facturas las emitirá { $company }, que también calcula el impuesto.
payment-method-moving-no-reuse = No podemos reutilizar la tarjeta guardada en { $provider }, así que necesitamos que confirme una.
payment-method-moving-paypal = { $provider } no puede transferir los pagos de PayPal, así que necesitamos que añada un método de pago.
payment-method-moving-invoice = Hoy paga por factura. Añada un método de pago para que sus pagos se cobren automáticamente.
payment-method-link-hedge = Este enlace le da acceso a su portal de facturación de { $company }. Si ha caducado, puede solicitar uno nuevo desde la página. Si ya ha añadido un método de pago, puede ignorar este correo.
payment-method-stop = Dejar de recibir estos recordatorios
add-payment-method = Añadir un método de pago

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Número de factura
invoice-date = Fecha de emisión
amount-due = Importe a pagar
amount-paid = Importe pagado
amount-carried = Trasladado
credit-note-number = Número de factura rectificativa
amount-credited = Importe abonado
quote-number = Número de presupuesto
valid-until = Válido hasta
# $date = a long localized date, under the headline amount.
due-on = Vence el { $date }
collected-with-next-invoice = Se cobrará con su próxima factura

## Status badges, next to the headline amount
status-paid = Pagado
status-payment-failed = Pago fallido
status-action-required = Acción requerida

## Summary headline ($plan = plan name; the plain line when the invoice has none)
invoice-ready-plan-period = Esta factura cubre su suscripción { $plan } del { $start } al { $end }.
invoice-ready-plan = Esta factura corresponde a su suscripción { $plan }.
invoice-paid-plan-period = Este pago cubre su suscripción { $plan } del { $start } al { $end }.
invoice-paid-plan = Este pago corresponde a su suscripción { $plan }.

## Buttons
view-and-pay = Ver y pagar
view-invoice = Ver factura
pay-invoice = Pagar la factura
confirm-payment = Confirmar el pago
update-payment-method = Actualizar el método de pago y pagar
view-credit-note = Ver la factura rectificativa
view-and-sign-quote = Ver y firmar el presupuesto

## Layout
already-paid-disregard = Si ya ha realizado el pago, ignore este correo electrónico.
powered-by = Con la tecnología de Meteroid
test-mode-notice = Modo de prueba: enviado desde un entorno que no es de producción.

## Seeded default reminder sequence: steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
reminder-default-before-subject = La factura { "{{invoice_number}}" } vence el { "{{due_date}}" }
reminder-default-before-body =
    Hola { "{{customer_name}}" },

    Le recordamos que la factura { "{{invoice_number}}" } por { "{{amount_due}}" } vence el { "{{due_date}}" }.

reminder-default-due-subject = La factura { "{{invoice_number}}" } vence hoy
reminder-default-due-body =
    Hola { "{{customer_name}}" },

    La factura { "{{invoice_number}}" } por { "{{amount_due}}" } vence hoy. Si ya ha realizado el pago, le damos las gracias y le rogamos que ignore este correo electrónico.

reminder-default-overdue-subject = La factura { "{{invoice_number}}" } lleva { "{{days_overdue}}" } días vencida
reminder-default-overdue-body =
    Hola { "{{customer_name}}" },

    No hemos recibido el pago de la factura { "{{invoice_number}}" } ({ "{{amount_due}}" }), que venció el { "{{due_date}}" }.

reminder-default-final-subject = Último aviso: factura { "{{invoice_number}}" }
reminder-default-final-body =
    Hola { "{{customer_name}}" },

    La factura { "{{invoice_number}}" } lleva ya { "{{days_overdue}}" } días vencida. Le rogamos que abone { "{{amount_due}}" } en un plazo de 7 días para evitar cualquier interrupción del servicio.

## Saldo prepago (tipos wallet_balance_low, auto_top_up_failed). $threshold y $amount son importes formateados.
wallet-balance-low-subject = Su saldo prepago de { $company } es bajo
wallet-balance-low-heading = Su saldo prepago es bajo
wallet-balance-low-intro = Su saldo disponible es inferior a { $threshold }. Añada crédito para seguir cubriendo su consumo.
auto-top-up-failed-subject = No se ha podido cobrar su recarga automática de { $company }
auto-top-up-failed-heading = No se ha podido cobrar su recarga automática
auto-top-up-failed-intro = No hemos podido cobrar { $amount } para recargar su saldo. Recargue su saldo manualmente y compruebe su método de pago.
auto-top-up-suspended-subject = Las recargas automáticas de su saldo de { $company } están en pausa
auto-top-up-suspended-heading = Las recargas automáticas están en pausa
auto-top-up-suspended-intro = Su recarga automática de { $amount } está en pausa tras un problema con el pago. Revise su método de pago y la configuración de recarga para reactivarla.
wallet-balance-low-intro-contact = Su saldo disponible es inferior a { $threshold }. Póngase en contacto con { $company } para añadir crédito.
auto-top-up-failed-intro-contact = No hemos podido cobrar { $amount } para recargar su saldo. Compruebe su método de pago o póngase en contacto con { $company } para añadir crédito.
auto-top-up-suspended-intro-contact = Su recarga automática de { $amount } está en pausa tras un problema con el pago. Póngase en contacto con { $company } para reactivarla.
auto-top-up-reversed-intro = Las recargas automáticas están en pausa porque se reembolsaron o impugnaron { $amount } de un pago de recarga. Revise su método de pago y la configuración de recarga para reactivarlas.
auto-top-up-reversed-intro-contact = Las recargas automáticas están en pausa porque se reembolsaron o impugnaron { $amount } de un pago de recarga. Póngase en contacto con { $company } para reactivarlas.
available-balance = Saldo disponible
alert-threshold = Umbral de alerta
top-up-amount = Importe de la recarga
reversed-payment-amount = Reembolsado o impugnado
status-low-balance = Saldo bajo
status-paused = En pausa

## Presupuesto (tipos budget_threshold, budget_exceeded). $level es un porcentaje, $budget un importe formateado.
budget-threshold-subject = Su gasto en { $company } ha alcanzado el { $level } de su presupuesto
budget-threshold-heading = Su gasto ha alcanzado el { $level } de su presupuesto
budget-threshold-intro = Su gasto de este periodo ha alcanzado el { $level } de su presupuesto de { $budget }. Incluye el consumo aún no facturado.
budget-exceeded-subject = Su gasto en { $company } ha alcanzado su presupuesto
budget-exceeded-heading = Su gasto ha alcanzado su presupuesto
budget-exceeded-intro = Su gasto de este periodo ha alcanzado su presupuesto de { $budget }. Incluye el consumo aún no facturado.
budget-notify-only = Este presupuesto solo le avisa. No limita su acceso.
budget-stops-usage-ahead = Cuando su gasto alcance este presupuesto, el acceso al consumo que computa puede quedar limitado hasta el próximo periodo de facturación. El consumo registrado hasta entonces se sigue facturando.
budget-stops-usage-now = Su gasto ha alcanzado este presupuesto: el acceso al consumo que computa puede quedar limitado hasta que empiece el próximo periodo de facturación o se aumente el presupuesto. El consumo registrado hasta entonces se sigue facturando.
budget-stops-usage-ahead-open = Cuando su gasto alcance este presupuesto, el acceso al consumo que computa puede quedar limitado hasta que se aumente el presupuesto. El consumo registrado hasta entonces se sigue facturando.
budget-stops-usage-now-open = Su gasto ha alcanzado este presupuesto: el acceso al consumo que computa puede quedar limitado hasta que se aumente el presupuesto. El consumo registrado hasta entonces se sigue facturando.
budget-spend = Gasto del periodo
budget-amount = Presupuesto
notification-level = Nivel de aviso
period-start = Inicio del periodo
period-end = Fin del periodo
status-budget-alert = Aviso de presupuesto
status-budget-reached = Presupuesto alcanzado

## Cancelación (tipo subscription_cancelled). $plan es el nombre del plan.
subscription-cancelled-subject = La cancelación de su suscripción de { $company } está confirmada
subscription-cancelled-heading = Cancelación confirmada
subscription-cancelled-intro = Su suscripción { $plan } sigue activa hasta el último día de servicio indicado abajo.
cancellation-received = Cancelación recibida el
# $date = una fecha larga localizada, $time = HH:MM en UTC.
cancellation-received-at = { $date } a las { $time } UTC
last-day-of-service = Último día de servicio

## Tarjeta que caduca (tipo payment_method_expiring). $method p. ej. "Visa •••• 4242", $date una fecha larga localizada.
payment-method-expiring-subject = Tu método de pago para { $company } caduca pronto
payment-method-expiring-heading = Tu tarjeta caduca pronto
payment-method-expiring-intro = Tu { $method } caduca el { $date }. Actualízala para que tu suscripción se siga renovando sin interrupciones.
payment-method-label = Método de pago
payment-method-expires = Caduca
update-card = Actualizar el método de pago

## Renewal reminder (kind renewal_reminder). $plan is the plan name, $date a long localized date.
renewal-reminder-renewing-subject = Su suscripción de { $company } se renueva pronto
renewal-reminder-renewing-heading = Su suscripción se renueva pronto
renewal-reminder-renewing-intro = Su suscripción { $plan } se renueva automáticamente el { $date }.
renewal-reminder-ending-subject = Su suscripción de { $company } termina pronto
renewal-reminder-ending-heading = Su suscripción termina pronto
renewal-reminder-ending-intro = Su suscripción { $plan } termina el { $date }. Contáctenos para mantenerla.
renewal-reminder-renews-on = Se renueva el
renewal-reminder-ends-on = Termina el
renewal-reminder-next-term-ends = Fin del próximo periodo
renewal-reminder-price-change = Cambio de precio
# $percent = the uplift, as a decimal.
renewal-reminder-uplift = +{ $percent } %
renewal-reminder-opt-out-by = Cancelación posible hasta
