use crate::StoreResult;
use crate::domain::Price;
use crate::domain::enums::BillingPeriodEnum;
use crate::domain::plan_options::{
    DEFAULT_OPTION_ALIAS, DEFAULT_OPTION_NAME, PlanOption, alias_from_name, cadence_option_name,
    cadence_options, in_cadence_option, valid_alias,
};
use crate::errors::StoreError;
use crate::repositories::price_components::lock_draft_version_tx;
use crate::store::{PgConn, Store};
use common_domain::ids::{
    BaseId, PlanOptionId, PlanVersionId, PriceComponentId, PriceId, TenantId,
};
use diesel_models::plan_component_prices::{PlanComponentPriceRow, PlanComponentPriceRowNew};
use diesel_models::plan_version_options::{PlanVersionOptionRow, PlanVersionOptionRowNew};
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;

/// The options of each of `versions`, by position.
pub(crate) async fn options_by_version(
    conn: &mut PgConn,
    tenant_id: TenantId,
    versions: &[PlanVersionId],
) -> StoreResult<HashMap<PlanVersionId, Vec<PlanOption>>> {
    let rows = PlanVersionOptionRow::list_by_plan_version_ids(conn, tenant_id, versions)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let mut by_version: HashMap<PlanVersionId, Vec<PlanOption>> = HashMap::new();
    for row in rows {
        by_version
            .entry(row.plan_version_id)
            .or_default()
            .push(row.into());
    }
    Ok(by_version)
}

pub(crate) async fn insert_default_option(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
) -> StoreResult<PlanOption> {
    PlanVersionOptionRowNew {
        id: PlanOptionId::new(),
        tenant_id,
        plan_version_id,
        name: DEFAULT_OPTION_NAME.to_owned(),
        alias: DEFAULT_OPTION_ALIAS.to_owned(),
        position: 0,
        hidden: false,
    }
    .insert(conn)
    .await
    .map(Into::into)
    .map_err(Into::into)
}

