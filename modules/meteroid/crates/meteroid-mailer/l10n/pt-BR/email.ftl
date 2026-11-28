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

## Quote signature code (one-time code before signing)
quote-signature-code-subject = Seu código para assinar o orçamento { $number }
quote-signature-code-heading = Seu código de assinatura
quote-signature-code-intro = Digite este código para assinar o orçamento { $number } de { $company }. Ele expira em { $minutes } minutos.
quote-signature-code-ignore = Se você não tentou assinar este orçamento, ignore este e-mail.

## Quote discussion (a message or a change request)
quote-message-subject = Nova mensagem sobre o orçamento { $number }
quote-changes-requested-subject = Alterações solicitadas no orçamento { $number }
quote-message-heading = { $author } enviou uma mensagem sobre o orçamento { $number }
quote-changes-requested-heading = { $author } solicitou alterações no orçamento { $number }
view-quote = Ver orçamento
quote-customer = Cliente

## Quote accepted (kind quote_accepted), and its outcome to the team
quote-accepted-subject = O orçamento { $number } de { $company } foi aceito
quote-accepted-heading = Seu orçamento foi aceito
quote-seller = De
quote-accepted-on = Aceito em
view-signed-quote = Ver orçamento assinado
quote-outcome-accepted-subject = { $customer } aceitou o orçamento { $number }
quote-outcome-declined-subject = { $customer } recusou o orçamento { $number }

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

## Forma de pagamento (tipo payment_method_requested: a cobrança mudou de provedor)
payment-method-requested-subject = Ação necessária: adicione uma forma de pagamento para sua assinatura { $company }
payment-method-requested-heading = Adicione uma forma de pagamento para manter sua assinatura
payment-method-requested-intro = A { $company } agora cobra sua assinatura diretamente, em vez de { $provider }. Seu plano e seu preço continuam os mesmos.
payment-method-requested-by = Adicione uma forma de pagamento antes de { $date } para que seu próximo pagamento seja processado.
payment-method-requested-anytime = Adicione uma forma de pagamento para que seu próximo pagamento seja processado.
payment-method-requested-hedge = Se você já adicionou uma, pode ignorar este e-mail.
payment-method-requested-heading-by = Adicione um método de pagamento antes de { $date }
payment-method-moving-intro = A { $company } vai transferir a cobrança da sua assinatura de { $provider } diretamente para a { $company } em { $date }. Seu plano e seu preço continuam os mesmos.
payment-method-moving-kept-cards = A { $provider } não consegue transferir os cartões salvos, então precisamos que você adicione um. Suas faturas passarão a ser emitidas pela { $company }, que também calcula o imposto.
payment-method-moving-no-reuse = Não podemos reutilizar o cartão salvo na { $provider }, então precisamos que você confirme um.
payment-method-moving-paypal = A { $provider } não pode transferir pagamentos do PayPal, então precisamos que você adicione uma forma de pagamento.
payment-method-moving-invoice = Hoje você paga por fatura. Adicione uma forma de pagamento para que seus pagamentos sejam cobrados automaticamente.
payment-method-link-hedge = Este link dá acesso ao seu portal de cobrança da { $company }. Se ele tiver expirado, você pode pedir um novo na página. Se você já adicionou um método de pagamento, pode ignorar este e-mail.
payment-method-stop = Parar de receber estes lembretes
add-payment-method = Adicionar forma de pagamento

## Summary box labels (no trailing colon: shown as a label/value table)
invoice-number = Número da fatura
invoice-date = Data de emissão
amount-due = Valor devido
amount-paid = Valor pago
amount-carried = Transferido
credit-note-number = Número da nota de crédito
amount-credited = Valor creditado
quote-number = Número do orçamento
valid-until = Válido até
# $date = a long localized date, under the headline amount.
due-on = Vence em { $date }
collected-with-next-invoice = Cobrado com a sua próxima fatura

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
view-invoice = Ver fatura
pay-invoice = Pagar fatura
confirm-payment = Confirmar pagamento
update-payment-method = Atualizar método de pagamento e pagar
view-credit-note = Ver nota de crédito
view-and-sign-quote = Ver e assinar orçamento

## Layout
already-paid-disregard = Se você já efetuou o pagamento, desconsidere este e-mail.
powered-by = Desenvolvido por Meteroid
test-mode-notice = Modo de teste: enviado de um ambiente que não é de produção.

## Seeded default reminder sequence: steps -3 / 0 / +7 / +14, copied into payment_reminder_step.content.
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

## Cancelamento (tipo subscription_cancelled). $plan é o nome do plano.
subscription-cancelled-subject = O cancelamento da sua assinatura { $company } está confirmado
subscription-cancelled-heading = Cancelamento confirmado
subscription-cancelled-intro = Sua assinatura { $plan } continua ativa até o último dia de serviço indicado abaixo.
cancellation-received = Cancelamento recebido em
# $date = uma data longa localizada, $time = HH:MM em UTC.
cancellation-received-at = { $date } às { $time } UTC
last-day-of-service = Último dia de serviço

