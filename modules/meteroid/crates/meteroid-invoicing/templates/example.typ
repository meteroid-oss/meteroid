#import "invoice.typ": invoice

// Define example data directly in the template
#let example_organization = (
  name: "Acme Inc.",
  logo_src: "logo.png",
  legal_number: "123456789",
  address: (
    line1: "123 Main Street",
    line2: none,
    city: "Paris",
    country: "France",
    state: none,
    zipcode: "75001",
  ),
  email: "contact@acme.com",
  tax_id: "FR123456789",
  footer_info: "Payment to be made within 14 days",
  footer_legal: "Acme Inc. is registered in France under company number 123456789. VAT ID: FR123456789. All prices are in EUR and include VAT where applicable. This invoice constitutes a request for payment in accordance with EU Directive 2006/112/EC. Payment terms: 14 days net. Late payment interest: 3% above the European Central Bank rate as per French Commercial Code L441-10.",
  currency_code: "EUR",
  exchange_rate: 1.08,
  accounting_currency_code: "USD",
)

#let example_customer = (
  name: "Client Corporation",
  legal_number: "987654321",
  address: (
    line1: "456 Client Avenue",
    line2: none,
    city: "New York",
    country: "USA",
    state: "NY",
    zipcode: "10001",
  ),
  email: "billing@clientcorp.com",
  tax_id: "US987654321",
)

// Example payment information
#let example_payment_info = (
  "Bank Name": "International Bank",
  "Account Holder": "Acme Inc.",
  "IBAN": "FR76 1234 5678 9012 3456 7890 123",
  "BIC/SWIFT": "INTLFRPP",
  "Reference": "INV-2025-001"
)

// Example transactions
#let example_transactions = (
  (
    method: "Card •••• 7726",
    date: "2025-03-15",
    amount: 1500.0,
  ),
  (
    method: "Bank Transfer",
    date: "2025-03-20",
    amount: 1080.0,
  ),
)

#let example_lines = (
  (
    name: "Consulting Services",
    description: "Technical consulting for Q1 2025",
    quantity: 15.0,
    unit_price: 50.0,
    vat_rate: 20.0,
    subtotal: 750.0,
    total: 750.0,
    start_date: "2025-03-01",
    end_date: "2025-03-31",
    sub_lines: (
      (
        name: "Frontend Development",
        quantity: 5.0,
        unit_price: 50.0,
        total: 250.0,
      ),
      (
        name: "Backend Development",
        quantity: 10.0,
        unit_price: 50.0,
        total: 500.0,
      ),
    ),
  ),
  (
    name: "Software License",
    description: "Annual license renewal",
    quantity: 1.0,
    unit_price: 250.0,
    vat_rate: 20.0,
    subtotal: 250.0,
    total: 250.0,
    start_date: "2025-03-01",
    end_date: "2025-03-31",
    sub_lines: (),
  ),
  (
    name: "Managed Services",
    description: "Monthly subscription",
    quantity: 1.0,
    unit_price: 350.0,
    vat_rate: 20.0,
    subtotal: 350.0,
    total: 350.0,
    start_date: "2025-03-01",
    end_date: "2025-03-31",
    sub_lines: (
      (
        name: "Cloud Hosting",
        quantity: 1.0,
        unit_price: 200.0,
        total: 200.0,
      ),
      (
        name: "Maintenance",
        quantity: 1.0,
        unit_price: 150.0,
        total: 150.0,
      ),
    ),
  ),
  (
    name: "Training Sessions",
    description: "Staff training workshops",
    quantity: 2.0,
    unit_price: 175.0,
    vat_rate: 20.0,
    subtotal: 350.0,
    total: 350.0,
    start_date: "2025-03-15",
    end_date: "2025-03-16",
    sub_lines: (),
  ),
  (
    name: "Hardware Supplies",
    description: "Various equipment",
    quantity: 1.0,
    unit_price: 450.0,
    vat_rate: 20.0,
    subtotal: 450.0,
    total: 450.0,
    start_date: "2025-03-10",
    end_date: "2025-03-10",
    sub_lines: (),
  ),
)

#let example_translations = (
  invoice_title: "Invoice",
  invoice_number: "Invoice number",
  issue_date: "Date of issue",
  amount_due: "Amount Due",
  due_date: "Date due",
  bill_from: "From",
  bill_to: "Bill to",
  description: "Description",
  quantity: "Qty",
  unit_price: "Unit price",
  tax_rate: "Tax Rate",
  tax: "Tax",
  amount: "Total (excl. tax)",
  subtotal: "Subtotal",
  total_due: "Total",
  legal_info: "Legal Information",
  vat_exempt_legal: "VAT not applicable",
  vat_id: "VAT ID",
  pay_online: "Pay online",
  payment_status: "PAYMENT STATUS",
  payment_status_paid: "Paid",
  payment_status_partially_paid: "Partially Paid",
  payment_status_unpaid: "Unpaid",
  payment_method: "Method",
  payment_date: "Date",
  payment_amount: "Amount",
  payment_terms_title: "PAYMENT TERMS",
  payment_terms_text: "Payment to be made within 14 days",
  tax_info_title: "TAX INFORMATION",
  tax_reverse_charge: "Tax to be paid on reverse charge basis",
  exchange_rate_info: "Exchange rate: 1 EUR = 1.08 USD | Converted amount = USD 1296.00",
  no_transactions: "No payments received",
  payment_info_title: "PAYMENT INFORMATION",
  tax_breakdown_title: "TAX BREAKDOWN",
  vat_standard: "VAT (Standard Rate)",
  vat_reduced: "VAT (Reduced Rate)",
  vat_exempt_notice: "VAT exempt items not included in tax calculation",
  reverse_charge_notice: "Reverse charge applicable - customer liable for VAT",
  intra_eu_notice: "Intra-EU supply - Art. 138 EU VAT Directive",
  b2b_notice: "Business-to-business transaction",
)

#let example_coupons = (
  (
    name: "Spring Promotion 10% off",
    total: 25.0,
  ),
  (
    name: "Loyalty Discount",
    total: 15.0,
  ),
)

#let example_tax_breakdown = (
  (
    name: "VAT (Standard Rate)",
    rate: 20.0,
    amount: 380.0,
  ),
  (
    name: "VAT (Reduced Rate)",
    rate: 10.0,
    amount: 50.0,
  ),
)

// Call the invoice function with example data
#invoice(
  "en-US",                 // lang
  example_organization,    // organization
  example_customer,        // customer
  "INV-2025-001",          // number
  "April 1, 2025",         // issue_date
  "April 15, 2025",        // due_date
  2150.0,                  // subtotal
  430.0,                   // tax_amount
  2580.0,                  // total_amount
  "EUR",                   // currency_code
  "€",                     // currency_symbol
  "Thank you for your subscription!", // memo
  14,                      // payment_term
  example_lines,           // lines
  example_coupons,         // coupons
  example_tax_breakdown,   // tax_breakdown
  example_translations,    // translations
  (symbol: "€"),           // formatted_currency
  pay_online_url: "https://pay.meteroid.com/inv-2025-001", // Optional payment URL
  footer_custom_message: "Invoice generated by YourCompany.com", // Optional footer message
  payment_status: "partially_paid", // "paid", "partially_paid", "unpaid"
  transactions: example_transactions, // list of transactions
  payment_info: example_payment_info, // key-value pairs for payment information
  // Optional section flags
  show_payment_status: true,
  show_payment_info: true,
  show_terms: true,
  show_tax_info: true,
  show_legal_info: true,
)