/// The options of a version being written, by position; a version without one (inserted
/// outside the store) gets the default, as every version has one.
pub(crate) async fn version_options(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
) -> StoreResult<Vec<PlanOption>> {
    let options: Vec<PlanOption> =
        PlanVersionOptionRow::list_by_plan_version_id(conn, tenant_id, plan_version_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(Into::into)
            .collect();
    if options.is_empty() {
        return Ok(vec![
            insert_default_option(conn, tenant_id, plan_version_id).await?,
        ]);
    }
    Ok(options)
}

/// The options and links of a version written whole from its charges' prices (API create or
/// replace): a charge priced on several cadences is split into an option per cadence, as the
/// options migration did; otherwise one default option holds every price.
pub(crate) async fn insert_options_and_links(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
    components: &[(PriceComponentId, Vec<PriceId>)],
) -> StoreResult<Vec<PlanOption>> {
    let mut price_ids: Vec<PriceId> = components
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect();
    price_ids.sort_by_key(|id| id.as_uuid());
    price_ids.dedup();
    let prices: HashMap<PriceId, Price> = if price_ids.is_empty() {
        HashMap::new()
    } else {
        PriceRow::list_by_ids(conn, &price_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|row| Price::try_from(row).map(|price| (price.id, price)))
            .collect::<Result<_, _>>()?
    };
    let per_component: Vec<Vec<&Price>> = components
        .iter()
        .map(|(_, ids)| {
            let mut own: Vec<&Price> = ids.iter().filter_map(|id| prices.get(id)).collect();
            own.sort_by_key(|price| price.id.as_uuid());
            own.dedup_by_key(|price| price.id);
            own
        })
        .collect();

    let cadences = cadence_options(&per_component);
    let options: Vec<(PlanVersionOptionRowNew, Option<BillingPeriodEnum>)> = if cadences.is_empty()
    {
        vec![(
            PlanVersionOptionRowNew {
                id: PlanOptionId::new(),
                tenant_id,
                plan_version_id,
                name: DEFAULT_OPTION_NAME.to_owned(),
                alias: DEFAULT_OPTION_ALIAS.to_owned(),
                position: 0,
                hidden: false,
            },
            None,
        )]
    } else {
        cadences
            .iter()
            .enumerate()
            .map(|(position, cadence)| {
                let (name, alias) = cadence_option_name(*cadence);
                (
                    PlanVersionOptionRowNew {
                        id: PlanOptionId::new(),
                        tenant_id,
                        plan_version_id,
                        name: name.to_owned(),
                        alias: alias.to_owned(),
                        position: position as i32,
                        hidden: false,
                    },
                    Some(*cadence),
                )
            })
            .collect()
    };
    let rows: Vec<PlanVersionOptionRowNew> = options.iter().map(|(row, _)| row.clone()).collect();
    let inserted = PlanVersionOptionRowNew::insert_batch(conn, &rows)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let mut links = Vec::new();
    for ((component_id, _), prices) in components.iter().zip(&per_component) {
        for (option, cadence) in &options {
            for price in prices {
                if cadence.is_none_or(|cadence| in_cadence_option(prices, price, cadence)) {
                    links.push(PlanComponentPriceRowNew {
                        plan_component_id: *component_id,
                        price_id: price.id,
                        option_id: option.id,
                    });
                }
            }
        }
    }
    PlanComponentPriceRowNew::insert_batch(conn, &links)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let mut inserted: Vec<PlanOption> = inserted.into_iter().map(Into::into).collect();
    inserted.sort_by_key(|option| option.position);
    Ok(inserted)
}

/// Copies the options of `src` into `dst` (same name, alias, position and visibility, new ids):
/// the old id of each to its copy.
pub(crate) async fn copy_options(
    conn: &mut PgConn,
    tenant_id: TenantId,
    src: PlanVersionId,
    dst: PlanVersionId,
) -> StoreResult<HashMap<PlanOptionId, PlanOptionId>> {
    let rows = PlanVersionOptionRow::list_by_plan_version_id(conn, tenant_id, src)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if rows.is_empty() {
        insert_default_option(conn, tenant_id, dst).await?;
        return Ok(HashMap::new());
    }
    let copies: Vec<PlanVersionOptionRowNew> = rows
        .iter()
        .map(|row| PlanVersionOptionRowNew {
            id: PlanOptionId::new(),
            tenant_id,
            plan_version_id: dst,
            name: row.name.clone(),
            alias: row.alias.clone(),
            position: row.position,
            hidden: row.hidden,
        })
        .collect();
    PlanVersionOptionRowNew::insert_batch(conn, &copies)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(rows
        .iter()
        .zip(copies.iter())
        .map(|(row, copy)| (row.id, copy.id))
        .collect())
}

fn checked_name(name: &str) -> StoreResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Report::new(StoreError::InvalidArgument(
            "An option needs a name".to_owned(),
        )));
    }
    Ok(name.to_owned())
}

fn checked_alias(alias: &str, others: &[&PlanOption]) -> StoreResult<String> {
    let alias = alias.trim();
    if !valid_alias(alias) {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "'{alias}' is not an alias: lowercase letters, digits, - and _, up to 40 characters"
        ))));
    }
    if others.iter().any(|other| other.alias == alias) {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Another option of this plan is already '{alias}'"
        ))));
    }
    Ok(alias.to_owned())
}

/// Refused for the default option (the first), and for the last one shown.
fn checked_hideable(option: &PlanOption, options: &[PlanOption]) -> StoreResult<()> {
    if options.first().is_some_and(|first| first.id == option.id) {
        return Err(Report::new(StoreError::InvalidArgument(
            "The default option is always shown".to_owned(),
        )));
    }
    if !options.iter().any(|o| o.id != option.id && !o.hidden) {
        return Err(Report::new(StoreError::InvalidArgument(
            "A plan keeps one option shown at least".to_owned(),
        )));
    }
    Ok(())
}

/// Refused when the option that would be the default (the first of `options`) is hidden.
fn checked_default_shown(options: &[&PlanOption]) -> StoreResult<()> {
    match options.first() {
        Some(first) if first.hidden => Err(Report::new(StoreError::InvalidArgument(format!(
            "The default option is always shown: show {} on the pricing page and portal first",
            first.name
        )))),
        _ => Ok(()),
    }
}

