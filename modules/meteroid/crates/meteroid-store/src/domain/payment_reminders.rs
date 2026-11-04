pub use crate::domain::enums::{
    PaymentReminderStepAction, ReminderEnrolledVia, ReminderNotEnrolledReason, ReminderState,
    ReminderStopReason,
};
use crate::errors::{StoreError, StoreErrorReport};
use chrono::NaiveDateTime;
use common_domain::ids::{
    BaseId, CustomerId, InvoiceId, InvoicingEntityId, PaymentReminderSequenceId,
    PaymentReminderStepId, PlanId, SegmentId, TenantId,
};
use diesel_models::payment_reminders::{
    InvoicePaymentReminderRow, PaymentReminderSequenceRow, PaymentReminderStepRow,
};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_STEPS: usize = 20;
pub const MIN_OFFSET_DAYS: i32 = -30;
pub const MAX_OFFSET_DAYS: i32 = 365;
pub const FALLBACK_LOCALE: &str = "en-US";

// ---- audience -----------------------------------------------------------------------------

/// Audience JSON envelope; root predicates are an implicit AND. Unknown keys are a
/// deserialization error, which is how the leaf set stays closed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audience {
    pub predicates: Vec<AudiencePredicate>,
}

impl Audience {
    /// The audience as API clients see it: ids as public `cus_…` / `plan_…` strings, not UUIDs.
    pub fn to_api_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(&serde_json::json!({
            "predicates": self.predicates.iter().map(AudiencePredicate::to_api_value).collect::<serde_json::Result<Vec<_>>>()?,
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AudiencePredicate {
    Attribute {
        key: AudienceAttribute,
        values: Vec<String>,
    },
    Plan {
        plan_ids: Vec<PlanId>,
    },
    Customer {
        customer_ids: Vec<CustomerId>,
    },
    Amount {
        op: CmpOp,
        threshold: i64,
        currency: String,
    },
    Property {
        key: String,
        op: PropertyOp,
        #[serde(default)]
        values: Vec<String>,
        #[serde(default)]
        value: Option<serde_json::Value>,
    },
    /// Reserved: rejected at save.
    Segment {
        segment_id: SegmentId,
    },
    And {
        children: Vec<AudiencePredicate>,
    },
    Or {
        children: Vec<AudiencePredicate>,
    },
    Not {
        child: Box<AudiencePredicate>,
    },
}

impl AudiencePredicate {
    fn to_api_value(&self) -> serde_json::Result<serde_json::Value> {
        use serde_json::json;
        let children = |children: &[AudiencePredicate]| {
            children
                .iter()
                .map(Self::to_api_value)
                .collect::<serde_json::Result<Vec<_>>>()
        };
        Ok(match self {
            Self::Plan { plan_ids } => {
                json!({ "plan": { "plan_ids": plan_ids.iter().map(|id| id.as_base62()).collect::<Vec<_>>() } })
            }
            Self::Customer { customer_ids } => json!({
                "customer": { "customer_ids": customer_ids.iter().map(|id| id.as_base62()).collect::<Vec<_>>() }
            }),
            Self::Segment { segment_id } => {
                json!({ "segment": { "segment_id": segment_id.as_base62() } })
            }
            Self::And { children: c } => json!({ "and": { "children": children(c)? } }),
            Self::Or { children: c } => json!({ "or": { "children": children(c)? } }),
            Self::Not { child } => json!({ "not": { "child": child.to_api_value()? } }),
            other => serde_json::to_value(other)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudienceAttribute {
    PlanType,
    InvoiceKind,
    PaymentMethodType,
    Currency,
    Country,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CmpOp {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyOp {
    In,
    Any,
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
    Set,
    NotSet,
}

// ---- per-locale copy ----------------------------------------------------------------------

/// `body` = restricted Markdown + `{{vars}}`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailCopy {
    pub subject: String,
    pub body: String,
}

/// Keyed by template locale (`common_domain::lang::SUPPORTED`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct EmailCopyByLocale(pub BTreeMap<String, EmailCopy>);

impl EmailCopyByLocale {
    /// Whole-entry fallback: exact → entity effective locale → `en-US` → first entry.
    pub fn resolve(&self, template_lang: &str, entity_lang: &str) -> Option<(&str, &EmailCopy)> {
        [template_lang, entity_lang, FALLBACK_LOCALE]
            .into_iter()
            .find_map(|lang| self.0.get_key_value(lang))
            .or_else(|| self.0.iter().next())
            .map(|(k, v)| (k.as_str(), v))
    }
}

// ---- sequences ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, o2o)]
#[try_from_owned(PaymentReminderStepRow, StoreErrorReport)]
pub struct PaymentReminderStep {
    pub id: PaymentReminderStepId,
    pub offset_days: i32,
    #[from(~.into())]
    pub action: PaymentReminderStepAction,
    #[from(serde_json::from_value(~).map_err(|e| {
        StoreError::SerdeError("Failed to deserialize payment_reminder_step content".to_string(), e)
    })?)]
    pub content: EmailCopyByLocale,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaymentReminderSequence {
    pub id: PaymentReminderSequenceId,
    pub tenant_id: TenantId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub name: String,
    pub position: i32,
    pub enabled: bool,
    /// `None` = the entity's default sequence, matches every invoice.
    pub audience: Option<Audience>,
    /// Ordered by `offset_days`.
    pub steps: Vec<PaymentReminderStep>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
}

impl PaymentReminderSequence {
    pub fn is_default(&self) -> bool {
        self.audience.is_none()
    }

    pub fn from_rows(
        row: PaymentReminderSequenceRow,
        steps: Vec<PaymentReminderStepRow>,
    ) -> Result<Self, StoreErrorReport> {
        let audience = row
            .audience
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| {
                StoreError::SerdeError(
                    "Failed to deserialize payment_reminder_sequence audience".to_string(),
                    e,
                )
            })?;
        let mut steps = steps
            .into_iter()
            .map(PaymentReminderStep::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        steps.sort_by_key(|s| s.offset_days);
        Ok(Self {
            id: row.id,
            tenant_id: row.tenant_id,
            invoicing_entity_id: row.invoicing_entity_id,
            name: row.name,
            position: row.position,
            enabled: row.enabled,
            audience,
            steps,
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentReminderStepInput {
    pub offset_days: i32,
    pub content: EmailCopyByLocale,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaymentReminderSequenceNew {
    pub invoicing_entity_id: InvoicingEntityId,
    pub name: String,
    pub enabled: bool,
    pub audience: Option<Audience>,
    pub steps: Vec<PaymentReminderStepInput>,
}

/// `steps` are replaced wholesale (safe: `offset_days` is the step identity).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaymentReminderSequencePatch {
    pub id: PaymentReminderSequenceId,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    /// `Some(None)` clears the audience (only legal for the default).
    pub audience: Option<Option<Audience>>,
    pub steps: Option<Vec<PaymentReminderStepInput>>,
}

// ---- per-invoice state --------------------------------------------------------------------

/// Local send hour in the entity's zone; execution is gated to `[hour, hour + 6h)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendWindow {
    pub tz: chrono_tz::Tz,
    pub hour: u8,
}

impl std::fmt::Display for SendWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02}:00 {}", self.hour, self.tz.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, o2o)]
#[from_owned(InvoicePaymentReminderRow)]
pub struct InvoicePaymentReminder {
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    #[from(~.into())]
    pub state: ReminderState,
    #[from(~.map(Into::into))]
    pub not_enrolled_reason: Option<ReminderNotEnrolledReason>,
    pub resolved_at: NaiveDateTime,
    pub generation: i32,
    pub sequence_id: Option<PaymentReminderSequenceId>,
    #[from(~.map(Into::into))]
    pub enrolled_via: Option<ReminderEnrolledVia>,
    pub enrolled_at: Option<NaiveDateTime>,
    pub last_offset_days: Option<i32>,
    pub next_reminder_at: Option<NaiveDateTime>,
    pub paused_until: Option<NaiveDateTime>,
    pub stopped_at: Option<NaiveDateTime>,
    #[from(~.map(Into::into))]
    pub stop_reason: Option<ReminderStopReason>,
    pub completed_at: Option<NaiveDateTime>,
    pub ended_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceRef {
    pub id: PaymentReminderSequenceId,
    pub name: String,
    pub enabled: bool,
    pub archived: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceReminderStepStatus {
    Sent,
    SkippedCatchUp,
    Scheduled,
    Due,
    NotSchedulable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceReminderStepView {
    pub offset_days: i32,
    pub fire_at: NaiveDateTime,
    pub status: InvoiceReminderStepStatus,
    pub sent_email_id: Option<common_domain::ids::EntityActivityId>,
}

/// Enum order is the precedence when several reasons apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NotEligibleReason {
    NotPayable,
    NoDueDate,
    NoPdf,
    ConsolidatedChild,
    AutoCharged,
    CustomerOptedOut,
    Stopped,
    TenantInactive,
    EmailsDisabled,
    KindDisabled,
    NoEnabledSequence,
    LadderStale,
}

/// Read model behind `GetInvoiceReminders`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceReminderState {
    pub sequence: Option<SequenceRef>,
    pub enrolled_at: Option<NaiveDateTime>,
    pub enrolled_via: Option<ReminderEnrolledVia>,
    pub last_offset_days: Option<i32>,
    pub next_reminder_at: Option<NaiveDateTime>,
    pub paused_until: Option<NaiveDateTime>,
    pub stopped_at: Option<NaiveDateTime>,
    pub stop_reason: Option<ReminderStopReason>,
    pub completed_at: Option<NaiveDateTime>,
    pub ended_at: Option<NaiveDateTime>,
    pub generation: i32,
    pub steps: Vec<InvoiceReminderStepView>,
    pub not_eligible_reason: Option<NotEligibleReason>,
    pub would_resolve_to: Option<SequenceRef>,
    pub would_resolve_via: Option<ReminderEnrolledVia>,
    pub send_window: SendWindow,
}

impl InvoiceReminderState {
    pub fn eligible(&self) -> bool {
        self.not_eligible_reason.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CustomerReminderPreference {
    pub opt_out: bool,
    pub sequence_id: Option<PaymentReminderSequenceId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(s: &str) -> EmailCopy {
        EmailCopy {
            subject: s.to_string(),
            body: format!("{s} body"),
        }
    }

    #[test]
    fn resolve_falls_back_whole_entry() {
        let mut by_locale = BTreeMap::new();
        by_locale.insert("en-US".to_string(), copy("en"));
        by_locale.insert("fr-FR".to_string(), copy("fr"));
        by_locale.insert("de-DE".to_string(), copy("de"));
        let c = EmailCopyByLocale(by_locale);

        assert_eq!(c.resolve("fr-FR", "de-DE").unwrap().0, "fr-FR");
        assert_eq!(c.resolve("it-IT", "de-DE").unwrap().0, "de-DE");
        assert_eq!(c.resolve("it-IT", "es-ES").unwrap().0, "en-US");

        let only_pt = EmailCopyByLocale(BTreeMap::from([("pt-BR".to_string(), copy("pt"))]));
        assert_eq!(only_pt.resolve("it-IT", "es-ES").unwrap().0, "pt-BR");
        assert!(
            EmailCopyByLocale::default()
                .resolve("en-US", "en-US")
                .is_none()
        );
    }

    #[test]
    fn api_json_uses_public_ids_and_parses_back() {
        let customer = CustomerId::new();
        let plan = PlanId::new();
        let audience = Audience {
            predicates: vec![AudiencePredicate::Not {
                child: Box::new(AudiencePredicate::Or {
                    children: vec![
                        AudiencePredicate::Customer {
                            customer_ids: vec![customer],
                        },
                        AudiencePredicate::Plan {
                            plan_ids: vec![plan],
                        },
                    ],
                }),
            }],
        };

        let json = audience.to_api_json().unwrap();

        assert!(json.contains(&customer.as_base62()));
        assert!(json.contains(&plan.as_base62()));
        assert!(!json.contains(&customer.as_uuid().to_string()));
        assert_eq!(serde_json::from_str::<Audience>(&json).unwrap(), audience);
    }

    #[test]
    fn audience_round_trips_and_rejects_unknown_leaves() {
        let json = serde_json::json!({ "predicates": [
            { "attribute": { "key": "plan_type", "values": ["STANDARD"] } },
            { "amount": { "op": "gte", "threshold": 50000, "currency": "EUR" } },
            { "property": { "key": "seats", "op": "set" } },
            { "not": { "child": { "customer": { "customer_ids": [] } } } },
            { "or": { "children": [ { "attribute": { "key": "country", "values": ["FR"] } } ] } }
        ]});
        let a: Audience = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(a.predicates.len(), 5);
        assert_eq!(
            serde_json::to_value(&a).unwrap()["predicates"][0],
            json["predicates"][0]
        );

        let unknown = serde_json::json!({ "predicates": [ { "tenant": { "ids": [] } } ] });
        assert!(serde_json::from_value::<Audience>(unknown).is_err());
    }

    #[test]
    fn send_window_displays_hour_and_zone() {
        let w = SendWindow {
            tz: chrono_tz::Europe::Paris,
            hour: 9,
        };
        assert_eq!(w.to_string(), "09:00 Europe/Paris");
    }
}
