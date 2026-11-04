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

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Número de factura
invoice-date = Fecha de emisión
amount-due = Importe a pagar
amount-paid = Importe pagado
credit-note-number = Número de factura rectificativa
amount-credited = Importe abonado
quote-number = Número de presupuesto
valid-until = Válido hasta
# $date = a long localized date, under the headline amount.
due-on = Vence el { $date }

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
pay-invoice = Pagar la factura
confirm-payment = Confirmar el pago
update-payment-method = Actualizar el método de pago y pagar
view-credit-note = Ver la factura rectificativa
view-and-sign-quote = Ver y firmar el presupuesto

## Layout
already-paid-disregard = Si ya ha realizado el pago, ignore este correo electrónico.
powered-by = Con la tecnología de Meteroid
test-mode-notice = Correo de prueba: enviado desde un entorno que no es de producción. No se realizará ningún cobro real.

## Seeded default reminder sequence (D30): steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
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