## Cartão expirando (tipo payment_method_expiring). $method p. ex. "Visa •••• 4242", $date uma data longa localizada.
payment-method-expiring-subject = Seu método de pagamento para { $company } expira em breve
payment-method-expiring-heading = Seu cartão expira em breve
payment-method-expiring-intro = Seu { $method } expira em { $date }. Atualize-o para que sua assinatura continue sendo renovada sem interrupção.
payment-method-label = Método de pagamento
payment-method-expires = Expira em
update-card = Atualizar método de pagamento

## Renewal reminder (kind renewal_reminder). $plan is the plan name, $date a long localized date.
renewal-reminder-renewing-subject = Sua assinatura { $company } será renovada em breve
renewal-reminder-renewing-heading = Sua assinatura será renovada em breve
renewal-reminder-renewing-intro = Sua assinatura { $plan } será renovada automaticamente em { $date }.
renewal-reminder-ending-subject = Sua assinatura { $company } termina em breve
renewal-reminder-ending-heading = Sua assinatura termina em breve
renewal-reminder-ending-intro = Sua assinatura { $plan } termina em { $date }. Fale conosco para mantê-la.
renewal-reminder-renews-on = Renova em
renewal-reminder-ends-on = Termina em
renewal-reminder-next-term-ends = Fim do próximo período
renewal-reminder-price-change = Alteração de preço
# $percent = the uplift, as a decimal.
renewal-reminder-uplift = +{ $percent } %
renewal-reminder-opt-out-by = Cancelamento possível até

## Testes (tipos trial_ending, trial_started, trial_ended). $plan e $next são nomes de planos, $date uma data por extenso
## (o fim de um teste: o dia em que a cobrança começa), $amount um valor formatado, $interval por ex. "por mês".
trial-ending-subject = Seu teste da { $company } termina em { $date }
trial-ending-heading = Seu teste termina em breve
trial-ending-charged-intro = Seu teste do { $plan } termina em { $date }. Depois disso, { $amount } serão cobrados na sua forma de pagamento cadastrada.
trial-ending-charged-intro-no-amount = Seu teste do { $plan } termina em { $date }. Depois disso, a cobrança começa na sua forma de pagamento cadastrada.
trial-ending-invoiced-intro = Seu teste do { $plan } termina em { $date }. Depois disso, sua primeira fatura, de { $amount }, será emitida com suas condições de pagamento habituais.
trial-ending-invoiced-intro-no-amount = Seu teste do { $plan } termina em { $date }. Depois disso, a cobrança começa por fatura, com suas condições de pagamento habituais.
trial-ending-ends-intro = Seu teste do { $plan } termina em { $date }, e sua assinatura termina junto. Nada será cobrado.
trial-ending-switches-intro = Seu teste do { $plan } termina em { $date }. Depois disso, sua assinatura passa para o plano { $next }.
trial-ending-no-method-heading = Adicione uma forma de pagamento para manter sua assinatura
trial-ending-no-method-intro = Seu teste do { $plan } termina em { $date } e não há forma de pagamento cadastrada.
trial-ending-no-method-await = Sem uma, sua assinatura fica pausada até a primeira fatura ser paga.
trial-ending-no-method-cancel = Sem uma, sua assinatura termina em { $date }.
trial-ending-no-method-switch = Sem uma, sua assinatura passa para o plano { $next } em { $date }.
trial-ending-no-method-ask = Adicione uma antes de { $date } para continuar no { $plan }.
trial-manage-before = Você pode alterar ou cancelar sua assinatura no seu portal de cobrança antes de { $date }.
trial-first-payment = Primeiro pagamento
trial-first-invoice = Primeira fatura
trial-charged-on = Cobrado em { $date }
trial-invoiced-on = Faturado em { $date }
trial-plan = Plano
trial-starts-on = Início do teste
trial-ends-on = Fim do teste
trial-ended-on = Teste encerrado em
trial-next-plan = Próximo plano
trial-started-subject = Seu teste da { $company } começou
trial-started-heading = Seu teste começou
trial-started-intro = Seu teste do { $plan } termina em { $date }. Depois disso, { $amount } serão cobrados { $interval } na sua forma de pagamento cadastrada até você cancelar.
trial-started-intro-no-amount = Seu teste do { $plan } termina em { $date }. Depois disso, a cobrança começa na sua forma de pagamento cadastrada até você cancelar.
trial-after-trial = Após o teste
trial-charged-from = { $interval } a partir de { $date }
trial-per-month = por mês
trial-per-quarter = a cada 3 meses
trial-per-half-year = a cada 6 meses
trial-per-year = por ano
trial-ended-subject = Seu teste da { $company } terminou
trial-ended-heading = Seu teste terminou
trial-ended-cancelled-intro = Seu teste do { $plan } terminou em { $date }, e sua assinatura terminou junto. Nada foi cobrado.
trial-ended-switched-intro = Seu teste terminou em { $date }. Sua assinatura agora está no plano { $plan }.
trial-ended-awaiting-payment-intro = Seu teste do { $plan } terminou em { $date }. Adicione uma forma de pagamento e pague sua primeira fatura para continuar sua assinatura.
manage-subscription = Gerenciar assinatura
view-plans = Ver planos
