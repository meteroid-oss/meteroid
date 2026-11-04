use common_domain::ids::{PlanVersionId, ProductId};

use diesel_models::plan_version_adjustments::{
    PlanVersionAdjustmentComponentRow, PlanVersionAdjustmentComponentRowNew,
    PlanVersionAdjustmentRow, PlanVersionAdjustmentRowNew,
};

/// Period minimum-commitment floor configured at the plan_version level.
///
/// `amount_subunit` is in the plan currency's minor unit (matches `LineItem::amount_subtotal`
/// so no conversion is needed at billing time). For USD: 100 subunits = $1.00. For JPY: 100
/// subunits = ¥100. For BTC: 100 subunits = 0.000001 BTC.
///
/// At invoice time (on `InvoiceType::Recurring` invoices only), the sum of lines whose
/// product is included by `scope` is compared to `amount_subunit` (prorated to the active
/// days of the period). If below, a "Minimum commitment" shortfall line tops up the invoice.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanMinimum {
    pub plan_version_id: PlanVersionId,
    pub amount_subunit: i64,
    pub scope: MinimumScope,
}

/// What counts toward the floor. Scoped by **product**, not price-component: a product is the
/// identity shared by plan components, operator overrides, and ad-hoc extra components, and is
/// carried on every invoice line — so a subscription's actual billed set is matched uniformly.
#[derive(Debug, Clone, PartialEq)]
pub enum MinimumScope {
    /// Every revenue line (all products, components AND add-ons) counts toward the floor.
    AllComponents,
    /// Only lines for the listed products count.
    Products(Vec<ProductId>),
}

impl MinimumScope {
    pub fn includes(&self, product_id: ProductId) -> bool {
        match self {
            MinimumScope::AllComponents => true,
            MinimumScope::Products(ids) => ids.contains(&product_id),
        }
    }

    /// The same products, whatever order they are listed in.
    pub fn same_as(&self, other: &MinimumScope) -> bool {
        match (self, other) {
            (MinimumScope::AllComponents, MinimumScope::AllComponents) => true,
            (MinimumScope::Products(a), MinimumScope::Products(b)) => same_set(a, b),
            _ => false,
        }
    }
}

fn same_set<T: Eq + std::hash::Hash>(a: &[T], b: &[T]) -> bool {
    a.iter().collect::<std::collections::HashSet<_>>()
        == b.iter().collect::<std::collections::HashSet<_>>()
}