/// The option `option_id` of a draft version, locked, with every option of that version.
async fn lock_draft_option(
    conn: &mut PgConn,
    tenant_id: TenantId,
    option_id: PlanOptionId,
) -> StoreResult<(PlanOption, Vec<PlanOption>)> {
    let row = PlanVersionOptionRow::find_by_id(conn, tenant_id, option_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    lock_draft_version_tx(conn, row.plan_version_id, tenant_id).await?;
    let options = version_options(conn, tenant_id, row.plan_version_id).await?;
    Ok((row.into(), options))
}

#[async_trait::async_trait]
pub trait PlanOptionInterface {
    async fn list_plan_options(
        &self,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> StoreResult<Vec<PlanOption>>;

    /// A new option of a draft, its cells those of `copy_from` (none: the default option): the
    /// same prices, linked, never copied.
    async fn create_plan_option(
        &self,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
        name: String,
        alias: Option<String>,
        copy_from: Option<PlanOptionId>,
    ) -> StoreResult<PlanOption>;

    /// Hiding is refused for the default option, and for the last one shown.
    async fn update_plan_option(
        &self,
        tenant_id: TenantId,
        option_id: PlanOptionId,
        name: Option<String>,
        alias: Option<String>,
        hidden: Option<bool>,
    ) -> StoreResult<PlanOption>;

    /// Refused for the last option, and for one holding a charge's only prices: the charge
    /// would silently become included.
    async fn remove_plan_option(
        &self,
        tenant_id: TenantId,
        option_id: PlanOptionId,
    ) -> StoreResult<()>;

    /// `ids` is every option of the version once, in the new order.
    async fn reorder_plan_options(
        &self,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
        ids: Vec<PlanOptionId>,
    ) -> StoreResult<Vec<PlanOption>>;
}

#[async_trait::async_trait]
impl PlanOptionInterface for Store {
    async fn list_plan_options(
        &self,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> StoreResult<Vec<PlanOption>> {
        let mut conn = self.get_conn().await?;
        Ok(options_by_version(&mut conn, tenant_id, &[plan_version_id])
            .await?
            .remove(&plan_version_id)
            .unwrap_or_default())
    }

    async fn create_plan_option(
        &self,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
        name: String,
        alias: Option<String>,
        copy_from: Option<PlanOptionId>,
    ) -> StoreResult<PlanOption> {
        self.transaction(|conn| {
            async move {
                lock_draft_version_tx(conn, plan_version_id, tenant_id).await?;
                let options = version_options(conn, tenant_id, plan_version_id).await?;
                let name = checked_name(&name)?;
                let others: Vec<&PlanOption> = options.iter().collect();
                let alias = match alias {
                    Some(alias) => checked_alias(&alias, &others)?,
                    None => {
                        let taken: Vec<String> = options.iter().map(|o| o.alias.clone()).collect();
                        alias_from_name(&name, &taken)
                    }
                };
                let source = match copy_from {
                    None => &options[0],
                    Some(id) => options.iter().find(|o| o.id == id).ok_or_else(|| {
                        Report::new(StoreError::InvalidArgument(
                            "The option copied is not of this plan version".to_owned(),
                        ))
                    })?,
                };
                let position = options.iter().map(|o| o.position).max().unwrap_or(-1) + 1;

                let created: PlanOption = PlanVersionOptionRowNew {
                    id: PlanOptionId::new(),
                    tenant_id,
                    plan_version_id,
                    name,
                    alias,
                    position,
                    hidden: false,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into();

                let links: Vec<PlanComponentPriceRowNew> =
                    PlanComponentPriceRow::list_by_option_ids(conn, &[source.id])
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into_iter()
                        .map(|link| PlanComponentPriceRowNew {
                            plan_component_id: link.plan_component_id,
                            price_id: link.price_id,
                            option_id: created.id,
                        })
                        .collect();
                PlanComponentPriceRowNew::insert_batch(conn, &links)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(created)
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_plan_option(
        &self,
        tenant_id: TenantId,
        option_id: PlanOptionId,
        name: Option<String>,
        alias: Option<String>,
        hidden: Option<bool>,
    ) -> StoreResult<PlanOption> {
        self.transaction(|conn| {
            async move {
                let (option, options) = lock_draft_option(conn, tenant_id, option_id).await?;
                let name = match name {
                    Some(name) => checked_name(&name)?,
                    None => option.name.clone(),
                };
                let others: Vec<&PlanOption> =
                    options.iter().filter(|o| o.id != option.id).collect();
                let alias = match alias {
                    Some(alias) => checked_alias(&alias, &others)?,
                    None => option.alias.clone(),
                };
                if hidden == Some(true) && !option.hidden {
                    checked_hideable(&option, &options)?;
                }
                let mut updated: PlanOption = PlanVersionOptionRow::update_name_alias(
                    conn, tenant_id, option_id, &name, &alias,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into();
                if let Some(hidden) = hidden.filter(|hidden| *hidden != option.hidden) {
                    updated = PlanVersionOptionRow::set_hidden(conn, tenant_id, option_id, hidden)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into();
                }
                Ok(updated)
            }
            .scope_boxed()
        })
        .await
    }

    async fn remove_plan_option(
        &self,
        tenant_id: TenantId,
        option_id: PlanOptionId,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                let (option, options) = lock_draft_option(conn, tenant_id, option_id).await?;
                if options.len() <= 1 {
                    return Err(Report::new(StoreError::FailedPrecondition(
                        "A plan keeps one option at least".to_owned(),
                    )));
                }

                let components = PriceComponentRow::list_by_plan_version_id(
                    conn,
                    tenant_id,
                    option.plan_version_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                let ids: Vec<PriceComponentId> = components.iter().map(|c| c.id).collect();
                let links = PlanComponentPriceRow::list_by_component_ids(conn, &ids)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let priced_here_only = components.iter().find(|component| {
                    let own: Vec<&PlanComponentPriceRow> = links
                        .iter()
                        .filter(|l| l.plan_component_id == component.id)
                        .collect();
                    !own.is_empty() && own.iter().all(|l| l.option_id == option.id)
                });
                if let Some(component) = priced_here_only {
                    return Err(Report::new(StoreError::FailedPrecondition(format!(
                        "{} is priced in {} only. Price it in another option, or remove it, first.",
                        component.name, option.name
                    ))));
                }

                let kept: Vec<&PlanOption> = options.iter().filter(|o| o.id != option_id).collect();
                checked_default_shown(&kept)?;
                PlanVersionOptionRow::delete(conn, tenant_id, option_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                for (position, kept) in options.iter().filter(|o| o.id != option_id).enumerate() {
                    if kept.position != position as i32 {
                        PlanVersionOptionRow::set_position(
                            conn,
                            tenant_id,
                            kept.id,
                            position as i32,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    }
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn reorder_plan_options(
        &self,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
        ids: Vec<PlanOptionId>,
    ) -> StoreResult<Vec<PlanOption>> {
        self.transaction(|conn| {
            async move {
                lock_draft_version_tx(conn, plan_version_id, tenant_id).await?;
                let options = version_options(conn, tenant_id, plan_version_id).await?;
                let mut asked = ids.clone();
                asked.sort_by_key(|id| id.as_uuid());
                asked.dedup();
                if asked.len() != ids.len()
                    || ids.len() != options.len()
                    || !ids.iter().all(|id| options.iter().any(|o| o.id == *id))
                {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "Name every option of the plan version once".to_owned(),
                    )));
                }
                let first = options.iter().find(|o| Some(&o.id) == ids.first());
                checked_default_shown(&first.into_iter().collect::<Vec<_>>())?;
                for (position, id) in ids.iter().enumerate() {
                    PlanVersionOptionRow::set_position(conn, tenant_id, *id, position as i32)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }
                let mut reordered = options;
                reordered.sort_by_key(|o| ids.iter().position(|id| *id == o.id));
                for (position, option) in reordered.iter_mut().enumerate() {
                    option.position = position as i32;
                }
                Ok(reordered)
            }
            .scope_boxed()
        })
        .await
    }
}
