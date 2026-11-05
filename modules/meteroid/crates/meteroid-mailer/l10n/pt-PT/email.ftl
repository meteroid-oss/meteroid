# Emails a clientes — textos fixos e conteúdo predefinido dos lembretes.
# $company = nome comercial da entidade, $number = número do documento, $amount = montante formatado, $legal_name = denominação social da entidade.

## Invoice ready (kind invoice_ready)
invoice-ready-subject = A sua fatura { $company } está disponível
invoice-ready-heading = A sua fatura está disponível

## Receipt (kind invoice_paid)
invoice-paid-subject = O seu recibo { $company }
invoice-paid-heading = Obrigado pelo seu pagamento!

## Quote (kind quote_ready)
quote-ready-subject = Orçamento { $number } de { $company }
quote-ready-heading = Recebeu um orçamento

## Credit note (kind credit_note_issued)
credit-note-issued-subject = A sua nota de crédito { $number } de { $company }
credit-note-issued-heading = Foi emitida uma nota de crédito

## Payment failed (kind payment_failed)
payment-failed-subject = Não foi possível processar o pagamento da fatura { $number }
payment-failed-heading = Não foi possível processar o seu pagamento
payment-failed-intro = Verifique o seu método de pagamento e tente novamente através da ligação abaixo.

## Authentication required (kind authentication_required, 3-D Secure)
authentication-required-subject = Confirme o seu pagamento da fatura { $number }
authentication-required-heading = O seu banco pede-lhe que confirme este pagamento
authentication-required-intro = O seu banco pede uma confirmação antes de debitar { $amount }. Demora apenas alguns segundos.

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Número da fatura
invoice-date = Data de emissão
amount-due = Montante devido
amount-paid = Montante pago
credit-note-number = Número da nota de crédito
amount-credited = Montante creditado
quote-number = Número do orçamento
valid-until = Válido até
# $date = a long localized date, under the headline amount.
due-on = Vence a { $date }

## Status badges, next to the headline amount
status-paid = Pago
status-payment-failed = Pagamento falhado
status-action-required = Ação necessária

## Summary headline ($plan = plan name; the plain line when the invoice has none)
invoice-ready-plan-period = Esta fatura abrange a sua subscrição { $plan } de { $start } a { $end }.
invoice-ready-plan = Esta fatura refere-se à sua subscrição { $plan }.
invoice-paid-plan-period = Este pagamento abrange a sua subscrição { $plan } de { $start } a { $end }.
invoice-paid-plan = Este pagamento refere-se à sua subscrição { $plan }.

## Buttons
view-and-pay = Ver e pagar
pay-invoice = Pagar a fatura
confirm-payment = Confirmar o pagamento
update-payment-method = Atualizar o método de pagamento e pagar
view-credit-note = Ver a nota de crédito
view-and-sign-quote = Ver e assinar o orçamento

## Layout
already-paid-disregard = Se já efetuou o pagamento, ignore este email.
powered-by = Desenvolvido por Meteroid
test-mode-notice = Email de teste: enviado a partir de um ambiente que não é de produção. Não será cobrado nenhum pagamento real.

## Seeded default reminder sequence: steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
reminder-default-before-subject = A fatura { "{{invoice_number}}" } vence a { "{{due_date}}" }
reminder-default-before-body =
    Olá { "{{customer_name}}" },

    Lembramos que a fatura { "{{invoice_number}}" } no montante de { "{{amount_due}}" } vence a { "{{due_date}}" }.

reminder-default-due-subject = A fatura { "{{invoice_number}}" } vence hoje
reminder-default-due-body =
    Olá { "{{customer_name}}" },

    A fatura { "{{invoice_number}}" } no montante de { "{{amount_due}}" } vence hoje. Se já efetuou o pagamento, agradecemos e pedimos que ignore este email.

reminder-default-overdue-subject = A fatura { "{{invoice_number}}" } está vencida há { "{{days_overdue}}" } dias
reminder-default-overdue-body =
    Olá { "{{customer_name}}" },

    Não recebemos o pagamento da fatura { "{{invoice_number}}" } ({ "{{amount_due}}" }), vencida a { "{{due_date}}" }.

reminder-default-final-subject = Último aviso: fatura { "{{invoice_number}}" }
reminder-default-final-body =
    Olá { "{{customer_name}}" },

    A fatura { "{{invoice_number}}" } está vencida há { "{{days_overdue}}" } dias. Pedimos que liquide { "{{amount_due}}" } no prazo de 7 dias para evitar qualquer interrupção do serviço.

