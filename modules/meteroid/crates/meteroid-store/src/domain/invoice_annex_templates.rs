use crate::errors::{StoreError, StoreErrorReport};
use chrono::NaiveDateTime;
use common_domain::ids::{InvoiceAnnexTemplateId, TenantId};
use diesel_models::enums::InvoiceAnnexKindEnum as DbAnnexKind;
use diesel_models::invoice_annex_templates::InvoiceAnnexTemplateRow;
use serde::{Deserialize, Serialize};

/// A reusable, tenant-scoped annex template. The `definition` is interpreted according to its
/// kind when resolving an annex onto an invoice.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnexTemplate {
    pub id: InvoiceAnnexTemplateId,
    pub tenant_id: TenantId,
    pub name: String,
    pub definition: AnnexDefinition,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone)]
pub struct AnnexTemplateNew {
    pub tenant_id: TenantId,
    pub name: String,
    pub definition: AnnexDefinition,
}

#[derive(Debug, Clone)]
pub struct AnnexTemplatePatch {
    pub id: InvoiceAnnexTemplateId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub definition: Option<AnnexDefinition>,
}

/// The typed annex definition. The DB stores the discriminant (`kind`) and the JSON body
/// separately; in the domain they are reunited into one tagged enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnnexDefinition {
    /// A table derived from the raw event stream.
    EventTable(EventTableDefinition),
    /// Static title + free text, no derived data.
    StaticText(StaticTextDefinition),
}

impl AnnexDefinition {
    pub fn kind(&self) -> DbAnnexKind {
        match self {
            AnnexDefinition::EventTable(_) => DbAnnexKind::EventTable,
            AnnexDefinition::StaticText(_) => DbAnnexKind::StaticText,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTableDefinition {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub source: EventSource,
    pub columns: Vec<AnnexColumn>,
    /// When non-empty, the annex switches to **aggregated mode**: events sharing the same value
    /// for every listed property collapse into a single row (e.g. `["client_name", "mission_id"]`
    /// → one row per mission per client). When empty, every event is listed individually.
    #[serde(default)]
    pub aggregate_by: Vec<String>,
    /// Property names used to group rows. In per-event mode these become group headings; in
    /// aggregated mode they drive per-group subtotal rows. Display-only; rows are never merged
    /// across the aggregation key.
    #[serde(default)]
    pub group_by: Vec<String>,
    /// Property names used to order rows (ascending), applied before grouping.
    #[serde(default)]
    pub sort_by: Vec<String>,
    /// Aggregated mode only: emit a "Total <group>" subtotal row at the end of each `group_by`
    /// group.
    #[serde(default)]
    pub show_group_subtotals: bool,
    /// Aggregated mode only: emit a grand "TOTAL" row at the end of the table.
    #[serde(default)]
    pub show_total: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventSource {
    pub event_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnexColumn {
    pub label: String,
    pub source: ColumnSource,
    #[serde(default)]
    pub transform: Option<ColumnTransform>,
    /// Aggregated mode only: how this column collapses the events of a row. Defaults to `first`
    /// (representative value, e.g. a label/dimension that is constant within the group).
    #[serde(default)]
    pub aggregation: Option<ColumnAggregation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnAggregation {
    /// Representative value from the first event (for constant dimensions/labels).
    First,
    /// Number of events in the row's group (e.g. number of billed units).
    Count,
    /// Numeric sum of the column's source value across the group's events.
    Sum,
    /// `min … max` of the events' timestamps, formatted via the column's `timestamp_format`
    /// (e.g. "Oct 2025 - Déc 2025"); collapses to a single label when min == max.
    PeriodRange,
    /// Billed amount for the row, derived from the invoice's usage line(s): row units × the
    /// effective unit rate. Best-effort — left blank when no usage line is attributable.
    BilledAmount,
}

impl ColumnAggregation {
    /// Columns whose values are numeric and therefore summable into subtotal/total rows.
    pub(crate) fn is_summable(self) -> bool {
        matches!(
            self,
            ColumnAggregation::Count | ColumnAggregation::Sum | ColumnAggregation::BilledAmount
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnSource {
    /// Value of an event property by name.
    Property(String),
    /// The event's timestamp.
    Timestamp,
    /// The price billed for this event. Only resolvable for simple per-unit pricing;
    /// left blank otherwise.
    BilledPrice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnTransform {
    /// `chrono` strftime format applied to a timestamp source (e.g. `%Y/%m`).
    TimestampFormat(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticTextDefinition {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
}

impl TryFrom<InvoiceAnnexTemplateRow> for AnnexTemplate {
    type Error = StoreErrorReport;

    fn try_from(row: InvoiceAnnexTemplateRow) -> Result<Self, Self::Error> {
        let definition = match row.kind {
            DbAnnexKind::EventTable => AnnexDefinition::EventTable(
                serde_json::from_value(row.definition)
                    .map_err(|e| StoreError::SerdeError("annex definition".to_string(), e))?,
            ),
            DbAnnexKind::StaticText => AnnexDefinition::StaticText(
                serde_json::from_value(row.definition)
                    .map_err(|e| StoreError::SerdeError("annex definition".to_string(), e))?,
            ),
        };

        Ok(AnnexTemplate {
            id: row.id,
            tenant_id: row.tenant_id,
            name: row.name,
            definition,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}
