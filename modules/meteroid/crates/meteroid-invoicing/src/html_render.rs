use crate::errors::InvoicingError;
use crate::model::*;
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::prelude::*;
use chrono::NaiveDate;
use maud::{html, Markup, DOCTYPE};
use rust_decimal::Decimal;
use rusty_money::{iso, FormattableCurrency};

#[allow(clippy::all)]
mod l10n {
    fluent_static::include_source!("l10n.rs");
    include!(concat!(env!("OUT_DIR"), "/i18n_data.rs"));

    pub fn get_country_local_name<'a>(lang: &str, country_code: &str) -> Option<&'a str> {
        let primary = LOCALES
            .get(lang)
            .and_then(|locale| locale.get(country_code).map(|v| v.as_str()));
        let fallback = || {
            LOCALES
                .get("en-US")
                .and_then(|locale| locale.get(country_code).map(|v| v.as_str()))
        };

        primary.or_else(fallback)
    }
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
                title { (l10n::invoice::invoice_title(lang) )};
                link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet"; // TODO include it in docker
                style {
                    (CSS)
                    r#"
                    body {
                        font-family: 'Inter', sans-serif;
                        font-optical-sizing: auto;
                        font-style: normal;
                    }
                    "#

                }

            }
            body class="" {
                div class="container mx-auto px-2 py-4 bg-white text-sm" {
                    (render_header(lang, &invoice.organization, &invoice.metadata)?)
                    (render_billing_info(lang, &invoice.organization, &invoice.customer, &invoice.metadata)?)
                    (render_invoice_lines(lang, &invoice.lines, &invoice.metadata.currency)?)
                    (render_invoice_summary(lang, &invoice.metadata ))
                    (render_legal_info(lang, &invoice.organization, &invoice.metadata)?)
                }
            }
        }
    })
}

fn render_header(
    lang: &str,
    organization: &Organization,
    invoice: &InvoiceMetadata,
) -> Result<Markup, InvoicingError> {
    Ok(html! {
        div class="px-2 flex justify-between items-center border-b pb-4" {
            h1 class="text-xl font-semibold text-gray-800" { (l10n::invoice::invoice_number(lang, &invoice.number)
                        .map_err(|_| InvoicingError::I18nError(format!("Failed to localise invoice number for value: {}", &invoice.number)))? )
            }
             @if let Some(logo_url) = &organization.logo_url {
                img src=(logo_url) alt=(l10n::invoice::company_logo_alt(lang));
            }
        }
    })
}

fn render_billing_info(
    lang: &str,
    organization: &Organization,
    customer: &Customer,
    invoice: &InvoiceMetadata,
) -> Result<Markup, InvoicingError> {
    Ok(html! {
        div class="grid grid-cols-3 mb-8" {
            div class="flex flex-col p-4 border-b border-r border-gray-200" {
                h2 class="text-md mb-2 text-gray-700" { (l10n::invoice::bill_from(lang)) }
                (render_address( organization, lang))
            }
            div class="flex flex-col p-4 border-b border-gray-200" {
                h2 class="text-md mb-2 text-gray-700" { (l10n::invoice::bill_to(lang)) }
                 (render_address( customer, lang))
            }
            div class="p-4 border-b border-l border-gray-200" {
                h2 class="text-right text-md mb-2 text-gray-700" { (l10n::invoice::amount_due(lang)) }
                p class="text-right mb-2 text-xl font-bold text-green-600" { (format_currency_minor(invoice.total_amount, &invoice.currency)) }

                div class="grid grid-cols-2 text-xs " {
                  div {
                    p class="text-gray-600" { (l10n::invoice::issue_date(lang)) }
                    p class="font-medium" { (format_date(lang, &invoice.issue_date)?) }
                  }
                  div{
                    p class="text-gray-600" { (l10n::invoice::due_date(lang)) }
                    p class="font-medium" { (format_date(lang, &invoice.due_date)?) }
                  }
                }
            }
        }
        @if let Some(memo) = &invoice.memo {
                p class="mb-4 rounded-lg p-4 bg-gray-50" { (memo) }
        }
    })
}

