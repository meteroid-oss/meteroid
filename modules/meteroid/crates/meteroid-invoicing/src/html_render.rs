use crate::errors::InvoicingError;
use chrono::prelude::*;
use chrono::NaiveDate;
use maud::{html, Markup, DOCTYPE};
use rust_decimal::Decimal;
use rusty_money::iso;

use crate::model::*;

mod l10n {
    fluent_static::include_source!("l10n.rs");
}

static CSS: &str = include_str!("../assets/styles.css");

pub fn render_invoice(invoice: &Invoice) -> Result<Markup, InvoicingError> {
    let lang_id =
        unic_langid::parser::parse_language_identifier(invoice.lang.as_bytes()).map_err(|_| {
            InvoicingError::I18nError(format!("Invalid language identifier : {}", &invoice.lang))
        })?;

    let lang = match lang_id.language.as_str() {
        "fr" => "fr-FR",
        _ => "en-US",
    };

    Ok(html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="UTF-8";
                meta name="viewport" content="width=device-width, initial-scale=1.0";
                title { (l10n::invoice::invoice_title(lang) )}
                style { (CSS) }
            }
            body class="" {
                div class="container mx-auto px-4 py-8 bg-white text-sm" {
                    (render_header(lang, &invoice.organization))
                    (render_invoice_info(lang, &invoice.metadata)?)
                    (render_billing_info(lang, &invoice.organization, &invoice.customer))
                    (render_invoice_lines(lang, &invoice.lines, &invoice.metadata.currency)?)
                    (render_invoice_summary(lang, &invoice.metadata ))
                    (render_footer(lang, &invoice.organization))
                }
            }
        }
    })
}

fn render_billing_info(lang: &str, organization: &Organization, customer: &Customer) -> Markup {
    html! {
        div class="grid grid-cols-2 gap-8 mb-8" {
            div {
                h2 class="text-xl font-semibold mb-2 text-gray-700" { (l10n::invoice::bill_from(lang)) }
                (render_address( organization))
            }
            div {
                h2 class="text-xl font-semibold mb-2 text-gray-700" { (l10n::invoice::bill_to(lang)) }
                (render_address(  customer))
            }
        }
    }
}

