# E-mails para clientes — textos fixos e conteúdo padrão dos lembretes.
# $company = nome comercial da entidade, $number = número do documento, $amount = valor formatado, $legal_name = razão social da entidade.

## Invoice ready (kind invoice_ready)
invoice-ready-subject = Sua fatura { $company } está disponível
invoice-ready-heading = Sua fatura está disponível

## Receipt (kind invoice_paid)
invoice-paid-subject = Seu recibo { $company }
invoice-paid-heading = Obrigado pelo seu pagamento!

## Quote (kind quote_ready)
quote-ready-subject = Orçamento { $number } de { $company }
quote-ready-heading = Você recebeu um orçamento

## Credit note (kind credit_note_issued)
credit-note-issued-subject = Sua nota de crédito { $number } de { $company }
credit-note-issued-heading = Uma nota de crédito foi emitida

## Payment failed (kind payment_failed)
payment-failed-subject = Não foi possível processar o pagamento da fatura { $number }
payment-failed-heading = Não foi possível processar seu pagamento
payment-failed-intro = Verifique seu método de pagamento e tente novamente pelo link abaixo.

## Authentication required (kind authentication_required, 3-D Secure)
authentication-required-subject = Confirme seu pagamento da fatura { $number }
authentication-required-heading = Seu banco precisa que você confirme este pagamento
authentication-required-intro = Seu banco solicitou uma confirmação antes de cobrar { $amount }. Leva apenas alguns segundos.

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Número da fatura
invoice-date = Data de emissão
amount-due = Valor devido
amount-paid = Valor pago
credit-note-number = Número da nota de crédito
amount-credited = Valor creditado
quote-number = Número do orçamento
valid-until = Válido até
# $date = a long localized date, under the headline amount.
due-on = Vence em { $date }

## Status badges, next to the headline amount
status-paid = Pago
status-payment-failed = Falha no pagamento
status-action-required = Ação necessária

## Summary headline ($plan = plan name; the plain line when the invoice has none)
invoice-ready-plan-period = Esta fatura cobre sua assinatura { $plan } de { $start } a { $end }.
invoice-ready-plan = Esta fatura refere-se à sua assinatura { $plan }.
invoice-paid-plan-period = Este pagamento cobre sua assinatura { $plan } de { $start } a { $end }.
invoice-paid-plan = Este pagamento refere-se à sua assinatura { $plan }.

## Buttons
view-and-pay = Ver e pagar
pay-invoice = Pagar fatura
confirm-payment = Confirmar pagamento
update-payment-method = Atualizar método de pagamento e pagar
view-credit-note = Ver nota de crédito
view-and-sign-quote = Ver e assinar orçamento

## Layout
already-paid-disregard = Se você já efetuou o pagamento, desconsidere este e-mail.
powered-by = Desenvolvido por Meteroid
test-mode-notice = E-mail de teste: enviado de um ambiente que não é de produção. Nenhum pagamento real será cobrado.

## Seeded default reminder sequence (D30): steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
reminder-default-before-subject = A fatura { "{{invoice_number}}" } vence em { "{{due_date}}" }
reminder-default-before-body =
    Olá, { "{{customer_name}}" },

    Lembrete: a fatura { "{{invoice_number}}" } no valor de { "{{amount_due}}" } vence em { "{{due_date}}" }.

reminder-default-due-subject = A fatura { "{{invoice_number}}" } vence hoje
reminder-default-due-body =
    Olá, { "{{customer_name}}" },

    A fatura { "{{invoice_number}}" } no valor de { "{{amount_due}}" } vence hoje. Se você já efetuou o pagamento, agradecemos e pedimos que desconsidere este e-mail.

reminder-default-overdue-subject = A fatura { "{{invoice_number}}" } está vencida há { "{{days_overdue}}" } dias
reminder-default-overdue-body =
    Olá, { "{{customer_name}}" },

    Não recebemos o pagamento da fatura { "{{invoice_number}}" } ({ "{{amount_due}}" }), vencida em { "{{due_date}}" }.

reminder-default-final-subject = Último aviso: fatura { "{{invoice_number}}" }
reminder-default-final-body =
    Olá, { "{{customer_name}}" },

    A fatura { "{{invoice_number}}" } está vencida há { "{{days_overdue}}" } dias. Pedimos que efetue o pagamento de { "{{amount_due}}" } em até 7 dias para evitar qualquer interrupção do serviço.

