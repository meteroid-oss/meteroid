//! Every request body the client sends, checked against the published spec
//! (`openapi/pennylane-v2.json`): required fields, unknown fields, types and enums.

use chrono::NaiveDate;
use pennylane_client::customer_invoices::{
    Amount, CustomerInvoiceImport, ImportedInvoiceUpdate, ImputationDates, InvoiceLine,
    TransactionReference,
};
use pennylane_client::customers::{Address, BillingLanguage, CompanyCustomer, IndividualCustomer};
use pennylane_client::vat_rate::{VAT_RATE_CODES, VatRate};
use rust_decimal_macros::dec;
use serde::Serialize;
use serde_json::Value;
use std::sync::LazyLock;

static SPEC: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../openapi/pennylane-v2.json")).expect("valid spec")
});

fn operation(method: &str, path: &str) -> &'static Value {
    let op = &SPEC["paths"][path][method];
    assert!(op.is_object(), "{method} {path} is not in the spec");
    op
}

fn request_schema(method: &str, path: &str) -> &'static Value {
    &operation(method, path)["requestBody"]["content"]["application/json"]["schema"]
}

fn response_schema(method: &str, path: &str, status: &str) -> &'static Value {
    &operation(method, path)["responses"][status]["content"]["application/json"]["schema"]
}

fn assert_conforms(method: &str, path: &str, body: &impl Serialize) {
    let value = serde_json::to_value(body).unwrap();
    let mut errors = vec![];
    validate(request_schema(method, path), &value, "$", &mut errors);
    assert!(errors.is_empty(), "{method} {path}:\n{}", errors.join("\n"));
}

fn validate(schema: &Value, value: &Value, at: &str, errors: &mut Vec<String>) {
    if let Some(all) = schema["allOf"].as_array() {
        for sub in all {
            validate(sub, value, at, errors);
        }
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(options) = schema[key].as_array() {
            let matches = options.iter().any(|sub| {
                let mut sub_errors = vec![];
                validate(sub, value, at, &mut sub_errors);
                sub_errors.is_empty()
            });
            if !matches {
                errors.push(format!("{at}: matches none of {key}"));
            }
        }
    }

    if value.is_null() {
        let allowed = schema["nullable"].as_bool() == Some(true)
            || schema["enum"]
                .as_array()
                .is_some_and(|values| values.contains(&Value::Null))
            || schema.get("type").is_none();
        if !allowed {
            errors.push(format!("{at}: null is not allowed"));
        }
        return;
    }

    if let Some(values) = schema["enum"].as_array()
        && !values.contains(value)
    {
        errors.push(format!("{at}: {value} is not one of the enum values"));
    }

    match schema["type"].as_str() {
        Some("object") => validate_object(schema, value, at, errors),
        Some("array") => match value.as_array() {
            Some(items) => {
                for (index, item) in items.iter().enumerate() {
                    validate(&schema["items"], item, &format!("{at}[{index}]"), errors);
                }
            }
            None => errors.push(format!("{at}: expected an array")),
        },
        Some("string") => match value.as_str() {
            Some(text) => validate_string(schema, text, at, errors),
            None => errors.push(format!("{at}: expected a string, got {value}")),
        },
        Some("integer") if !value.is_i64() => errors.push(format!("{at}: expected an integer")),
        Some("number") if !value.is_number() => errors.push(format!("{at}: expected a number")),
        Some("boolean") if !value.is_boolean() => errors.push(format!("{at}: expected a boolean")),
        _ => {
            if schema.get("properties").is_some() {
                validate_object(schema, value, at, errors);
            }
        }
    }
}

fn validate_object(schema: &Value, value: &Value, at: &str, errors: &mut Vec<String>) {
    let Some(object) = value.as_object() else {
        errors.push(format!("{at}: expected an object"));
        return;
    };
    let properties = schema["properties"].as_object();
    for required in schema["required"].as_array().into_iter().flatten() {
        let required = required.as_str().unwrap();
        if !object.contains_key(required) {
            errors.push(format!("{at}: missing required `{required}`"));
        }
    }
    for (key, field) in object {
        match properties.and_then(|properties| properties.get(key)) {
            Some(field_schema) => validate(field_schema, field, &format!("{at}.{key}"), errors),
            None if properties.is_some() => errors.push(format!("{at}: unknown field `{key}`")),
            None => {}
        }
    }
}

