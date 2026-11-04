use crate::model::{AlertTriggered, EmailValidationLink, OrgInvite, ResetPasswordLink};
use crate::render::{BrandCtx, EntityFooterCtx, FooterCtx};
use sailfish::TemplateSimple;
use secrecy::ExposeSecret;

#[derive(TemplateSimple)]
#[template(path = "reset_password_link.html")]
pub struct ResetPasswordLinkTemplate {
    pub reset_url: String,
    pub url_expires_in: String,
}

pub const METEROID_WORDMARK_URL: &str =
    "https://static.meteroid.com/assets/mail-images/meteroid-logo-wordmark--light.png";

pub const DEFAULT_BUTTON_COLOR: &str = "#000000";

/// Intrinsic 504x90 wordmark, shown at 24px high.
const METEROID_WORDMARK_SIZE: (u32, u32) = (134, 24);

impl From<ResetPasswordLink> for ResetPasswordLinkTemplate {
    fn from(link: ResetPasswordLink) -> Self {
        ResetPasswordLinkTemplate {
            reset_url: link.url.expose_secret().to_string(),
            url_expires_in: format_duration(link.url_expires_in),
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "email_validation_link.stpl")]
pub struct EmailValidationLinkContent {
    pub validation_url: String,
    pub url_expires_in: String,
    pub user: String,
}

pub struct EmailValidationLinkTemplate {
    pub tpl: LayoutTemplate<EmailValidationLinkContent, FooterTemplate>,
}

impl From<EmailValidationLink> for EmailValidationLinkTemplate {
    fn from(link: EmailValidationLink) -> Self {
        let user = link.recipient.first_name.unwrap_or(
            link.recipient
                .email
                .split('@')
                .next()
                .map(std::string::ToString::to_string)
                .unwrap_or(link.recipient.email),
        );

        let content = EmailValidationLinkContent {
            validation_url: link.url.expose_secret().to_string(),
            url_expires_in: format_duration(link.url_expires_in),
            user,
        };
        EmailValidationLinkTemplate {
            tpl: LayoutTemplate::meteroid("Confirm your email".to_string(), content),
        }
    }
}

fn format_duration(duration: chrono::Duration) -> String {
    if duration.num_days() > 1 {
        format!("{} days", duration.num_days())
    } else if duration.num_hours() > 1 {
        format!("{} hours", duration.num_hours())
    } else {
        format!("{} minutes", duration.num_minutes())
    }
}

fn format_date(date: chrono::NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

pub fn format_currency(amount: i64, currency: &str) -> String {
    let cur = meteroid_money::iso::find(currency).unwrap_or(meteroid_money::iso::USD);
    let amount = meteroid_money::Money::from_minor(amount, cur);
    format!("{amount}")
}

/// `amount` minor units of `unit` at its precision, grouped by thousands and named, e.g.
/// `12,500.5 Halyard credits`; singular for exactly one.
pub fn format_credit_units(amount: i64, unit: &crate::render::EmailCreditUnit) -> String {
    let sign = if amount < 0 { "-" } else { "" };
    let digits = amount.unsigned_abs().to_string();
    let precision = usize::from(unit.precision);
    let padded = format!("{digits:0>width$}", width = precision + 1);
    let (whole, fraction) = padded.split_at(padded.len() - precision);
    let mut grouped = String::with_capacity(whole.len() + whole.len() / 3);
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let one = 10u64.checked_pow(u32::from(unit.precision)) == Some(amount.unsigned_abs());
    let name = if one { &unit.name } else { &unit.plural_name };
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        format!("{sign}{grouped} {name}")
    } else {
        format!("{sign}{grouped}.{fraction} {name}")
    }
}

fn non_blank(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Lowercased `#rgb` / `#rrggbb`, or `None`, so the value can be inlined in a `style` attribute.
pub fn hex_color(color: Option<&str>) -> Option<String> {
    color
        .map(str::trim)
        .filter(|c| {
            let hex = c.strip_prefix('#').unwrap_or("");
            matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .map(str::to_ascii_lowercase)
}

/// WCAG relative luminance of a `#rgb` / `#rrggbb` colour (0 = black, 1 = white).
pub fn relative_luminance(color: &str) -> f64 {
    let hex = color.trim().trim_start_matches('#');
    let hex: String = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect(),
        _ => hex.to_string(),
    };
    let channel = |i: usize| {
        let c = f64::from(u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0))
            / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

/// WCAG contrast ratio between two colours, from 1 (same) to 21 (black on white).
pub fn contrast_ratio(a: &str, b: &str) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// WCAG non-text threshold (3:1) against the white email card.
pub fn is_legible_on_white(color: &str) -> bool {
    contrast_ratio(color, "#ffffff") >= 3.0
}

/// Text colour readable on a `#rgb` / `#rrggbb` fill: near-black on light fills, white otherwise.
pub fn on_color(fill: &str) -> &'static str {
    if relative_luminance(fill) > 0.179 {
        "#111318"
    } else {
        "#ffffff"
    }
}

/// The two brand colours of an email, decided once for the header and the buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmailColors {
    /// Top bar and monogram; `None` keeps the neutral header.
    pub accent: Option<String>,
    pub button: String,
}

impl EmailColors {
    /// An explicit light-background colour wins; else the brand colour, with a dark button
    /// when it fails 3:1 on white (the bar and monogram keep it as an accent).
    pub fn for_brand(brand: &BrandCtx) -> Self {
        if let Some(light) = hex_color(brand.brand_color_light.as_deref()) {
            return EmailColors {
                accent: Some(light.clone()),
                button: light,
            };
        }
        match hex_color(brand.brand_color.as_deref()) {
            Some(c) if is_legible_on_white(&c) => EmailColors {
                accent: Some(c.clone()).filter(|c| c != DEFAULT_BUTTON_COLOR),
                button: c,
            },
            Some(c) => EmailColors {
                accent: Some(c),
                button: DEFAULT_BUTTON_COLOR.to_string(),
            },
            None => EmailColors {
                accent: None,
                button: DEFAULT_BUTTON_COLOR.to_string(),
            },
        }
    }
}

/// The `lang` attribute: primary subtag of a template locale (`fr-FR` → `fr`).
pub fn html_lang(lang: &str) -> String {
    lang.split(['-', '_'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("en")
        .to_ascii_lowercase()
}

#[derive(TemplateSimple)]
#[template(path = "layout.stpl")]
pub struct LayoutTemplate<C: TemplateSimple, F: TemplateSimple> {
    pub lang: String,
    pub title: String,
    /// Inbox preview line, hidden in the body.
    pub preheader: Option<String>,
    pub header: HeaderTemplate,
    pub footer: F,
    pub content: C,
}

impl<C: TemplateSimple> LayoutTemplate<C, FooterTemplate> {
    /// Meteroid-branded English layout for account mails.
    pub fn meteroid(title: String, content: C) -> Self {
        LayoutTemplate {
            lang: "en".to_string(),
            title,
            preheader: None,
            header: HeaderTemplate {
                company_name: "Meteroid".to_string(),
                logo_url: Some(METEROID_WORDMARK_URL.to_string()),
                logo_size: Some(METEROID_WORDMARK_SIZE),
                brand_color: None,
                test_notice: None,
            },
            footer: FooterTemplate::new(),
            content,
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "header.stpl")]
pub struct HeaderTemplate {
    pub company_name: String,
    pub logo_url: Option<String>,
    /// `(width, height)` when the logo's size is known; tenant logos get a 32px height.
    pub logo_size: Option<(u32, u32)>,
    pub brand_color: Option<String>,
    /// Banner at the top of the card for mails sent from a non-production tenant.
    pub test_notice: Option<String>,
}

impl From<&BrandCtx> for HeaderTemplate {
    fn from(brand: &BrandCtx) -> Self {
        HeaderTemplate {
            company_name: brand.company_name.clone(),
            logo_url: brand.logo_url.clone(),
            logo_size: None,
            brand_color: EmailColors::for_brand(brand).accent,
            test_notice: None,
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "footer.stpl")]
pub struct FooterTemplate {
    pub current_year: i32,
}

impl FooterTemplate {
    pub fn new() -> Self {
        use chrono::Datelike;
        FooterTemplate {
            current_year: chrono::Utc::now().year(),
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "entity_footer.stpl")]
pub struct EntityFooterTemplate {
    pub legal_name: String,
    pub address_lines: Vec<String>,
    pub vat_number: Option<String>,
    pub legal_number: Option<String>,
    pub powered_by: Option<String>,
}

impl EntityFooterTemplate {
    pub fn new(ctx: &EntityFooterCtx, l10n: &crate::l10n::EmailL10N) -> Self {
        EntityFooterTemplate {
            legal_name: ctx.legal_name.clone(),
            address_lines: ctx
                .address_lines
                .iter()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
            vat_number: non_blank(&ctx.vat_number).map(str::to_string),
            legal_number: non_blank(&ctx.legal_number).map(str::to_string),
            powered_by: ctx.powered_by.then(|| l10n.powered_by().to_string()),
        }
    }
}

/// Renders `content` inside the layout with whichever footer `footer` selects.
pub fn render_layout<C: TemplateSimple>(
    lang: &str,
    title: String,
    preheader: Option<String>,
    header: HeaderTemplate,
    footer: &FooterCtx,
    l10n: &crate::l10n::EmailL10N,
    content: C,
) -> Result<String, sailfish::RenderError> {
    let lang = html_lang(lang);
    match footer {
        FooterCtx::Meteroid => LayoutTemplate {
            lang,
            title,
            preheader,
            header,
            footer: FooterTemplate::new(),
            content,
        }
        .render_once(),
        FooterCtx::Entity(entity) => LayoutTemplate {
            lang,
            title,
            preheader,
            header,
            footer: EntityFooterTemplate::new(entity, l10n),
            content,
        }
        .render_once(),
    }
}

/// A summary box: an optional headline amount, then label/value rows, all localized.
#[derive(TemplateSimple)]
#[template(path = "summary_box.stpl")]
pub struct SummaryBoxTemplate {
    pub hero: Option<SummaryHero>,
    pub rows: Vec<(String, String)>,
}

/// The one figure the email is about (amount due, paid, credited), set large.
pub struct SummaryHero {
    pub label: String,
    pub amount: String,
    /// A line under the amount, e.g. the localized due date.
    pub caption: Option<String>,
    pub status: Option<StatusBadge>,
}

/// A small coloured pill next to the hero label ("Paid", "Payment failed").
pub struct StatusBadge {
    pub label: String,
    pub color: &'static str,
    pub background: &'static str,
}

impl StatusBadge {
    pub fn success(label: String) -> Self {
        StatusBadge {
            label,
            color: "#0b6b3a",
            background: "#dcf3e5",
        }
    }

    pub fn danger(label: String) -> Self {
        StatusBadge {
            label,
            color: "#a8201a",
            background: "#fde7e5",
        }
    }

    pub fn warning(label: String) -> Self {
        StatusBadge {
            label,
            color: "#8a4b00",
            background: "#fdefd6",
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "button.stpl")]
pub struct ButtonTemplate {
    pub label: String,
    pub url: String,
    pub color: String,
}

#[derive(TemplateSimple)]
#[template(path = "invoice_ready.stpl")]
pub struct InvoiceReadyContent {
    pub heading: String,
    pub label: Option<String>,
    pub summary: SummaryBoxTemplate,
    pub cta: ButtonTemplate,
}

#[derive(TemplateSimple)]
#[template(path = "invoice_receipt.stpl")]
pub struct InvoicePaidContent {
    pub heading: String,
    pub label: Option<String>,
    pub summary: SummaryBoxTemplate,
}

#[derive(TemplateSimple)]
#[template(path = "quote_ready.stpl")]
pub struct QuoteReadyContent {
    pub heading: String,
    pub summary: SummaryBoxTemplate,
    pub custom_message: Option<String>,
    pub cta: ButtonTemplate,
}

#[derive(TemplateSimple)]
#[template(path = "credit_note_issued.stpl")]
pub struct CreditNoteIssuedContent {
    pub heading: String,
    pub summary: SummaryBoxTemplate,
    pub cta: Option<ButtonTemplate>,
}

#[derive(TemplateSimple)]
#[template(path = "payment_failed.stpl")]
pub struct PaymentFailedContent {
    pub heading: String,
    pub label: Option<String>,
    pub intro: String,
    pub summary: SummaryBoxTemplate,
    pub cta: ButtonTemplate,
}

#[derive(TemplateSimple)]
#[template(path = "authentication_required.stpl")]
pub struct AuthenticationRequiredContent {
    pub heading: String,
    pub label: Option<String>,
    pub summary: SummaryBoxTemplate,
    pub intro: String,
    pub cta: ButtonTemplate,
    pub hedge: String,
}

/// A prepaid-balance notice: no document and no button, the summary carries the figures.
#[derive(TemplateSimple)]
#[template(path = "wallet_notice.stpl")]
pub struct WalletNoticeContent {
    pub heading: String,
    pub intro: String,
    pub summary: SummaryBoxTemplate,
}

#[derive(TemplateSimple)]
#[template(path = "budget_alert.stpl")]
pub struct BudgetAlertContent {
    pub heading: String,
    pub intro: String,
    pub summary: SummaryBoxTemplate,
    pub note: String,
}

/// `html_fragment` is the single unescaped insertion point of the layout; it is only ever
/// fed by `copy::to_html`.
#[derive(TemplateSimple)]
#[template(path = "custom_body.stpl")]
pub struct CustomBodyContent {
    pub html_fragment: String,
    pub summary: Option<SummaryBoxTemplate>,
    pub cta: Option<ButtonTemplate>,
    pub hedge: Option<String>,
}

#[derive(TemplateSimple)]
#[template(path = "alert_triggered.stpl")]
pub struct AlertTriggeredContent {
    pub alert_name: String,
    pub severity: String,
    pub summary: String,
    pub metric_key: String,
    pub window_date: String,
    pub alerts_url: String,
}

pub struct AlertTriggeredTemplate {
    pub tpl: LayoutTemplate<AlertTriggeredContent, FooterTemplate>,
}

impl From<AlertTriggered> for AlertTriggeredTemplate {
    fn from(data: AlertTriggered) -> Self {
        let title = format!("{} · {}", data.severity, data.alert_name);
        let content = AlertTriggeredContent {
            alert_name: data.alert_name,
            severity: data.severity,
            summary: data.summary,
            metric_key: data.metric_key,
            window_date: format_date(data.window_date),
            alerts_url: data.alerts_url,
        };
        AlertTriggeredTemplate {
            tpl: LayoutTemplate::meteroid(title, content),
        }
    }
}

#[derive(TemplateSimple)]
#[template(path = "org_invite.stpl")]
pub struct OrgInviteContent {
    pub org_name: String,
    pub inviter_name: String,
    pub role: String,
    pub invite_url: String,
    pub expires_in: String,
}

pub struct OrgInviteTemplate {
    pub tpl: LayoutTemplate<OrgInviteContent, FooterTemplate>,
}

impl From<OrgInvite> for OrgInviteTemplate {
    fn from(data: OrgInvite) -> Self {
        let content = OrgInviteContent {
            org_name: data.org_name,
            inviter_name: data.inviter_name,
            role: data.role,
            invite_url: data.invite_url,
            expires_in: data.expires_in,
        };
        let tpl = LayoutTemplate::meteroid(
            format!("You've been invited to join {}", content.org_name),
            content,
        );
        OrgInviteTemplate { tpl }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EmailColors, FooterTemplate, contrast_ratio, hex_color, html_lang, is_legible_on_white,
        on_color,
    };
    use crate::render::BrandCtx;
    use chrono::Datelike;
    use sailfish::TemplateSimple;

    #[test]
    fn test_footer_renders_current_year() {
        let html = FooterTemplate::new().render_once().unwrap();
        let current_year = chrono::Utc::now().year();
        let expected = format!("© {}", current_year);
        assert!(
            html.contains(&expected),
            "footer should contain '{expected}'"
        );
    }

    #[test]
    fn hex_color_accepts_hex_only() {
        assert_eq!(hex_color(None), None);
        assert_eq!(hex_color(Some(" #1A2B3C ")).as_deref(), Some("#1a2b3c"));
        assert_eq!(hex_color(Some("#abc")).as_deref(), Some("#abc"));
        assert_eq!(hex_color(Some("red")), None);
        assert_eq!(hex_color(Some("#12345")), None);
        assert_eq!(hex_color(Some("#fff;color:red")), None);
        assert_eq!(hex_color(Some("")), None);
    }

    #[test]
    fn on_color_picks_a_readable_text_colour() {
        assert_eq!(on_color("#000000"), "#ffffff");
        assert_eq!(on_color("#4f46e5"), "#ffffff");
        assert_eq!(on_color("#1a2b3c"), "#ffffff");
        assert_eq!(on_color("#ffffff"), "#111318");
        assert_eq!(on_color("#fde047"), "#111318");
        assert_eq!(on_color("#fff"), "#111318");
    }

    #[test]
    fn contrast_follows_wcag() {
        assert!((contrast_ratio("#000000", "#ffffff") - 21.0).abs() < 1e-9);
        assert!((contrast_ratio("#fff", "#ffffff") - 1.0).abs() < 1e-9);
        assert!(is_legible_on_white("#1a2b3c"));
        assert!(is_legible_on_white("#4f46e5"));
        assert!(is_legible_on_white("#767676"));
        assert!(!is_legible_on_white("#c6f94e"));
        assert!(!is_legible_on_white("#fde047"));
        assert!(!is_legible_on_white("#ffffff"));
    }

    #[test]
    fn email_colors_pick_accent_and_button() {
        let colors = |brand: Option<&str>, light: Option<&str>| {
            EmailColors::for_brand(&BrandCtx {
                brand_color: brand.map(Into::into),
                brand_color_light: light.map(Into::into),
                ..Default::default()
            })
        };
        let c = colors(Some("#1A2B3C"), None);
        assert_eq!(
            (c.accent.as_deref(), c.button.as_str()),
            (Some("#1a2b3c"), "#1a2b3c")
        );
        let c = colors(Some("#C6F94E"), None);
        assert_eq!(
            (c.accent.as_deref(), c.button.as_str()),
            (Some("#c6f94e"), "#000000")
        );
        let c = colors(Some("#C6F94E"), Some("#2e7d32"));
        assert_eq!(
            (c.accent.as_deref(), c.button.as_str()),
            (Some("#2e7d32"), "#2e7d32")
        );
        let c = colors(Some("#1a2b3c"), Some(""));
        assert_eq!(c.button, "#1a2b3c");
        let c = colors(Some("red"), None);
        assert_eq!((c.accent, c.button.as_str()), (None, "#000000"));
    }

    #[test]
    fn html_lang_is_the_primary_subtag() {
        assert_eq!(html_lang("fr-FR"), "fr");
        assert_eq!(html_lang("pt-BR"), "pt");
        assert_eq!(html_lang("en-US"), "en");
        assert_eq!(html_lang(""), "en");
    }
}

#[cfg(test)]
mod credit_unit_format_tests {
    use super::format_credit_units;
    use crate::render::EmailCreditUnit;

    fn unit(precision: u8) -> EmailCreditUnit {
        EmailCreditUnit {
            name: "Halyard credit".into(),
            plural_name: "Halyard credits".into(),
            precision,
        }
    }

    #[test]
    fn credit_units_read_in_whole_units_by_their_name() {
        assert_eq!(
            format_credit_units(12_500_500, &unit(3)),
            "12,500.5 Halyard credits"
        );
        assert_eq!(format_credit_units(500, &unit(0)), "500 Halyard credits");
        assert_eq!(format_credit_units(5, &unit(2)), "0.05 Halyard credits");
        assert_eq!(format_credit_units(100, &unit(2)), "1 Halyard credit");
        assert_eq!(format_credit_units(-1, &unit(0)), "-1 Halyard credit");
        assert_eq!(
            format_credit_units(-1_000_000, &unit(0)),
            "-1,000,000 Halyard credits"
        );
    }
}