fn render_invoice_lines(
    lang: &str,
    lines: &[InvoiceLine],
    currency: &iso::Currency,
) -> Result<Markup, InvoicingError> {
    Ok(html! {
        div class="mb-8" {
            h2 class="px-2 text-md font-semibold mb-4 text-gray-700 uppercase" { (l10n::invoice::invoice_lines(lang)) }
            table class="w-full border-collapse" {
                thead {
                    tr class="text-gray-500 text-sm" {
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
                                    (format!("{} → {}", format_date_short(lang, &line.start_date)?, format_date_short(lang, &line.end_date)?))
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
                                    (format_percentage_dec(vat_rate))
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
        div class="grid grid-cols-2 border-b border-gray-200 mb-8" {
            div {}
            div class="mb-4 rounded-lg p-4 bg-gray-50" {
                table class="w-full" {
                    tr class="font-semibold"  {
                        td class="p-2" { (l10n::invoice::subtotal(lang)) }
                        td class="p-2 text-right font-medium" { (format_currency_minor(invoice.subtotal, &invoice.currency)) }
                    }
                    tr {
                        td class="p-2 text-gray-600" { (l10n::invoice::tax(lang)) " " (format_percentage_minor(invoice.tax_rate)) }
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
}

fn render_legal_info(
    lang: &str,
    organization: &Organization,
    invoice: &InvoiceMetadata,
) -> Result<Markup, InvoicingError> {
    let exchange_rate_text = match organization.exchange_rate {
        Some(rate) => {
            let date = format_date(lang, &invoice.issue_date)
                .map_err(|_| InvoicingError::I18nError("Failed to format date".to_string()))?;
            let equality = format!(
                "1 {} = {} {}",
                invoice.currency.code(),
                rate,
                organization.accounting_currency.code()
            );
            let amount_converted = format_currency_dec(
                Decimal::from(invoice.total_amount) * rate,
                &organization.accounting_currency,
            );

            l10n::invoice::exchange_rate_info(lang, &equality, &amount_converted, &date).ok()
        }
        None => None,
    };

    Ok(html! {
        div class="px-2 mb-8 text-gray-700" {
            h2 class="text-md font-semibold mb-4 text-gray-700 uppercase" { (l10n::invoice::legal_info(lang)) }

            // TODO need proper tax info engine for other EU countries
            @if invoice.tax_rate == 0 {
                p { (l10n::invoice::vat_exempt_legal(lang)) }
            }
            @if let Some(footer_info) = &organization.footer_info {
                p { (footer_info) }
            }
            @if let Some(footer_legal) = &organization.footer_legal  {
                p { (footer_legal) }
            }

              @if let Some(exchange_rate_text) = exchange_rate_text {
                p { (exchange_rate_text)  }
            }

            // TODO add change rate info
        }

    })
}

fn format_currency_dec(amount: Decimal, currency: &iso::Currency) -> String {
    rusty_money::Money::from_decimal(amount, currency).to_string() // TODO improve i18n format (currency after for FR, dot or coma, ..)
}

fn format_currency_minor(amount: i64, currency: &iso::Currency) -> String {
    rusty_money::Money::from_minor(amount, currency).to_string()
}

fn format_quantity(quantity: Decimal) -> String {
    format!("{:.2}", quantity)
}

fn format_percentage_dec(rate: Decimal) -> String {
    format!("{}%", rate.normalize())
}
fn format_percentage_minor(rate: i32) -> String {
    format!(
        "{}%",
        (Decimal::from(rate) / Decimal::from(100)).normalize()
    )
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

fn render_address<T>(entity: &T, lang: &str) -> Markup
where
    T: HasAddress,
{
    let local_country = entity
        .country()
        .and_then(|c| l10n::get_country_local_name(lang, c));

    html! {
        p class="font-semibold flex-1 pb-1" { (entity.name()) }
        div class="text-xs" {
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
        @if let Some(country) = local_country {
             p { (country) }
        }
        // p { (entity.email()) }
        @if let Some(tax_id) = entity.tax_id() {
            p { "Tax ID: " (tax_id) }
        }
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
        self.address.country.as_deref() // TODO we have the country code, not full name. We need i18n full name
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