fn validate_string(schema: &Value, text: &str, at: &str, errors: &mut Vec<String>) {
    if let Some(min) = schema["minLength"].as_u64()
        && (text.chars().count() as u64) < min
    {
        errors.push(format!("{at}: shorter than {min}"));
    }
    if schema["format"].as_str() == Some("date")
        && NaiveDate::parse_from_str(text, "%Y-%m-%d").is_err()
    {
        errors.push(format!("{at}: `{text}` is not a date"));
    }
    match schema["pattern"].as_str() {
        Some(r"^(-?\d+(\.\d+)?)?$") if !is_decimal(text) => {
            errors.push(format!("{at}: `{text}` is not a decimal"));
        }
        Some(pattern) if pattern != r"^(-?\d+(\.\d+)?)?$" => {
            errors.push(format!("{at}: unchecked pattern {pattern}"));
        }
        _ => {}
    }
}

fn is_decimal(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() {
        return text.is_empty();
    }
    let mut parts = digits.splitn(2, '.');
    let whole = parts.next().unwrap();
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    all_digits(whole) && parts.next().is_none_or(all_digits)
}

fn address() -> Address {
    Address {
        address: "10 rue de la Paix, Bâtiment B".into(),
        postal_code: "75002".into(),
        city: "Paris".into(),
        country_alpha2: "FR".into(),
    }
}

fn company() -> CompanyCustomer {
    CompanyCustomer {
        name: "Acme SAS".into(),
        vat_number: Some("FR32123456789".into()),
        reg_no: Some("123456789".into()),
        phone: Some("+33102030405".into()),
        billing_address: address(),
        delivery_address: Some(address()),
        emails: vec!["billing@acme.test".into()],
        external_reference: "cus_123".into(),
        billing_language: Some(BillingLanguage::French),
    }
}

fn individual() -> IndividualCustomer {
    IndividualCustomer {
        first_name: "Ada".into(),
        last_name: "Lovelace".into(),
        phone: None,
        billing_address: address(),
        delivery_address: None,
        emails: vec![],
        external_reference: "cus_456".into(),
        billing_language: Some(BillingLanguage::English),
    }
}

fn invoice() -> CustomerInvoiceImport {
    let line = InvoiceLine {
        label: "Pro plan".into(),
        quantity: dec!(2),
        raw_currency_unit_price: "50.00".into(),
        unit: String::new(),
        vat_rate: VatRate::standard("FR", dec!(0.2)).unwrap(),
        currency_amount: Amount(dec!(120.00)),
        currency_tax: Amount(dec!(20.00)),
        amount: None,
        tax: None,
        imputation_dates: Some(ImputationDates {
            start_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
        }),
    };
    CustomerInvoiceImport {
        file_attachment_id: 4321,
        customer_id: 1001,
        external_reference: "inv_123".into(),
        invoice_number: "INV-0001".into(),
        date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        deadline: NaiveDate::from_ymd_opt(2026, 11, 30).unwrap(),
        currency: "EUR".into(),
        currency_amount_before_tax: Amount(dec!(100.00)),
        currency_amount: Amount(dec!(120.00)),
        currency_tax: Amount(dec!(20.00)),
        amount: None,
        tax: None,
        credited_invoice_id: None,
        transaction_reference: Some(TransactionReference::bank_label("INV-0001")),
        purchase_order_reference: Some("PO-42".into()),
        invoice_lines: vec![line],
    }
}

#[test]
fn company_customer_bodies_conform() {
    assert_conforms("post", "/api/external/v2/company_customers", &company());
    assert_conforms("put", "/api/external/v2/company_customers/{id}", &company());
    let sparse = CompanyCustomer {
        vat_number: None,
        reg_no: None,
        phone: None,
        delivery_address: None,
        billing_language: None,
        ..company()
    };
    assert_conforms("post", "/api/external/v2/company_customers", &sparse);
}

#[test]
fn individual_customer_bodies_conform() {
    assert_conforms(
        "post",
        "/api/external/v2/individual_customers",
        &individual(),
    );
    assert_conforms(
        "put",
        "/api/external/v2/individual_customers/{id}",
        &individual(),
    );
}

#[test]
fn invoice_import_conforms() {
    assert_conforms(
        "post",
        "/api/external/v2/customer_invoices/import",
        &invoice(),
    );

    let mut in_dollars = invoice();
    in_dollars.currency = "USD".into();
    in_dollars.amount = Some(Amount(dec!(110.40)));
    in_dollars.tax = Some(Amount(dec!(18.40)));
    in_dollars.invoice_lines[0].amount = Some(Amount(dec!(110.40)));
    in_dollars.invoice_lines[0].tax = Some(Amount(dec!(18.40)));
    assert_conforms(
        "post",
        "/api/external/v2/customer_invoices/import",
        &in_dollars,
    );

    let paid_with_stripe = CustomerInvoiceImport {
        transaction_reference: Some(TransactionReference::stripe_payment("pi_123")),
        purchase_order_reference: None,
        ..invoice()
    };
    assert_conforms(
        "post",
        "/api/external/v2/customer_invoices/import",
        &paid_with_stripe,
    );
}