## Saldo pré-pago (tipos wallet_balance_low, auto_top_up_failed). $threshold e $amount são valores formatados.
wallet-balance-low-subject = Seu saldo pré-pago { $company } está baixo
wallet-balance-low-heading = Seu saldo pré-pago está baixo
wallet-balance-low-intro = Seu saldo disponível está abaixo de { $threshold }. Adicione crédito para continuar cobrindo seu uso.
auto-top-up-failed-subject = Não foi possível cobrar sua recarga automática { $company }
auto-top-up-failed-heading = Não foi possível cobrar sua recarga automática
auto-top-up-failed-intro = Não conseguimos cobrar { $amount } para recarregar seu saldo. Recarregue seu saldo manualmente e verifique seu método de pagamento.
auto-top-up-suspended-subject = As recargas automáticas do seu saldo { $company } estão pausadas
auto-top-up-suspended-heading = As recargas automáticas estão pausadas
auto-top-up-suspended-intro = Sua recarga automática de { $amount } foi pausada após um problema no pagamento. Verifique seu método de pagamento e as configurações de recarga para retomá-la.
wallet-balance-low-intro-contact = Seu saldo disponível está abaixo de { $threshold }. Entre em contato com { $company } para adicionar crédito.
auto-top-up-failed-intro-contact = Não conseguimos cobrar { $amount } para recarregar seu saldo. Verifique seu método de pagamento ou entre em contato com { $company } para adicionar crédito.
auto-top-up-suspended-intro-contact = Sua recarga automática de { $amount } foi pausada após um problema no pagamento. Entre em contato com { $company } para retomá-la.
auto-top-up-reversed-intro = As recargas automáticas foram pausadas porque { $amount } de um pagamento de recarga foram reembolsados ou contestados. Verifique seu método de pagamento e as configurações de recarga para retomá-las.
auto-top-up-reversed-intro-contact = As recargas automáticas foram pausadas porque { $amount } de um pagamento de recarga foram reembolsados ou contestados. Entre em contato com { $company } para retomá-las.
available-balance = Saldo disponível
alert-threshold = Limite de alerta
top-up-amount = Valor da recarga
reversed-payment-amount = Reembolsado ou contestado
status-low-balance = Saldo baixo
status-paused = Pausada

## Orçamento (tipos budget_threshold, budget_exceeded). $level é uma porcentagem, $budget um valor formatado.
budget-threshold-subject = Seu gasto { $company } atingiu { $level } do orçamento
budget-threshold-heading = Seu gasto atingiu { $level } do orçamento
budget-threshold-intro = Seu gasto neste período atingiu { $level } do seu orçamento de { $budget }. Inclui o uso ainda não faturado.
budget-exceeded-subject = Seu gasto { $company } atingiu o orçamento
budget-exceeded-heading = Seu gasto atingiu o orçamento
budget-exceeded-intro = Seu gasto neste período atingiu o seu orçamento de { $budget }. Inclui o uso ainda não faturado.
budget-notify-only = Este orçamento apenas avisa você. Ele não limita seu acesso.
budget-stops-usage-ahead = Quando seu gasto atingir este orçamento, o acesso ao uso que ele contabiliza pode ser limitado até o próximo período de faturamento. O uso registrado até então continua sendo cobrado.
budget-stops-usage-now = Seu gasto atingiu este orçamento: o acesso ao uso que ele contabiliza pode ser limitado até o início do próximo período de faturamento ou até que o orçamento seja aumentado. O uso registrado até então continua sendo cobrado.
budget-stops-usage-ahead-open = Quando seu gasto atingir este orçamento, o acesso ao uso que ele contabiliza pode ser limitado até que o orçamento seja aumentado. O uso registrado até então continua sendo cobrado.
budget-stops-usage-now-open = Seu gasto atingiu este orçamento: o acesso ao uso que ele contabiliza pode ser limitado até que o orçamento seja aumentado. O uso registrado até então continua sendo cobrado.
budget-spend = Gasto do período
budget-amount = Orçamento
notification-level = Nível de aviso
period-start = Início do período
period-end = Fim do período
status-budget-alert = Alerta de orçamento
status-budget-reached = Orçamento atingido