impl PlanMinimum {
    pub fn from_rows(
        row: PlanVersionAdjustmentRow,
        components: Vec<PlanVersionAdjustmentComponentRow>,
    ) -> Self {
        let scope = if row.includes_all_components {
            MinimumScope::AllComponents
        } else {
            MinimumScope::Products(components.into_iter().map(|c| c.product_id).collect())
        };
        Self {
            plan_version_id: row.plan_version_id,
            amount_subunit: row.amount_subunit,
            scope,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanMinimumNew {
    pub plan_version_id: PlanVersionId,
    pub amount_subunit: i64,
    pub scope: MinimumScope,
}

impl PlanMinimumNew {
    pub fn to_row(&self) -> PlanVersionAdjustmentRowNew {
        PlanVersionAdjustmentRowNew {
            plan_version_id: self.plan_version_id,
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            amount_subunit: self.amount_subunit,
            includes_all_components: matches!(self.scope, MinimumScope::AllComponents),
        }
    }

    pub fn component_rows(&self) -> Vec<PlanVersionAdjustmentComponentRowNew> {
        match &self.scope {
            MinimumScope::AllComponents => vec![],
            MinimumScope::Products(ids) => ids
                .iter()
                .copied()
                .map(|product_id| PlanVersionAdjustmentComponentRowNew {
                    plan_version_id: self.plan_version_id,
                    kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                    product_id,
                })
                .collect(),
        }
    }
}

/// Input used by `replace_plan_version`: references scoped components by NAME (resolved to their
/// product ids in the same transaction that recreates the components and writes the minimum).
#[derive(Debug, Clone)]
pub struct ReplacePlanMinimumInput {
    pub amount_subunit: i64,
    pub scope: ReplacePlanMinimumScope,
}

/// Create-time minimum override scope, as received before the request's ad-hoc extras have been
/// materialized into products. Existing products are referenced by id directly; inline
/// new-product extras are referenced by their (local) name and resolved to the freshly-minted
/// product id inside the create transaction via [`CreateMinimumScope::resolve`].
#[derive(Debug, Clone)]
pub enum CreateMinimumScope {
    AllComponents,
    Products {
        product_ids: Vec<ProductId>,
        /// Names of new-product extras in the same request to fold into the scope once created.
        extra_product_names: Vec<String>,
    },
}

impl CreateMinimumScope {
    pub fn is_all_components(&self) -> bool {
        matches!(self, CreateMinimumScope::AllComponents)
    }

    pub fn same_as(&self, other: &CreateMinimumScope) -> bool {
        match (self, other) {
            (CreateMinimumScope::AllComponents, CreateMinimumScope::AllComponents) => true,
            (
                CreateMinimumScope::Products {
                    product_ids: a,
                    extra_product_names: a_names,
                },
                CreateMinimumScope::Products {
                    product_ids: b,
                    extra_product_names: b_names,
                },
            ) => same_set(a, b) && same_set(a_names, b_names),
            _ => false,
        }
    }

    /// Resolve to a final [`MinimumScope`] against the request's materialized components, given as
    /// `(name, product_id)` pairs. Each referenced extra name must map to exactly one product:
    /// - missing → error (unknown extra),
    /// - matches several distinct products (duplicate/ambiguous names) → error asking for a unique
    ///   name. This keeps the by-name correlation safe without a separate client-minted ref.
    pub fn resolve(&self, named_products: &[(String, ProductId)]) -> Result<MinimumScope, String> {
        match self {
            CreateMinimumScope::AllComponents => Ok(MinimumScope::AllComponents),
            CreateMinimumScope::Products {
                product_ids,
                extra_product_names,
            } => {
                let mut ids = product_ids.clone();
                for name in extra_product_names {
                    let distinct: std::collections::HashSet<ProductId> = named_products
                        .iter()
                        .filter(|(n, _)| n == name)
                        .map(|(_, p)| *p)
                        .collect();
                    match distinct.len() {
                        0 => {
                            return Err(format!(
                                "minimum_commitment scope references unknown extra component '{}'",
                                name
                            ));
                        }
                        1 => ids.push(*distinct.iter().next().unwrap()),
                        _ => {
                            return Err(format!(
                                "minimum_commitment scope references the ambiguous extra component \
                                 name '{}' (used by more than one component); rename it so the \
                                 scoped extra has a unique name",
                                name
                            ));
                        }
                    }
                }
                Ok(MinimumScope::Products(ids))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ReplacePlanMinimumScope {
    AllComponents,
    /// Component names whose products are scoped (resolved to product ids at insert time).
    Products(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    #[test]
    fn resolve_all_components_ignores_inputs() {
        assert!(matches!(
            CreateMinimumScope::AllComponents.resolve(&[]).unwrap(),
            MinimumScope::AllComponents
        ));
    }

    #[test]
    fn resolve_merges_existing_ids_with_pending_extra_name() {
        let existing = ProductId::new();
        let extra = ProductId::new();
        let scope = CreateMinimumScope::Products {
            product_ids: vec![existing],
            extra_product_names: vec!["Overage".into()],
        };
        let named = vec![("Overage".to_string(), extra)];
        match scope.resolve(&named).unwrap() {
            MinimumScope::Products(ids) => {
                assert_eq!(ids.len(), 2);
                assert!(ids.contains(&existing) && ids.contains(&extra));
            }
            _ => panic!("expected Products"),
        }
    }

    #[test]
    fn resolve_unknown_extra_name_errors() {
        let scope = CreateMinimumScope::Products {
            product_ids: vec![],
            extra_product_names: vec!["ghost".into()],
        };
        assert!(scope.resolve(&[]).is_err());
    }

    #[test]
    fn resolve_ambiguous_extra_name_errors() {
        // Two components share the name but resolve to DIFFERENT products → ambiguous.
        let scope = CreateMinimumScope::Products {
            product_ids: vec![],
            extra_product_names: vec!["dup".into()],
        };
        let named = vec![
            ("dup".to_string(), ProductId::new()),
            ("dup".to_string(), ProductId::new()),
        ];
        assert!(scope.resolve(&named).is_err());
    }

    #[test]
    fn resolve_same_name_same_product_is_not_ambiguous() {
        let p = ProductId::new();
        let scope = CreateMinimumScope::Products {
            product_ids: vec![],
            extra_product_names: vec!["dup".into()],
        };
        let named = vec![("dup".to_string(), p), ("dup".to_string(), p)];
        match scope.resolve(&named).unwrap() {
            MinimumScope::Products(ids) => assert_eq!(ids, vec![p]),
            _ => panic!("expected Products"),
        }
    }

    #[test]
    fn a_scope_is_the_same_whatever_the_order_of_its_products() {
        let (a, b) = (ProductId::new(), ProductId::new());
        assert!(MinimumScope::Products(vec![a, b]).same_as(&MinimumScope::Products(vec![b, a])));
        assert!(!MinimumScope::Products(vec![a]).same_as(&MinimumScope::Products(vec![a, b])));
        assert!(!MinimumScope::Products(vec![]).same_as(&MinimumScope::AllComponents));

        let scope = |ids: Vec<ProductId>, names: &[&str]| CreateMinimumScope::Products {
            product_ids: ids,
            extra_product_names: names.iter().map(|n| n.to_string()).collect(),
        };
        assert!(scope(vec![a, b], &["x", "y"]).same_as(&scope(vec![b, a], &["y", "x"])));
        assert!(!scope(vec![a], &["x"]).same_as(&scope(vec![a], &["y"])));
        assert!(CreateMinimumScope::AllComponents.same_as(&CreateMinimumScope::AllComponents));
    }
}