#[test]
fn credit_note_import_conforms() {
    let mut credit_note = invoice();
    credit_note.credited_invoice_id = Some(5678);
    credit_note.transaction_reference = None;
    credit_note.currency_amount_before_tax = Amount(dec!(-100.00));
    credit_note.currency_amount = Amount(dec!(-120.00));
    credit_note.currency_tax = Amount(dec!(-20.00));
    credit_note.invoice_lines[0].raw_currency_unit_price = "-50.00".into();
    credit_note.invoice_lines[0].currency_amount = Amount(dec!(-120.00));
    credit_note.invoice_lines[0].currency_tax = Amount(dec!(-20.00));
    credit_note.invoice_lines[0].vat_rate = VatRate::CROSSBORDER;
    credit_note.invoice_lines[0].imputation_dates = None;
    assert_conforms(
        "post",
        "/api/external/v2/customer_invoices/import",
        &credit_note,
    );
}

#[test]
fn imported_invoice_update_conforms() {
    let update = ImportedInvoiceUpdate {
        transaction_reference: TransactionReference::gocardless_payment("PM0001"),
    };
    assert_conforms(
        "put",
        "/api/external/v2/customer_invoices/{id}/update_imported",
        &update,
    );
    let update = ImportedInvoiceUpdate {
        transaction_reference: TransactionReference::stripe_charge("ch_123"),
    };
    assert_conforms(
        "put",
        "/api/external/v2/customer_invoices/{id}/update_imported",
        &update,
    );
}

#[test]
fn the_check_rejects_what_pennylane_would() {
    let schema = request_schema("post", "/api/external/v2/customer_invoices/import");
    let mut body = serde_json::to_value(invoice()).unwrap();
    body["label_typo"] = Value::from("x");
    body.as_object_mut().unwrap().remove("deadline");
    body["currency_amount"] = Value::from("12,00");
    body["invoice_lines"][0]["vat_rate"] = Value::from("FR_190");

    let mut errors = vec![];
    validate(schema, &body, "$", &mut errors);
    let errors = errors.join("\n");
    for expected in [
        "unknown field `label_typo`",
        "missing required `deadline`",
        "`12,00` is not a decimal",
        "$.invoice_lines[0].vat_rate: \"FR_190\" is not one of the enum values",
    ] {
        assert!(errors.contains(expected), "{expected} not in:\n{errors}");
    }
}

#[test]
fn vat_rate_codes_are_the_published_enum() {
    let published = &request_schema("post", "/api/external/v2/customer_invoices/import")["properties"]
        ["invoice_lines"]["items"]["properties"]["vat_rate"]["enum"];
    let published: Vec<&str> = published
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap())
        .collect();
    assert_eq!(published, VAT_RATE_CODES);
}

#[test]
fn lookups_filter_on_external_reference() {
    for path in [
        "/api/external/v2/customers",
        "/api/external/v2/customer_invoices",
    ] {
        let filter = operation("get", path)["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|param| param["name"] == "filter")
            .unwrap_or_else(|| panic!("{path} has no filter"));
        let doc = filter["description"].as_str().unwrap();
        assert!(
            doc.contains("`external_reference`: `") && doc.contains("`eq`"),
            "{path} cannot filter on external_reference"
        );
    }
}

#[test]
fn responses_carry_what_is_read() {
    let has = |schema: &Value, field: &str| schema["properties"].get(field).is_some();

    let invoice = response_schema("post", "/api/external/v2/customer_invoices/import", "201");
    for field in ["id", "external_reference", "paid", "transaction_reference"] {
        assert!(has(invoice, field), "invoice response lacks {field}");
    }
    let attachment = response_schema("post", "/api/external/v2/file_attachments", "201");
    assert!(has(attachment, "id"));
    let customer = response_schema("post", "/api/external/v2/company_customers", "201");
    assert!(has(customer, "id") && has(customer, "external_reference"));
    let me = response_schema("get", "/api/external/v2/me", "200");
    assert!(has(me, "company") && has(me, "scopes"));
    assert!(has(&me["properties"]["company"], "id"));
}
