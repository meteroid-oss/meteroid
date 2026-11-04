use common_domain::ids::QuoteId;

use diesel_models::quote_adjustments::{
    QuoteAdjustmentComponentRow, QuoteAdjustmentComponentRowNew, QuoteAdjustmentRow,
    QuoteAdjustmentRowNew,
};

use crate::domain::plan_minimums::{CreateMinimumScope, MinimumScope};

/// Quote-level minimum-commitment override. When a quote with a minimum is converted to a
/// subscription, this overrides whatever the plan would have snapshotted onto the sub. The
/// resulting `subscription_minimum` row carries `is_override = true` so plan changes
/// preserve it.
#[derive(Debug, Clone, PartialEq)]
pub struct QuoteMinimum {
    pub quote_id: QuoteId,
    pub amount_subunit: i64,
    /// Scope over products that already exist at quote time (existing-product components/extras).
    pub scope: MinimumScope,
    /// Names of inline new-product extras also in scope; their products are only minted at
    /// conversion, so they're carried by name and resolved then (and surfaced for display/editing).
    pub pending_extra_product_names: Vec<String>,
}

impl QuoteMinimum {
    pub fn from_rows(
        row: QuoteAdjustmentRow,
        components: Vec<QuoteAdjustmentComponentRow>,
    ) -> Self {
        let scope = if row.includes_all_components {
            MinimumScope::AllComponents
        } else {
            MinimumScope::Products(components.into_iter().map(|c| c.product_id).collect())
        };
        Self {
            quote_id: row.quote_id,
            amount_subunit: row.amount_subunit,
            scope,
            pending_extra_product_names: row
                .pending_extra_product_names
                .into_iter()
                .flatten()
                .collect(),
        }
    }

    /// The scope as a subscription created from the quote takes it, extras still by name.
    pub fn create_scope(&self) -> CreateMinimumScope {
        match &self.scope {
            MinimumScope::AllComponents => CreateMinimumScope::AllComponents,
            MinimumScope::Products(product_ids) => CreateMinimumScope::Products {
                product_ids: product_ids.clone(),
                extra_product_names: self.pending_extra_product_names.clone(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuoteMinimumNew {
    pub quote_id: QuoteId,
    pub amount_subunit: i64,
    /// Unresolved scope: existing products by id + inline new-product extras by name.
    pub scope: CreateMinimumScope,
}

impl QuoteMinimumNew {
    pub fn to_row(&self) -> QuoteAdjustmentRowNew {
        let pending_extra_product_names = match &self.scope {
            CreateMinimumScope::Products {
                extra_product_names,
                ..
            } => extra_product_names.iter().cloned().map(Some).collect(),
            CreateMinimumScope::AllComponents => vec![],
        };
        QuoteAdjustmentRowNew {
            quote_id: self.quote_id,
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            amount_subunit: self.amount_subunit,
            includes_all_components: self.scope.is_all_components(),
            pending_extra_product_names,
        }
    }

    pub fn component_rows(&self) -> Vec<QuoteAdjustmentComponentRowNew> {
        match &self.scope {
            CreateMinimumScope::AllComponents => vec![],
            CreateMinimumScope::Products { product_ids, .. } => product_ids
                .iter()
                .copied()
                .map(|product_id| QuoteAdjustmentComponentRowNew {
                    quote_id: self.quote_id,
                    kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                    product_id,
                })
                .collect(),
        }
    }
}