## Saldo pré-pago (tipos wallet_balance_low, auto_top_up_failed). $threshold e $amount são montantes formatados.
wallet-balance-low-subject = O seu saldo pré-pago { $company } está baixo
wallet-balance-low-heading = O seu saldo pré-pago está baixo
wallet-balance-low-intro = O seu saldo disponível é inferior a { $threshold }. Adicione crédito para continuar a cobrir a sua utilização.
auto-top-up-failed-subject = Não foi possível debitar o seu carregamento automático { $company }
auto-top-up-failed-heading = Não foi possível debitar o seu carregamento automático
auto-top-up-failed-intro = Não conseguimos debitar { $amount } para carregar o seu saldo. Carregue o seu saldo manualmente e verifique o seu método de pagamento.
auto-top-up-suspended-subject = Os carregamentos automáticos do seu saldo { $company } estão em pausa
auto-top-up-suspended-heading = Os carregamentos automáticos estão em pausa
auto-top-up-suspended-intro = O seu carregamento automático de { $amount } está em pausa após um problema de pagamento. Verifique o seu método de pagamento e as definições de carregamento para o retomar.
wallet-balance-low-intro-contact = O seu saldo disponível é inferior a { $threshold }. Contacte { $company } para adicionar crédito.
auto-top-up-failed-intro-contact = Não conseguimos debitar { $amount } para carregar o seu saldo. Verifique o seu método de pagamento ou contacte { $company } para adicionar crédito.
auto-top-up-suspended-intro-contact = O seu carregamento automático de { $amount } está em pausa após um problema de pagamento. Contacte { $company } para o retomar.
auto-top-up-reversed-intro = Os carregamentos automáticos estão em pausa porque { $amount } de um pagamento de carregamento foram reembolsados ou contestados. Verifique o seu método de pagamento e as definições de carregamento para os retomar.
auto-top-up-reversed-intro-contact = Os carregamentos automáticos estão em pausa porque { $amount } de um pagamento de carregamento foram reembolsados ou contestados. Contacte { $company } para os retomar.
available-balance = Saldo disponível
alert-threshold = Limite de alerta
top-up-amount = Montante do carregamento
reversed-payment-amount = Reembolsado ou contestado
status-low-balance = Saldo baixo
status-paused = Em pausa

## Orçamento (tipos budget_threshold, budget_exceeded). $level é uma percentagem, $budget um montante formatado.
budget-threshold-subject = A sua despesa { $company } atingiu { $level } do orçamento
budget-threshold-heading = A sua despesa atingiu { $level } do orçamento
budget-threshold-intro = A sua despesa neste período atingiu { $level } do seu orçamento de { $budget }. Inclui a utilização ainda não faturada.
budget-exceeded-subject = A sua despesa { $company } atingiu o orçamento
budget-exceeded-heading = A sua despesa atingiu o orçamento
budget-exceeded-intro = A sua despesa neste período atingiu o seu orçamento de { $budget }. Inclui a utilização ainda não faturada.
budget-notify-only = Este orçamento apenas o avisa. Não limita o seu acesso.
budget-stops-usage-ahead = Quando a sua despesa atingir este orçamento, o acesso à utilização que contabiliza pode ser limitado até ao próximo período de faturação. A utilização registada até lá continua a ser cobrada.
budget-stops-usage-now = A sua despesa atingiu este orçamento: o acesso à utilização que contabiliza pode ser limitado até ao início do próximo período de faturação ou até o orçamento ser aumentado. A utilização registada até lá continua a ser cobrada.
budget-stops-usage-ahead-open = Quando a sua despesa atingir este orçamento, o acesso à utilização que contabiliza pode ser limitado até o orçamento ser aumentado. A utilização registada até lá continua a ser cobrada.
budget-stops-usage-now-open = A sua despesa atingiu este orçamento: o acesso à utilização que contabiliza pode ser limitado até o orçamento ser aumentado. A utilização registada até lá continua a ser cobrada.
budget-spend = Despesa do período
budget-amount = Orçamento
notification-level = Nível de aviso
period-start = Início do período
period-end = Fim do período
status-budget-alert = Alerta de orçamento
status-budget-reached = Orçamento atingido