fn render_invoice_lines(
    lang: &str,
    lines: &[InvoiceLine],
    currency: &iso::Currency,
) -> Result<Markup, InvoicingError> {
    Ok(html! {
        div class="mb-8" {
            h2 class="text-xl font-semibold mb-4 text-gray-700" { (l10n::invoice::invoice_lines(lang)) }
            table class="w-full border-collapse" {
                thead {
                    tr class="bg-gray-200 text-gray-700" {
                        th class="p-2 text-left" { (l10n::invoice::description(lang)) }
                        th class="p-2 text-right" { (l10n::invoice::quantity(lang)) }
                        th class="p-2 text-right" { (l10n::invoice::unit_price(lang)) }
                        th class="p-2 text-right" { (l10n::invoice::tax_rate(lang)) }
                        th class="p-2 text-right" { (l10n::invoice::amount(lang)) }
                    }
                }
                tbody {
                    @for line in lines {
                        tr class="border-b border-gray-200" {
                             td class="p-2 text-gray-600" {
                                div class="font-semibold" { (line.name) }
                                @if let Some(desc) = &line.description {
                                    div class="text-sm text-gray-500" { (desc) }
                                }
                                div class="text-sm text-gray-500" {
                                    (format!("{} ➔ {}", format_date_short(lang, &line.start_date)?, format_date_short(lang, &line.end_date)?))
                                }
                            }
                            td class="p-2 text-right text-gray-600" {
                                 @if let Some(quantity) = line.quantity {
                                    (format_quantity(quantity))
                                }
                            }
                            td class="p-2 text-right text-gray-600" {
                                 @if let Some(unit_price) = line.unit_price {
                                    (format_currency_dec(unit_price, currency))
                                }
                            }
                            td class="p-2 text-right text-gray-600" {
                                  @if let Some(vat_rate) = line.vat_rate {
                                    (format_percentage(vat_rate))
                                }
                            }
                            td class="p-2 text-right font-medium text-gray-800" {
                                (format_currency_minor(line.subtotal, currency))
                            }
                        }
                        @if !line.sub_lines.is_empty() {
                            tr class="bg-gray-50" {
                                td colspan="4" class="p-2" {
                                    table class="w-full text-sm" {
                                        @for sub_line in &line.sub_lines {
                                            tr {
                                                td class="p-1 text-gray-600" { (sub_line.name) } // TODO i18n the name (rebuild it from attributes, similar to how we initially build it)
                                                td class="p-1 text-right text-gray-600" { (format_quantity(sub_line.quantity)) }
                                                td class="p-1 text-right text-gray-600" { (format_currency_dec(sub_line.unit_price, currency)) }
                                                td class="p-1 text-right text-gray-700" { (format_currency_minor(sub_line.total, currency)) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

fn render_invoice_summary(lang: &str, invoice: &InvoiceMetadata) -> Markup {
    html! {
        div class="mb-8" {
            h2 class="text-xl font-semibold mb-4 text-gray-700" { (l10n::invoice::invoice_summary(lang)) }
            table class="w-full" {
                tr {
                    td class="p-2 text-gray-600" { (l10n::invoice::subtotal(lang)) }
                    td class="p-2 text-right font-medium text-gray-800" { (format_currency_minor(invoice.subtotal, &invoice.currency)) }
                }
                tr {
                    td class="p-2 text-gray-600" { (l10n::invoice::tax_total(lang)) }
                    td class="p-2 text-right font-medium text-gray-800" { (format_currency_minor(invoice.tax_amount, &invoice.currency)) }
                }
                tr class="border-t border-gray-200" {
                    td class="p-2 text-lg font-semibold text-gray-700" { (l10n::invoice::total_due(lang)) }
                    td class="p-2 text-right text-lg font-bold text-green-600" { (format_currency_minor(invoice.total_amount, &invoice.currency)) }
                }
            }
        }
    }
}

fn render_footer(lang: &str, organization: &Organization) -> Markup {
    html! {
        footer class="mt-8 text-center text-gray-500 text-sm" {
            p { (l10n::invoice::thank_you_message(lang)) }
            @if let Some(email) = &organization.email {
                p { (l10n::invoice::contact_message(lang)) " " (email) }
            }
        }
    }
}

fn render_header(lang: &str, organization: &Organization) -> Markup {
    html! {
        div class="flex justify-between items-center mb-8 border-b pb-4" {
            h1 class="text-3xl font-bold text-gray-800" { (l10n::invoice::invoice_title(lang)) }
            @if let Some(logo_url) = &organization.logo_url {
                img src=(logo_url) alt=(l10n::invoice::company_logo_alt(lang));
            }
        }
    }
}

fn render_invoice_info(lang: &str, invoice: &InvoiceMetadata) -> Result<Markup, InvoicingError> {
    Ok(html! {
        div class="grid grid-cols-2 gap-8 mb-8" {
            div {
                h2 class="text-xl font-semibold mb-2 text-gray-700" { (l10n::invoice::invoice_details(lang)) }
                p class="text-gray-600" { (l10n::invoice::invoice_number(lang)) " " span class="font-medium" { (invoice.number) } }
                p class="text-gray-600" { (l10n::invoice::issue_date(lang)) " " span class="font-medium" { (format_date(lang, &invoice.issue_date)?) } }
                p class="text-gray-600" {
                    (l10n::invoice::payment_terms(lang, invoice.payment_term )
                        .map_err(|_| InvoicingError::I18nError(format!("Failed to localise payment_terms for value: {}", &invoice.payment_term)))?  )
               }
            }
            div class="text-right" {
                h2 class="text-xl font-semibold mb-2 text-gray-700" { (l10n::invoice::amount_due(lang)) }
                p class="text-3xl font-bold text-green-600" { (format_currency_minor(invoice.total_amount, &invoice.currency)) }
                p class="text-gray-600" { (l10n::invoice::due_date(lang)) " " span class="font-medium" { (format_date(lang, &invoice.due_date)?) } }
            }
        }
    })
}

fn format_currency_dec(amount: Decimal, currency: &iso::Currency) -> String {
    rusty_money::Money::from_decimal(amount, currency).to_string()
}

fn format_currency_minor(amount: i64, currency: &iso::Currency) -> String {
    rusty_money::Money::from_minor(amount, currency).to_string()
}

fn format_quantity(quantity: Decimal) -> String {
    format!("{:.2}", quantity)
}

fn format_percentage(rate: Decimal) -> String {
    format!("{:.2}%", rate * Decimal::from(100))
}

fn format_date(lang: &str, date: &NaiveDate) -> Result<String, InvoicingError> {
    // TODO use icu crate for date formatting when adding new languages, fluent has no date formatting as of now https://github.com/projectfluent/fluent-rs/pull/335
    match lang {
        "fr-FR" => Ok(date.format_localized("%e %B %Y", Locale::fr_FR).to_string()),
        _ => Ok(date.format("%B %e, %Y").to_string()),
    }
}

fn format_date_short(lang: &str, date: &NaiveDate) -> Result<String, InvoicingError> {
    match lang {
        "fr-FR" => Ok(date.format_localized("%e %b %Y", Locale::fr_FR).to_string()),
        _ => Ok(date.format("%b %e, %Y").to_string()),
    }
}

fn render_address<T>(entity: &T) -> Markup
where
    T: HasAddress,
{
    html! {
        p { (entity.name()) }
        @if let Some(legal_number) = entity.legal_number() {
            p { (legal_number) }
        }
        @if let Some(address_line1) = entity.address_line1() {
            p { (address_line1) }
        }
        @if let Some(address_line2) = entity.address_line2() {
            p { (address_line2) }
        }
        p {
            @if let Some(zipcode) = entity.zipcode() {
                span { (zipcode) }
                ", "
             }
            @if let Some(city) = entity.city() {
                span { (city) }
            }
        }
        @if let Some(state) = entity.state() {
            p { (state) }
        }
        @if let Some(country) = entity.country() {
             p { (country) }
        }
        // p { (entity.email()) }
        @if let Some(tax_id) = entity.tax_id() {
            p { "Tax ID: " (tax_id) }
        }
    }
}

trait HasAddress {
    fn name(&self) -> &str;
    fn legal_number(&self) -> Option<&str>;
    fn address_line1(&self) -> Option<&str>;
    fn address_line2(&self) -> Option<&str>;
    fn zipcode(&self) -> Option<&str>;
    fn city(&self) -> Option<&str>;
    fn state(&self) -> Option<&str>;
    fn country(&self) -> Option<&str>;
    // fn email(&self) -> &str;
    fn tax_id(&self) -> Option<&str>;
}

impl HasAddress for Organization {
    fn name(&self) -> &str {
        &self.name
    }
    fn legal_number(&self) -> Option<&str> {
        self.legal_number.as_deref()
    }
    fn address_line1(&self) -> Option<&str> {
        self.address.line1.as_deref()
    }
    fn address_line2(&self) -> Option<&str> {
        self.address.line2.as_deref()
    }
    fn zipcode(&self) -> Option<&str> {
        self.address.zip_code.as_deref()
    }
    fn city(&self) -> Option<&str> {
        self.address.city.as_deref()
    }
    fn state(&self) -> Option<&str> {
        self.address.state.as_deref()
    }
    fn country(&self) -> Option<&str> {
        self.address.country.as_deref()
    }
    fn tax_id(&self) -> Option<&str> {
        self.tax_id.as_deref()
    }
}

impl HasAddress for Customer {
    fn name(&self) -> &str {
        &self.name
    }
    fn legal_number(&self) -> Option<&str> {
        self.legal_number.as_deref()
    }
    fn address_line1(&self) -> Option<&str> {
        self.address.line1.as_deref()
    }
    fn address_line2(&self) -> Option<&str> {
        self.address.line2.as_deref()
    }
    fn zipcode(&self) -> Option<&str> {
        self.address.zip_code.as_deref()
    }
    fn city(&self) -> Option<&str> {
        self.address.city.as_deref()
    }
    fn state(&self) -> Option<&str> {
        self.address.state.as_deref()
    }
    fn country(&self) -> Option<&str> {
        self.address.country.as_deref()
    }
    fn tax_id(&self) -> Option<&str> {
        self.tax_id.as_deref()
    }
}
