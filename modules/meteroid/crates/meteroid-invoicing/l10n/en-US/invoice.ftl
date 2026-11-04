invoice-title = Invoice
invoice-number = Invoice No.
company-logo-alt = Company Logo
issue-date = Issue Date
amount-due = Amount Due
due-date = Due Date
due-label = due
bill-from = Bill From
bill-to = Bill To
invoice-lines = Details
description = Description
quantity = Quantity
unit-price = Unit Price
prorated = prorated
credits-used = used
credits-covered = covered by credit balance
tax-rate = Tax Rate
tax = Tax
amount = Amount (excl. tax)
subtotal = Subtotal
total-due = Total Due
total = Total
credits-applied = Credits applied
prepaid-credit-tax-note = Tax on prepaid credit is charged when the credit is used.
legal-info = Legal Information
exchange-rate-info = Exchange rate on {$date}: {$equality} | Total converted: {$amount_converted}
payment-status = Payment Status
payment-status-paid = Paid
payment-status-partially-paid = Partially Paid
payment-status-unpaid = Unpaid
payment-method = Method
payment-date = Date
payment-amount = Amount
no-transactions = No payments received
payment-info-title = Payment Information
payment-terms-title = Payment Terms
payment-terms-text = Payment to be made within {$days} days
tax-info-title = Tax Information
tax-included-text = All prices are in {$currency} and include VAT unless otherwise stated
tax-included-generic = All prices are in {$currency} and include tax unless otherwise stated
tax-reverse-charge = Reverse charge
pay-online = Pay online
seller-vat-id = Seller VAT ID
customer-vat-id = Customer VAT ID
seller-tax-id = Seller Tax ID
customer-tax-id = Customer Tax ID
vat-exempt-notice = VAT-exempt items are not included in the tax calculation.
export-notice-fr = Article 259-1° of the French General Tax Code (CGI): supply of services to a taxable person established outside the European Union (Article 44 of Directive 2006/112/EC).
export-notice-be = Place of supply outside Belgium – supply of services to a taxable person established outside the European Union (Article 21, § 2, of the Belgian VAT Code; Article 44 of Directive 2006/112/EC).
export-notice-be-b2c = Place of supply outside Belgium – electronically supplied service to a non-taxable person established outside the European Union (Article 21bis, § 2, 9°, of the Belgian VAT Code; Article 58 of Directive 2006/112/EC).
export-notice-ch = Place of supply abroad (Article 8 para. 1 MWSTG/LTVA).
vat-not-applicable = VAT not applicable
export-notice-generic = Supply of services to a taxable person established outside the European Union (Article 44 of Directive 2006/112/EC).
export-notice-non-eu = Cross-border supply of services.
export-notice-fr-b2c = Article 259 B of the French General Tax Code (CGI): supply of services to a non-taxable person established outside the European Union.
export-notice-generic-b2c = Supply of services to a non-taxable person established outside the European Union (Articles 58 and 59 of Directive 2006/112/EC).
export-notice-no-vat = Cross-border supply of services.
no-vat-territory-notice = Article 294-1 of the French General Tax Code (CGI): supply made in French Guiana or Mayotte, where VAT is provisionally not applicable.
export-notice-it = Article 7-ter of Italian Presidential Decree No. 633/1972 (Article 44 of Directive 2006/112/EC).
vat-not-applicable-ch = Not taxable
seller-vat-id-ch = Supplier VAT no.
seller-vat-id-at = Seller VAT ID (UID)
export-notice-pt = Article 6(6)(a) of the Portuguese VAT Code (CIVA), a contrario: supply of services to a taxable person established outside the European Union (Article 44 of Directive 2006/112/EC).
export-notice-pt-b2c = Article 6(9)(h) of the Portuguese VAT Code (CIVA): electronically supplied service to a non-taxable person established outside the European Union (Article 58 of Directive 2006/112/EC).
seller-vat-id-pt = Seller VAT ID (NIF)
seller-tax-id-br = Seller CNPJ
reverse-charge-notice-generic = Tax to be accounted for by the recipient.
credit-note-reverse-charge-notice-generic = Tax to be accounted for by the recipient on the original invoice.
tax-not-applicable = Tax not applicable
vat-not-applicable-it = Transaction not subject to VAT
tax-reverse-charge-generic = Tax to be paid on reverse charge basis
vat-exempt-headline = VAT exempt
tax-exempt-headline = Tax exempt
tax-exempt-notice-generic = Tax-exempt items are not included in the tax calculation.
reverse-charge-notice = VAT to be accounted for by the customer (Article 196 of Directive 2006/112/EC).
company-registration = Company Registration Number
customer-registration = Customer Registration Number
discount = Discount
promotional-credit = Promotional credit
purchase-order = Purchase Order
no-tax-applied = No tax applied
# Tax-inclusive documents: the net total before the tax that was carved out of the
# quoted prices, and the suffix marking a tax row as already included in them.
total-excl-tax = Total excluding tax
tax-row-included = included
cancels-and-replaces = Cancels and replaces invoice {$invoice_number} dated {$invoice_date}

# Credit Note specific translations
credit-note-title = Credit Note
credit-note-number = Credit Note No.
related-invoice = Original Invoice
credit-to = Credit To
reason = Reason
refund = Refund
credit-to-balance = Credit to Balance
debt-cancellation = Debt Cancellation
amount-to-refund = Amount to Refund
credited-amount = Credited Amount
total-credit = Total Credit
related-invoice-value = {$invoice_number} dated {$invoice_date}
credit-note-reverse-charge-notice = VAT to be accounted for by the customer on the original invoice (Article 196 of Directive 2006/112/EC).

# Tax breakdown row names. The engine records what a resolved rate is (VAT, GST, ...);
# these word it in this language. tax-name-vat-ch is the Swiss-regime override,
# selected by the seller's jurisdiction like the other -ch keys, not by language.
tax-name-vat = VAT
tax-name-vat-ch = VAT
tax-name-gst = GST
tax-name-hst = HST
tax-name-pst = PST
tax-name-qst = QST
tax-name-sales-tax = Sales Tax

# A finalized invoice with nothing left to collect.
nothing-to-pay = Nothing to pay
# Credit note: the debt it cancelled, and the promotional credit it gave back.
debt-cancelled-amount = Debt Cancelled
promotional-credit-to-restore = Promotional credit to be restored
promotional-credit-restored = Promotional credit restored

# Credit-purchase lines the platform writes, worded at render time.
credit-purchase-wallet-top-up = Wallet top-up
credit-purchase-automatic-top-up = Automatic top-up
credit-purchase = Purchase credits
credit-purchase-currency-wallet = Prepaid credit added to the {$currency} wallet
credit-purchase-valid-months = Credit valid for {$months} months after payment
credit-purchase-units-valid-months = {$credits} {$unit}, valid for {$months} months after payment
