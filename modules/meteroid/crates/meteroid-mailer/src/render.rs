//! Pure customer-email rendering: a fixed sailfish layout around one of the kind bodies, with
//! every string from the `EmailL10n` bundle of `layout.lang`. No I/O; the worker,
//! `PreviewEmail`, `SendTestEmail` and `ResendEmail` all go through [`render`].

use crate::errors::MailerServiceError;
use crate::l10n::{EmailL10N, email_l10n};
use crate::template::{
    AuthenticationRequiredContent, BudgetAlertContent, ButtonTemplate, CreditNoteIssuedContent,
    CustomBodyContent, EmailColors, HeaderTemplate, InvoicePaidContent, InvoiceReadyContent,
    PaymentFailedContent, QuoteReadyContent, StatusBadge, SummaryBoxTemplate, SummaryHero,
    WalletNoticeContent, render_layout,
};
use chrono::NaiveDate;
use common_domain::l10n::long_date;

pub use crate::template::format_currency;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrandCtx {
    pub company_name: String,
    pub logo_url: Option<String>,
    pub brand_color: Option<String>,
    /// Explicit colour for white backgrounds; overrides `brand_color` in emails when set.
    pub brand_color_light: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntityFooterCtx {
    pub legal_name: String,
    pub address_lines: Vec<String>,
    pub vat_number: Option<String>,
    pub legal_number: Option<String>,
    pub powered_by: bool,
}

/// Account mails keep today's Meteroid footer; customer mails carry the entity's legal block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FooterCtx {
    Meteroid,
    Entity(EntityFooterCtx),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutCtx {
    /// A `common_domain::lang::SUPPORTED` locale; anything else reads `en-US`.
    pub lang: &'static str,
    pub brand: BrandCtx,
    pub footer: FooterCtx,
    /// Non-production tenant: a localized "test email" banner tops the card.
    pub test_mode: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvoiceSummary {
    pub number: String,
    pub invoice_date: NaiveDate,
    pub due_date: NaiveDate,
    pub amount_minor: i64,
    pub currency: String,
    pub label: Option<String>,
}

/// The button under the body. A blank `label` takes the kind's default text
/// (`view-and-pay`, `update-payment-method`, `confirm-payment`, `view-credit-note`,
/// `view-and-sign-quote`, `pay-invoice` for custom bodies).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cta {
    pub label: String,
    pub url: String,
}

impl Cta {
    pub fn new(url: impl Into<String>) -> Self {
        Cta {
            label: String::new(),
            url: url.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    InvoiceReady {
        summary: InvoiceSummary,
        cta: Cta,
    },
    InvoicePaid {
        summary: InvoiceSummary,
    },
    QuoteReady {
        quote_number: String,
        expires_at: Option<NaiveDate>,
        custom_message: Option<String>,
        cta: Cta,
    },
    CreditNoteIssued {
        credit_note_number: String,
        invoice_number: Option<String>,
        amount_minor: i64,
        currency: String,
        cta: Option<Cta>,
    },
    /// No decline reason: providers only give free text, never shown to customers.
    PaymentFailed {
        summary: InvoiceSummary,
        cta: Cta,
    },
    AuthenticationRequired {
        summary: InvoiceSummary,
        cta: Cta,
    },
    /// The available balance fell below the customer's alert threshold.
    WalletBalanceLow {
        available_minor: i64,
        threshold_minor: i64,
        currency: String,
        /// Set when `currency` is a credit unit code.
        credit_unit: Option<EmailCreditUnit>,
        actions: WalletActions,
    },
    /// Spend reached a notification level of a budget; `level_pct >= 100` means it is reached.
    BudgetAlert {
        level_pct: u32,
        used_minor: i64,
        amount_minor: i64,
        currency: String,
        /// Set when `currency` is a credit unit code.
        credit_unit: Option<EmailCreditUnit>,
        window_start: NaiveDate,
        window_end: Option<NaiveDate>,
        /// The budget refuses the usage counted in its spend once reached.
        stops_usage: bool,
    },
    /// An automatic top-up could not be charged; `suspended` when top-ups are now paused.
    AutoTopUpFailed {
        amount_minor: i64,
        currency: String,
        /// Set when `amount_minor` is credits of a unit (`currency` its code), not money.
        credit_unit: Option<EmailCreditUnit>,
        suspended: bool,
        /// Suspended because a top-up payment of `amount_minor` was refunded or disputed.
        payment_reversed: bool,
        actions: WalletActions,
    },
    /// `html_fragment` comes from `copy::to_html` only; it is inserted unescaped.
    Custom {
        html_fragment: String,
        summary: Option<InvoiceSummary>,
        cta: Option<Cta>,
        hedge: Option<String>,
    },
}

/// What the customer may do about a prepaid-balance notice in the portal: the copy never asks for
/// an action the portal does not offer, and points to the merchant instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WalletActions {
    pub can_top_up: bool,
    pub can_manage_auto_top_up: bool,
}

/// A credit unit as the customer reads it; its code is internal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmailCreditUnit {
    pub name: String,
    pub plural_name: String,
    pub precision: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    /// The kind's localized subject; not available for `Body::Custom`.
    Fixed,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub subject: String,
    pub body_html: String,
}

fn row(label: impl ToString, value: impl Into<String>) -> (String, String) {
    (label.to_string(), value.into())
}

fn button(cta: Cta, default_label: impl ToString, color: &str) -> ButtonTemplate {
    let label = if cta.label.trim().is_empty() {
        default_label.to_string()
    } else {
        cta.label
    };
    ButtonTemplate {
        label,
        url: cta.url,
        color: color.to_string(),
    }
}

fn hero(label: impl ToString, amount_minor: i64, currency: &str) -> SummaryHero {
    SummaryHero {
        label: label.to_string(),
        amount: format_currency(amount_minor, currency),
        caption: None,
        status: None,
    }
}

/// A money amount, or an amount of a credit unit when one is given.
fn format_amount(amount_minor: i64, currency: &str, unit: Option<&EmailCreditUnit>) -> String {
    match unit {
        Some(unit) => crate::template::format_credit_units(amount_minor, unit),
        None => format_currency(amount_minor, currency),
    }
}

/// The amount due as the headline, its due date under it.
fn amount_due_hero(l10n: &EmailL10N, lang: &str, s: &InvoiceSummary) -> SummaryHero {
    SummaryHero {
        caption: Some(l10n.due_on(long_date(lang, &s.due_date)).to_string()),
        ..hero(l10n.amount_due(), s.amount_minor, &s.currency)
    }
}

/// An invoice summary: `hero` on top, then the invoice number and date.
fn invoice_box(
    l10n: &EmailL10N,
    lang: &str,
    s: &InvoiceSummary,
    hero: SummaryHero,
) -> SummaryBoxTemplate {
    SummaryBoxTemplate {
        hero: Some(hero),
        rows: vec![
            row(l10n.invoice_number(), s.number.clone()),
            row(l10n.invoice_date(), long_date(lang, &s.invoice_date)),
        ],
    }
}

/// The inbox preview line: the headline amount, its caption or status, the document number.
fn preview_line(summary: &SummaryBoxTemplate, number: &str) -> Option<String> {
    let hero = summary.hero.as_ref()?;
    let detail = hero
        .caption
        .as_deref()
        .or_else(|| hero.status.as_ref().map(|s| s.label.as_str()));
    let parts = [Some(hero.amount.as_str()), detail, Some(number)];
    Some(parts.into_iter().flatten().collect::<Vec<_>>().join(" · "))
}

/// Inline styles on the tags `copy::to_html` emits, for clients that drop `<style>`.
/// Text and variables are already escaped there, so a literal tag can only be markup.
fn style_fragment(html: &str) -> String {
    const BLOCK: &str = "margin:0 0 16px;font-size:15px;line-height:24px;color:#414651;";
    html.replace("<p>", &format!("<p style=\"{BLOCK}\">"))
        .replace("<ul>", &format!("<ul style=\"{BLOCK}padding:0 0 0 22px;\">"))
        .replace("<ol>", &format!("<ol style=\"{BLOCK}padding:0 0 0 22px;\">"))
        .replace("<ol start=", &format!("<ol style=\"{BLOCK}padding:0 0 0 22px;\" start="))
        .replace("<li>", "<li style=\"margin:0 0 4px;\">")
        .replace(
            "<blockquote>",
            "<blockquote style=\"margin:0 0 16px;padding:0 0 0 14px;border-left:3px solid #e2e4e9;color:#6b7079;\">",
        )
}

fn non_blank(label: &Option<String>) -> Option<String> {
    label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
}

/// Pure, sync, no I/O.
pub fn render(
    layout: &LayoutCtx,
    subject: Subject,
    body: Body,
) -> Result<Rendered, MailerServiceError> {
    let lang = layout.lang;
    let l10n = email_l10n(lang);
    let company = layout.brand.company_name.as_str();
    let mut header = HeaderTemplate::from(&layout.brand);
    if layout.test_mode {
        header.test_notice = Some(l10n.test_mode_notice().to_string());
    }
    let color = EmailColors::for_brand(&layout.brand).button;
    let footer = &layout.footer;

    let (fixed_subject, body_html) = match body {
        Body::InvoiceReady { summary, cta } => {
            let summary_box = invoice_box(
                &l10n,
                lang,
                &summary,
                amount_due_hero(&l10n, lang, &summary),
            );
            let preheader = preview_line(&summary_box, &summary.number);
            let content = InvoiceReadyContent {
                heading: l10n.invoice_ready_heading().to_string(),
                label: non_blank(&summary.label),
                summary: summary_box,
                cta: button(cta, l10n.view_and_pay(), &color),
            };
            let subject = l10n.invoice_ready_subject(company).to_string();
            (
                subject.clone(),
                render_layout(lang, subject, preheader, header, footer, &l10n, content)?,
            )
        }
        Body::InvoicePaid { summary } => {
            let paid = SummaryHero {
                status: Some(StatusBadge::success(l10n.status_paid().to_string())),
                ..hero(l10n.amount_paid(), summary.amount_minor, &summary.currency)
            };
            let summary_box = invoice_box(&l10n, lang, &summary, paid);
            let preheader = preview_line(&summary_box, &summary.number);
            let content = InvoicePaidContent {
                heading: l10n.invoice_paid_heading().to_string(),
                label: non_blank(&summary.label),
                summary: summary_box,
            };
            let subject = l10n.invoice_paid_subject(company).to_string();
            (
                subject.clone(),
                render_layout(lang, subject, preheader, header, footer, &l10n, content)?,
            )
        }
        Body::QuoteReady {
            quote_number,
            expires_at,
            custom_message,
            cta,
        } => {
            let mut rows = vec![row(l10n.quote_number(), quote_number.clone())];
            if let Some(expires_at) = expires_at {
                rows.push(row(l10n.valid_until(), long_date(lang, &expires_at)));
            }
            let content = QuoteReadyContent {
                heading: l10n.quote_ready_heading().to_string(),
                summary: SummaryBoxTemplate { hero: None, rows },
                custom_message: non_blank(&custom_message),
                cta: button(cta, l10n.view_and_sign_quote(), &color),
            };
            let subject = l10n.quote_ready_subject(&quote_number, company).to_string();
            (
                subject.clone(),
                render_layout(lang, subject, None, header, footer, &l10n, content)?,
            )
        }
        Body::CreditNoteIssued {
            credit_note_number,
            invoice_number,
            amount_minor,
            currency,
            cta,
        } => {
            let mut rows = vec![row(l10n.credit_note_number(), credit_note_number.clone())];
            if let Some(invoice_number) = non_blank(&invoice_number) {
                rows.push(row(l10n.invoice_number(), invoice_number));
            }
            let summary_box = SummaryBoxTemplate {
                hero: Some(hero(l10n.amount_credited(), amount_minor, &currency)),
                rows,
            };
            let preheader = preview_line(&summary_box, &credit_note_number);
            let content = CreditNoteIssuedContent {
                heading: l10n.credit_note_issued_heading().to_string(),
                summary: summary_box,
                cta: cta.map(|cta| button(cta, l10n.view_credit_note(), &color)),
            };
            let subject = l10n
                .credit_note_issued_subject(company, &credit_note_number)
                .to_string();
            (
                subject.clone(),
                render_layout(lang, subject, preheader, header, footer, &l10n, content)?,
            )
        }
        Body::PaymentFailed { summary, cta } => {
            let failed = SummaryHero {
                status: Some(StatusBadge::danger(
                    l10n.status_payment_failed().to_string(),
                )),
                ..amount_due_hero(&l10n, lang, &summary)
            };
            let summary_box = invoice_box(&l10n, lang, &summary, failed);
            let preheader = preview_line(&summary_box, &summary.number);
            let content = PaymentFailedContent {
                heading: l10n.payment_failed_heading().to_string(),
                label: non_blank(&summary.label),
                intro: l10n.payment_failed_intro().to_string(),
                summary: summary_box,
                cta: button(cta, l10n.update_payment_method(), &color),
            };
            let subject = l10n.payment_failed_subject(&summary.number).to_string();
            (
                subject.clone(),
                render_layout(lang, subject, preheader, header, footer, &l10n, content)?,
            )
        }
        Body::AuthenticationRequired { summary, cta } => {
            let amount = format_currency(summary.amount_minor, &summary.currency);
            let pending = SummaryHero {
                status: Some(StatusBadge::warning(
                    l10n.status_action_required().to_string(),
                )),
                ..amount_due_hero(&l10n, lang, &summary)
            };
            let summary_box = invoice_box(&l10n, lang, &summary, pending);
            let preheader = preview_line(&summary_box, &summary.number);
            let content = AuthenticationRequiredContent {
                heading: l10n.authentication_required_heading().to_string(),
                label: non_blank(&summary.label),
                summary: summary_box,
                intro: l10n.authentication_required_intro(&amount).to_string(),
                cta: button(cta, l10n.confirm_payment(), &color),
                hedge: l10n.already_paid_disregard().to_string(),
            };
            let subject = l10n
                .authentication_required_subject(&summary.number)
                .to_string();
            (
                subject.clone(),
                render_layout(lang, subject, preheader, header, footer, &l10n, content)?,
            )
        }
        Body::WalletBalanceLow {
            available_minor,
            threshold_minor,
            currency,
            credit_unit,
            actions,
        } => {
            let threshold = format_amount(threshold_minor, &currency, credit_unit.as_ref());
            let summary_box = SummaryBoxTemplate {
                hero: Some(SummaryHero {
                    status: Some(StatusBadge::warning(l10n.status_low_balance().to_string())),
                    amount: format_amount(available_minor, &currency, credit_unit.as_ref()),
                    ..hero(l10n.available_balance(), available_minor, &currency)
                }),
                rows: vec![row(l10n.alert_threshold(), threshold.clone())],
            };
            let preheader = summary_box.hero.as_ref().map(|hero| hero.amount.clone());
            let intro = if actions.can_top_up {
                l10n.wallet_balance_low_intro(&threshold)
            } else {
                l10n.wallet_balance_low_intro_contact(&threshold, company)
            };
            let content = WalletNoticeContent {
                heading: l10n.wallet_balance_low_heading().to_string(),
                intro: intro.to_string(),
                summary: summary_box,
            };
            let subject = l10n.wallet_balance_low_subject(company).to_string();
            (
                subject.clone(),
                render_layout(lang, subject, preheader, header, footer, &l10n, content)?,
            )
        }
        Body::BudgetAlert {
            level_pct,
            used_minor,
            amount_minor,
            currency,
            credit_unit,
            window_start,
            window_end,
            stops_usage,
        } => {
            let reached = level_pct >= 100;
            let budget = format_amount(amount_minor, &currency, credit_unit.as_ref());
            let level = format!("{level_pct}%");
            let badge = if reached {
                StatusBadge::danger(l10n.status_budget_reached().to_string())
            } else {
                StatusBadge::warning(l10n.status_budget_alert().to_string())
            };
            let mut rows = vec![
                row(l10n.budget_amount(), budget.clone()),
                row(l10n.notification_level(), level.clone()),
                row(l10n.period_start(), long_date(lang, &window_start)),
            ];
            if let Some(end) = window_end {
                rows.push(row(l10n.period_end(), long_date(lang, &end)));
            }
            let summary_box = SummaryBoxTemplate {
                hero: Some(SummaryHero {
                    status: Some(badge),
                    amount: format_amount(used_minor, &currency, credit_unit.as_ref()),
                    ..hero(l10n.budget_spend(), used_minor, &currency)
                }),
                rows,
            };
            let preheader = summary_box.hero.as_ref().map(|hero| hero.amount.clone());
            let (heading, intro, subject) = if reached {
                (
                    l10n.budget_exceeded_heading().to_string(),
                    l10n.budget_exceeded_intro(&budget).to_string(),
                    l10n.budget_exceeded_subject(company).to_string(),
                )
            } else {
                (
                    l10n.budget_threshold_heading(&level).to_string(),
                    l10n.budget_threshold_intro(&level, &budget).to_string(),
                    l10n.budget_threshold_subject(company, &level).to_string(),
                )
            };
            let content = BudgetAlertContent {
                heading,
                intro,
                summary: summary_box,
                note: match (stops_usage, reached, window_end.is_some()) {
                    (false, _, _) => l10n.budget_notify_only().to_string(),
                    (true, false, true) => l10n.budget_stops_usage_ahead().to_string(),
                    (true, false, false) => l10n.budget_stops_usage_ahead_open().to_string(),
                    (true, true, true) => l10n.budget_stops_usage_now().to_string(),
                    (true, true, false) => l10n.budget_stops_usage_now_open().to_string(),
                },
            };
            (
                subject.clone(),
                render_layout(lang, subject, preheader, header, footer, &l10n, content)?,
            )
        }
        Body::AutoTopUpFailed {
            amount_minor,
            currency,
            credit_unit,
            suspended,
            payment_reversed,
            actions,
        } => {
            let reversed = suspended && payment_reversed;
            let amount = format_amount(amount_minor, &currency, credit_unit.as_ref());
            let status = if suspended {
                StatusBadge::danger(l10n.status_paused().to_string())
            } else {
                StatusBadge::danger(l10n.status_payment_failed().to_string())
            };
            let summary_box = SummaryBoxTemplate {
                hero: Some(SummaryHero {
                    status: Some(status),
                    amount: amount.clone(),
                    ..hero(
                        if reversed {
                            l10n.reversed_payment_amount()
                        } else {
                            l10n.top_up_amount()
                        },
                        amount_minor,
                        &currency,
                    )
                }),
                rows: vec![],
            };
            let preheader = summary_box.hero.as_ref().map(|hero| hero.amount.clone());
            let (heading, intro, subject) = if suspended {
                (
                    l10n.auto_top_up_suspended_heading().to_string(),
                    match (reversed, actions.can_manage_auto_top_up) {
                        (true, true) => l10n.auto_top_up_reversed_intro(&amount),
                        (true, false) => l10n.auto_top_up_reversed_intro_contact(&amount, company),
                        (false, true) => l10n.auto_top_up_suspended_intro(&amount),
                        (false, false) => {
                            l10n.auto_top_up_suspended_intro_contact(&amount, company)
                        }
                    }
                    .to_string(),
                    l10n.auto_top_up_suspended_subject(company).to_string(),
                )
            } else {
                (
                    l10n.auto_top_up_failed_heading().to_string(),
                    if actions.can_top_up {
                        l10n.auto_top_up_failed_intro(&amount)
                    } else {
                        l10n.auto_top_up_failed_intro_contact(&amount, company)
                    }
                    .to_string(),
                    l10n.auto_top_up_failed_subject(company).to_string(),
                )
            };
            let content = WalletNoticeContent {
                heading,
                intro,
                summary: summary_box,
            };
            (
                subject.clone(),
                render_layout(lang, subject, preheader, header, footer, &l10n, content)?,
            )
        }
        Body::Custom {
            html_fragment,
            summary,
            cta,
            hedge,
        } => {
            let Subject::Custom(subject) = &subject else {
                return Err(MailerServiceError::InvalidContent(
                    "a custom body needs a custom subject".to_string(),
                ));
            };
            // No preheader: the operator's first lines are the better inbox preview.
            let content = CustomBodyContent {
                html_fragment: style_fragment(&html_fragment),
                summary: summary.map(|s| SummaryBoxTemplate {
                    hero: Some(amount_due_hero(&l10n, lang, &s)),
                    rows: vec![row(l10n.invoice_number(), s.number)],
                }),
                cta: cta.map(|cta| button(cta, l10n.pay_invoice(), &color)),
                hedge: non_blank(&hedge),
            };
            return Ok(Rendered {
                subject: subject.clone(),
                body_html: render_layout(
                    lang,
                    subject.clone(),
                    None,
                    header,
                    footer,
                    &l10n,
                    content,
                )?,
            });
        }
    };

    let subject = match subject {
        Subject::Custom(s) => s,
        Subject::Fixed => fixed_subject,
    };
    Ok(Rendered { subject, body_html })
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::lang::SUPPORTED;

    /// What sailfish emits for `<%= %>`.
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#039;")
    }

    fn summary() -> InvoiceSummary {
        InvoiceSummary {
            number: "INV-2026-0042".into(),
            invoice_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            due_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            amount_minor: 120_000,
            currency: "EUR".into(),
            label: Some("Scale plan".into()),
        }
    }

    fn entity_footer() -> EntityFooterCtx {
        EntityFooterCtx {
            legal_name: "Acme SAS".into(),
            address_lines: vec!["1 rue de la Paix".into(), "75002 Paris".into()],
            vat_number: Some("FR12345678901".into()),
            legal_number: Some("123 456 789 RCS Paris".into()),
            powered_by: true,
        }
    }

    fn layout(lang: &'static str) -> LayoutCtx {
        LayoutCtx {
            lang,
            brand: BrandCtx {
                company_name: "Acme".into(),
                logo_url: Some("https://cdn.example/logo.png".into()),
                brand_color: Some("#1a2b3c".into()),
                brand_color_light: None,
            },
            footer: FooterCtx::Entity(entity_footer()),
            test_mode: false,
        }
    }

    fn wallet_actions(self_serve: bool) -> WalletActions {
        WalletActions {
            can_top_up: self_serve,
            can_manage_auto_top_up: self_serve,
        }
    }

    #[test]
    fn a_wallet_notice_asks_only_for_what_the_portal_offers() {
        let low = |actions: WalletActions| {
            render(
                &layout("en-US"),
                Subject::Fixed,
                Body::WalletBalanceLow {
                    available_minor: 4_250,
                    threshold_minor: 10_000,
                    currency: "EUR".into(),
                    credit_unit: None,
                    actions,
                },
            )
            .unwrap()
            .body_html
        };
        let self_serve = low(wallet_actions(true));
        assert!(
            self_serve.contains("Add credit to keep your usage covered."),
            "{self_serve}"
        );
        let contact = low(wallet_actions(false));
        assert!(contact.contains("Contact Acme to add credit."), "{contact}");
        assert!(!contact.contains("class=\"button\""), "no link: {contact}");

        let suspended = |actions: WalletActions| {
            render(
                &layout("en-US"),
                Subject::Fixed,
                Body::AutoTopUpFailed {
                    amount_minor: 50_000,
                    currency: "EUR".into(),
                    credit_unit: None,
                    suspended: true,
                    payment_reversed: false,
                    actions,
                },
            )
            .unwrap()
            .body_html
        };
        assert!(suspended(wallet_actions(true)).contains("top-up settings to resume it"));
        let contact = suspended(wallet_actions(false));
        assert!(contact.contains("Contact Acme to resume it."), "{contact}");
        assert!(!contact.contains("top-up settings"), "{contact}");

        // A suspension after a charge-back names the reversed payment, not the rule's top-up.
        let reversed = render(
            &layout("fr-FR"),
            Subject::Fixed,
            Body::AutoTopUpFailed {
                amount_minor: 5_000,
                currency: "EUR".into(),
                credit_unit: None,
                suspended: true,
                payment_reversed: true,
                actions: wallet_actions(true),
            },
        )
        .unwrap()
        .body_html;
        assert!(
            reversed.contains("car €50,00 d&#039;un paiement de recharge"),
            "the amount reversed: {reversed}"
        );
        assert!(reversed.contains("remboursés ou contestés"), "{reversed}");
        assert!(
            !reversed.contains("Votre recharge automatique de"),
            "{reversed}"
        );
    }

    fn bodies() -> Vec<(&'static str, Body)> {
        vec![
            (
                "invoice_ready",
                Body::InvoiceReady {
                    summary: summary(),
                    cta: Cta::new("https://pay.example/inv"),
                },
            ),
            ("invoice_paid", Body::InvoicePaid { summary: summary() }),
            (
                "quote_ready",
                Body::QuoteReady {
                    quote_number: "Q-7".into(),
                    expires_at: Some(NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()),
                    custom_message: Some("Looking forward".into()),
                    cta: Cta::new("https://portal.example/q"),
                },
            ),
            (
                "credit_note_issued",
                Body::CreditNoteIssued {
                    credit_note_number: "CN-3".into(),
                    invoice_number: Some("INV-2026-0042".into()),
                    amount_minor: 5_000,
                    currency: "EUR".into(),
                    cta: Some(Cta::new("https://portal.example/cn")),
                },
            ),
            (
                "payment_failed",
                Body::PaymentFailed {
                    summary: summary(),
                    cta: Cta::new("https://pay.example/retry"),
                },
            ),
            (
                "authentication_required",
                Body::AuthenticationRequired {
                    summary: summary(),
                    cta: Cta::new("https://pay.example/3ds"),
                },
            ),
            (
                "wallet_balance_low",
                Body::WalletBalanceLow {
                    available_minor: 4_250,
                    threshold_minor: 10_000,
                    currency: "EUR".into(),
                    credit_unit: None,
                    actions: wallet_actions(true),
                },
            ),
            (
                "budget_threshold",
                Body::BudgetAlert {
                    level_pct: 80,
                    used_minor: 80_000,
                    amount_minor: 100_000,
                    currency: "EUR".into(),
                    credit_unit: None,
                    window_start: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                    window_end: Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()),
                    stops_usage: false,
                },
            ),
            (
                "budget_exceeded",
                Body::BudgetAlert {
                    level_pct: 100,
                    used_minor: 100_000,
                    amount_minor: 100_000,
                    currency: "EUR".into(),
                    credit_unit: None,
                    window_start: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                    window_end: None,
                    stops_usage: true,
                },
            ),
            (
                "auto_top_up_failed",
                Body::AutoTopUpFailed {
                    amount_minor: 50_000,
                    currency: "EUR".into(),
                    credit_unit: None,
                    suspended: false,
                    payment_reversed: false,
                    actions: wallet_actions(true),
                },
            ),
            (
                "auto_top_up_suspended",
                Body::AutoTopUpFailed {
                    amount_minor: 50_000,
                    currency: "EUR".into(),
                    credit_unit: None,
                    suspended: true,
                    payment_reversed: false,
                    actions: wallet_actions(false),
                },
            ),
            (
                "auto_top_up_reversed",
                Body::AutoTopUpFailed {
                    amount_minor: 5_000,
                    currency: "EUR".into(),
                    credit_unit: None,
                    suspended: true,
                    payment_reversed: true,
                    actions: wallet_actions(true),
                },
            ),
        ]
    }

    #[test]
    fn every_kind_renders_in_every_locale() {
        for lang in SUPPORTED {
            let l10n = email_l10n(lang);
            let html_lang = format!("<html lang=\"{}\">", &lang[..2]);
            for (kind, body) in bodies() {
                let rendered = render(&layout(lang), Subject::Fixed, body).unwrap();
                let ctx = format!("{lang}/{kind}");
                let html = &rendered.body_html;
                assert!(html.starts_with(&html_lang), "{ctx}: {}", &html[..40]);
                assert!(!rendered.subject.trim().is_empty(), "{ctx}: subject");
                assert!(
                    !rendered.subject.contains(['\u{2068}', '\u{2069}'])
                        && !html.contains(['\u{2068}', '\u{2069}']),
                    "{ctx}: bidi isolation marks"
                );
                assert!(html.contains("alt=\"Acme\""), "{ctx}: brand");
                assert!(html.contains("https://cdn.example/logo.png"), "{ctx}: logo");
                assert!(html.contains("Acme SAS"), "{ctx}: legal name");
                assert!(html.contains("75002 Paris"), "{ctx}: address");
                assert!(html.contains("FR12345678901"), "{ctx}: vat");
                assert!(html.contains(&esc(&l10n.powered_by())), "{ctx}: powered-by");
                assert!(
                    html.contains("meteroid-logo-small.png"),
                    "{ctx}: powered-by logo"
                );
                if kind != "quote_ready" {
                    assert!(html.contains("class=\"hero-amount\""), "{ctx}: hero");
                    assert!(html.contains("mso-hide:all"), "{ctx}: preheader");
                }
                assert!(
                    !html.contains("Meteroid SAS"),
                    "{ctx}: meteroid footer leaked"
                );
                let (heading, button) = match kind {
                    "invoice_ready" => (l10n.invoice_ready_heading(), Some(l10n.view_and_pay())),
                    "invoice_paid" => (l10n.invoice_paid_heading(), None),
                    "quote_ready" => (l10n.quote_ready_heading(), Some(l10n.view_and_sign_quote())),
                    "credit_note_issued" => (
                        l10n.credit_note_issued_heading(),
                        Some(l10n.view_credit_note()),
                    ),
                    "payment_failed" => (
                        l10n.payment_failed_heading(),
                        Some(l10n.update_payment_method()),
                    ),
                    "authentication_required" => (
                        l10n.authentication_required_heading(),
                        Some(l10n.confirm_payment()),
                    ),
                    "wallet_balance_low" => (l10n.wallet_balance_low_heading(), None),
                    "budget_threshold" => (l10n.budget_threshold_heading("80%"), None),
                    "budget_exceeded" => (l10n.budget_exceeded_heading(), None),
                    "auto_top_up_failed" => (l10n.auto_top_up_failed_heading(), None),
                    "auto_top_up_suspended" | "auto_top_up_reversed" => {
                        (l10n.auto_top_up_suspended_heading(), None)
                    }
                    _ => unreachable!(),
                };
                assert!(
                    html.contains(&format!(">{}</h1>", esc(&heading))),
                    "{ctx}: heading"
                );
                match button {
                    Some(label) => assert!(
                        html.contains("bgcolor=\"#1a2b3c\"")
                            && html.contains(&format!(">{}</a>", esc(&label))),
                        "{ctx}: button"
                    ),
                    None => assert!(!html.contains("class=\"button\""), "{ctx}: no button"),
                }
            }
        }
    }

    #[test]
    fn a_budget_note_says_whether_the_budget_stops_usage() {
        let l10n = email_l10n("en-US");
        let note = |level_pct: u32, stops_usage: bool, ends: bool| {
            let html = render(
                &layout("en-US"),
                Subject::Fixed,
                Body::BudgetAlert {
                    level_pct,
                    used_minor: 1_000,
                    amount_minor: 1_000,
                    currency: "EUR".into(),
                    credit_unit: None,
                    window_start: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                    window_end: ends.then(|| NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()),
                    stops_usage,
                },
            )
            .unwrap()
            .body_html;
            [
                l10n.budget_notify_only(),
                l10n.budget_stops_usage_ahead(),
                l10n.budget_stops_usage_now(),
                l10n.budget_stops_usage_ahead_open(),
                l10n.budget_stops_usage_now_open(),
            ]
            .into_iter()
            .map(|text| html.contains(&esc(&text)))
            .collect::<Vec<_>>()
        };
        assert_eq!(note(80, false, true), [true, false, false, false, false]);
        assert_eq!(note(100, false, false), [true, false, false, false, false]);
        assert_eq!(note(80, true, true), [false, true, false, false, false]);
        assert_eq!(note(100, true, true), [false, false, true, false, false]);
        assert_eq!(note(80, true, false), [false, false, false, true, false]);
        assert_eq!(note(100, true, false), [false, false, false, false, true]);
    }

    #[test]
    fn a_credit_unit_reads_by_its_name_never_its_code() {
        let rendered = render(
            &layout("en-US"),
            Subject::Fixed,
            Body::WalletBalanceLow {
                available_minor: 100,
                threshold_minor: 50_000,
                currency: "HALYARD_CREDITS".into(),
                credit_unit: Some(EmailCreditUnit {
                    name: "Halyard credit".into(),
                    plural_name: "Halyard credits".into(),
                    precision: 2,
                }),
                actions: WalletActions::default(),
            },
        )
        .unwrap();
        let html = &rendered.body_html;
        assert!(html.contains("1 Halyard credit<"), "{html}");
        assert!(html.contains("500 Halyard credits"), "{html}");
        assert!(!html.contains("HALYARD_CREDITS"), "{html}");
        assert!(!rendered.subject.contains("HALYARD_CREDITS"));
    }

    #[test]
    fn subjects_and_dates_follow_the_locale() {
        let inv = |lang| {
            render(
                &layout(lang),
                Subject::Fixed,
                Body::InvoiceReady {
                    summary: summary(),
                    cta: Cta::new("https://pay.example/inv"),
                },
            )
            .unwrap()
        };
        let en = inv("en-US");
        assert_eq!(en.subject, "Your Acme invoice is ready");
        assert!(en.body_html.contains("March 31, 2026"), "{}", en.body_html);
        assert!(en.body_html.contains(">Due March 31, 2026</td>"));
        assert!(en.body_html.contains(">€1.200,00</td>"));
        assert!(en.body_html.contains(">Amount due</td>"));
        assert!(en.body_html.contains(">Scale plan</p>"));
        assert!(
            en.body_html
                .contains("€1.200,00 · Due March 31, 2026 · INV-2026-0042")
        );
        let fr = inv("fr-FR");
        assert_eq!(fr.subject, "Votre facture Acme est disponible");
        assert!(fr.body_html.contains("31 mars 2026"), "{}", fr.body_html);
        assert!(fr.body_html.starts_with("<html lang=\"fr\">"));
        assert!(fr.body_html.contains(">Échéance le 31 mars 2026</td>"));
        let de = inv("de-DE");
        assert!(de.body_html.contains("31. März 2026"), "{}", de.body_html);

        let cn = render(
            &layout("en-US"),
            Subject::Fixed,
            Body::CreditNoteIssued {
                credit_note_number: "CN-3".into(),
                invoice_number: None,
                amount_minor: 5_000,
                currency: "USD".into(),
                cta: None,
            },
        )
        .unwrap();
        assert_eq!(cn.subject, "Your Acme credit note CN-3");
        assert!(cn.body_html.contains("$50.00"));
        assert!(!cn.body_html.contains("Invoice number"));
        assert!(cn.body_html.contains(">Amount credited</td>"));
        assert!(!cn.body_html.contains("class=\"button\""));

        let quote = render(
            &layout("en-US"),
            Subject::Fixed,
            Body::QuoteReady {
                quote_number: "Q-7".into(),
                expires_at: None,
                custom_message: None,
                cta: Cta::new("https://portal.example/q"),
            },
        )
        .unwrap();
        assert_eq!(quote.subject, "Quote Q-7 from Acme");
        assert!(!quote.body_html.contains("Valid until"));
        assert!(!quote.body_html.contains("class=\"custom-message\""));

        let failed = render(
            &layout("en-US"),
            Subject::Fixed,
            Body::PaymentFailed {
                summary: summary(),
                cta: Cta::new("https://pay.example/retry"),
            },
        )
        .unwrap();
        assert_eq!(
            failed.subject,
            "Payment for invoice INV-2026-0042 could not be processed"
        );
        assert!(!failed.body_html.contains("Reason"), "{}", failed.body_html);
        assert!(failed.body_html.contains(">Payment failed</td>"));

        let paid = render(
            &layout("en-US"),
            Subject::Fixed,
            Body::InvoicePaid { summary: summary() },
        )
        .unwrap()
        .body_html;
        assert!(paid.contains(">Amount paid</td>"), "{paid}");
        assert!(paid.contains("class=\"status\""), "{paid}");
        assert!(paid.contains(">Paid</td>"), "{paid}");
        assert!(paid.contains(">€1.200,00</td>"), "{paid}");
        assert!(!paid.contains("Due March"), "{paid}");
        assert!(!paid.contains("class=\"button\""), "{paid}");

        let auth = render(
            &layout("en-US"),
            Subject::Fixed,
            Body::AuthenticationRequired {
                summary: summary(),
                cta: Cta::new("https://pay.example/3ds"),
            },
        )
        .unwrap();
        assert!(auth.body_html.contains("before charging €1.200,00"));
        assert!(
            auth.body_html
                .contains("If you have already paid, please disregard this email.")
        );
    }

    #[test]
    fn custom_subject_overrides_and_custom_body_requires_one() {
        let rendered = render(
            &layout("en-US"),
            Subject::Custom("Reminder: INV-2026-0042".into()),
            Body::InvoicePaid { summary: summary() },
        )
        .unwrap();
        assert_eq!(rendered.subject, "Reminder: INV-2026-0042");
        assert!(
            rendered
                .body_html
                .contains("<title>Your Acme receipt</title>")
        );

        let custom = || Body::Custom {
            html_fragment: "<p>Hi <strong>Acme</strong></p>".into(),
            summary: Some(summary()),
            cta: Some(Cta::new("https://pay.example/inv")),
            hedge: Some("If you have already paid, please disregard this email.".into()),
        };
        let err = render(&layout("en-US"), Subject::Fixed, custom()).unwrap_err();
        assert!(matches!(err, MailerServiceError::InvalidContent(_)));

        let rendered =
            render(&layout("fr-FR"), Subject::Custom("Rappel".into()), custom()).unwrap();
        let html = &rendered.body_html;
        assert!(html.contains(">Hi <strong>Acme</strong></p>"), "{html}");
        assert!(html.contains("<p style=\"margin:0 0 16px;"), "{html}");
        assert!(!html.contains("mso-hide:all"), "{html}");
        assert!(html.contains("<title>Rappel</title>"), "{html}");
        assert!(html.contains(">Montant dû</td>"), "{html}");
        assert!(html.contains("31 mars 2026"), "{html}");
        assert!(html.contains(">Payer la facture</a>"), "{html}");
        assert!(html.contains("please disregard"), "{html}");
        assert!(
            !html.contains(&esc(&email_l10n("fr-FR").invoice_date())),
            "{html}"
        );
    }

    #[test]
    fn meteroid_footer_and_default_button_colour() {
        let ctx = LayoutCtx {
            lang: "en-US",
            brand: BrandCtx {
                company_name: "Acme".into(),
                logo_url: None,
                brand_color: Some("red".into()),
                brand_color_light: None,
            },
            footer: FooterCtx::Meteroid,
            test_mode: false,
        };
        let html = render(
            &ctx,
            Subject::Fixed,
            Body::InvoiceReady {
                summary: summary(),
                cta: Cta {
                    label: "Pay now".into(),
                    url: "https://pay.example/inv".into(),
                },
            },
        )
        .unwrap()
        .body_html;
        assert!(html.contains("Meteroid SAS"), "{html}");
        assert!(!html.contains("class=\"brand-bar\""), "{html}");
        assert!(html.contains("bgcolor=\"#000000\""), "{html}");
        assert!(
            html.contains("color:#ffffff;text-decoration:none;\">Pay now</a>"),
            "{html}"
        );
        assert!(html.contains("class=\"monogram\""), "{html}");
        assert!(html.contains(">A</td>"), "{html}");
        assert!(html.contains(">Acme</td>"), "{html}");
    }

    fn render_with_colors(brand_color: &str, brand_color_light: Option<&str>) -> String {
        let mut ctx = layout("en-US");
        ctx.brand.logo_url = None;
        ctx.brand.brand_color = Some(brand_color.into());
        ctx.brand.brand_color_light = brand_color_light.map(Into::into);
        render(
            &ctx,
            Subject::Fixed,
            Body::InvoiceReady {
                summary: summary(),
                cta: Cta {
                    label: "Pay now".into(),
                    url: "https://pay.example/inv".into(),
                },
            },
        )
        .unwrap()
        .body_html
    }

    fn button_fill(html: &str) -> &str {
        let start = html.find("class=\"button\"").expect("button");
        let rest = &html[start..];
        let from = rest.find("bgcolor=\"").unwrap() + "bgcolor=\"".len();
        &rest[from..from + rest[from..].find('"').unwrap()]
    }

    #[test]
    fn dark_brand_colour_is_used_everywhere() {
        let html = render_with_colors("#1A2B3C", None);
        assert_eq!(button_fill(&html), "#1a2b3c", "{html}");
        assert!(html.contains("class=\"brand-bar\" height=\"4\" bgcolor=\"#1a2b3c\""));
        assert!(html.contains("class=\"monogram\""), "{html}");
        assert!(
            html.contains("background-color:#1a2b3c;font-size:15px"),
            "{html}"
        );
    }

    #[test]
    fn light_brand_colour_keeps_the_accent_and_falls_back_to_a_dark_button() {
        let html = render_with_colors("#C6F94E", None);
        assert_eq!(button_fill(&html), "#000000", "{html}");
        assert!(html.contains("color:#ffffff;text-decoration:none;\">Pay now</a>"));
        assert!(html.contains("class=\"brand-bar\" height=\"4\" bgcolor=\"#c6f94e\""));
        assert!(
            html.contains("background-color:#c6f94e;font-size:15px"),
            "{html}"
        );
    }

    #[test]
    fn explicit_light_background_colour_wins() {
        let html = render_with_colors("#C6F94E", Some("#2E7D32"));
        assert_eq!(button_fill(&html), "#2e7d32", "{html}");
        assert!(html.contains("class=\"brand-bar\" height=\"4\" bgcolor=\"#2e7d32\""));
        assert!(!html.contains("#c6f94e"), "{html}");

        let html = render_with_colors("#1a2b3c", Some("not-a-colour"));
        assert_eq!(button_fill(&html), "#1a2b3c", "{html}");
    }

    #[test]
    fn test_mode_banner_tops_the_card_only_when_set() {
        let body = || Body::InvoicePaid { summary: summary() };
        let html = render(&layout("en-US"), Subject::Fixed, body())
            .unwrap()
            .body_html;
        assert!(!html.contains("class=\"test-notice"), "{html}");
        assert!(
            html.contains("border-radius:11px 11px 0 0;\">&nbsp;</td>"),
            "{html}"
        );

        for lang in SUPPORTED {
            let mut ctx = layout(lang);
            ctx.test_mode = true;
            let html = render(&ctx, Subject::Fixed, body()).unwrap().body_html;
            let notice = esc(&email_l10n(lang).test_mode_notice());
            assert!(html.contains(&format!(">{notice}</td>")), "{lang}: {html}");
            let banner = html.find("class=\"test-notice").expect("banner");
            assert!(banner < html.find("class=\"brand-bar\"").unwrap(), "{lang}");
            assert!(
                html.contains("border-radius:0;\">&nbsp;</td>"),
                "{lang}: {html}"
            );
        }
    }

    #[test]
    fn powered_by_is_optional_and_values_are_escaped() {
        let mut ctx = layout("en-US");
        ctx.brand.company_name = "Acme & <Sons>".into();
        ctx.footer = FooterCtx::Entity(EntityFooterCtx {
            powered_by: false,
            legal_name: "<script>x</script>".into(),
            ..entity_footer()
        });
        let rendered = render(
            &ctx,
            Subject::Fixed,
            Body::InvoicePaid { summary: summary() },
        )
        .unwrap();
        let html = &rendered.body_html;
        assert!(!html.contains("Powered by"), "{html}");
        assert!(!html.contains("meteroid-logo-small.png"), "{html}");
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;x&lt;/script&gt;"), "{html}");
        assert!(html.contains("Acme &amp; &lt;Sons&gt;"), "{html}");
        assert_eq!(rendered.subject, "Your Acme & <Sons> receipt");
    }
}
