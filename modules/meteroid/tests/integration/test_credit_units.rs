//! S3: the tenant's credit unit catalog (gRPC CRUD) and what archiving does and does not block.
use crate::data::ids::TENANT_ID;
use crate::meteroid_it::container::SeedLevel;
use crate::{helpers, meteroid_it};
use common_domain::ids::{CreditUnitId, TenantId};
use meteroid_grpc::meteroid::api::creditunits::v1 as grpc;
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::credit::denomination::resolve_denomination;
use tonic::Code;

#[tokio::test]
async fn test_credit_units_crud_validates_and_archives() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    let mut credit_units = clients.credit_units.clone();

    // --- create ---
    let created = credit_units
        .create_credit_unit(grpc::CreateCreditUnitRequest {
            code: "credits".into(), // lower-case is accepted and normalized to upper-case
            name: "Credit".into(),
            plural_name: "Credits".into(),
            symbol: Some("cr".into()),
            precision: 2,
        })
        .await
        .expect("create_credit_unit")
        .into_inner()
        .unit
        .expect("unit");

    assert_eq!(created.code, "CREDITS");
    assert_eq!(created.name, "Credit");
    assert_eq!(created.plural_name, "Credits");
    assert_eq!(created.symbol, Some("cr".to_string()));
    assert_eq!(created.precision, 2);
    assert!(!created.archived);

    // --- list / get ---
    let listed = credit_units
        .list_credit_units(grpc::ListCreditUnitsRequest {
            include_archived: false,
        })
        .await
        .expect("list_credit_units")
        .into_inner()
        .units;
    assert!(listed.iter().any(|u| u.id == created.id));

    let fetched = credit_units
        .get_credit_unit(grpc::GetCreditUnitRequest {
            id: created.id.clone(),
        })
        .await
        .expect("get_credit_unit")
        .into_inner()
        .unit
        .expect("unit");
    assert_eq!(fetched.id, created.id);

    // --- validation: too short is structurally an ISO-currency-length clash, refused ---
    let short_code_error = credit_units
        .create_credit_unit(grpc::CreateCreditUnitRequest {
            code: "USD".into(),
            name: "Dollar credit".into(),
            plural_name: "Dollar credits".into(),
            symbol: None,
            precision: 2,
        })
        .await
        .expect_err("a 3-letter code must be refused");
    assert_eq!(short_code_error.code(), Code::InvalidArgument);

    // --- validation: duplicate code within the tenant ---
    let duplicate_error = credit_units
        .create_credit_unit(grpc::CreateCreditUnitRequest {
            code: "CREDITS".into(),
            name: "Other".into(),
            plural_name: "Others".into(),
            symbol: None,
            precision: 0,
        })
        .await
        .expect_err("a duplicate code must be refused");
    assert_eq!(duplicate_error.code(), Code::AlreadyExists);

    // --- update: name/plural/symbol, code and precision have no field ---
    let updated = credit_units
        .update_credit_unit(grpc::UpdateCreditUnitRequest {
            id: created.id.clone(),
            name: Some("Token".into()),
            plural_name: Some("Tokens".into()),
            symbol: Some(String::new()), // clears it
        })
        .await
        .expect("update_credit_unit")
        .into_inner()
        .unit
        .expect("unit");
    assert_eq!(updated.name, "Token");
    assert_eq!(updated.plural_name, "Tokens");
    assert_eq!(updated.symbol, None);
    assert_eq!(updated.code, "CREDITS", "code is immutable");
    assert_eq!(updated.precision, 2, "precision is immutable");

    // --- archive: blocks new pricing, existing resolution keeps working ---
    let archived = credit_units
        .archive_credit_unit(grpc::ArchiveCreditUnitRequest {
            id: created.id.clone(),
        })
        .await
        .expect("archive_credit_unit")
        .into_inner()
        .unit
        .expect("unit");
    assert!(archived.archived);

    let still_listed_with_archived = credit_units
        .list_credit_units(grpc::ListCreditUnitsRequest {
            include_archived: true,
        })
        .await
        .expect("list_credit_units")
        .into_inner()
        .units;
    assert!(
        still_listed_with_archived
            .iter()
            .any(|u| u.id == created.id && u.archived)
    );

    let hidden_by_default = credit_units
        .list_credit_units(grpc::ListCreditUnitsRequest {
            include_archived: false,
        })
        .await
        .expect("list_credit_units")
        .into_inner()
        .units;
    assert!(
        !hidden_by_default.iter().any(|u| u.id == created.id),
        "archived units are hidden by default"
    );

    assert_archive_blocks_new_pricing_but_not_existing_resolution(
        &setup.store,
        TENANT_ID,
        created.id,
    )
    .await;
}

/// `ensure_credit_unit_active` is the primitive S4/S5 call before accepting a `credit_unit_id`
/// on a new price, pack or grant; `resolve_denomination` is what every *existing* reader
/// (wallets, prices, projection) already calls, and must keep resolving the unit archived or not.
async fn assert_archive_blocks_new_pricing_but_not_existing_resolution(
    store: &meteroid_store::Store,
    tenant_id: TenantId,
    archived_unit_id: String,
) {
    let unit_id = CreditUnitId::from_proto(archived_unit_id).expect("valid credit unit id");

    let refused = store
        .ensure_credit_unit_active(tenant_id, unit_id)
        .await
        .expect_err("an archived unit must refuse new pricing");
    assert!(matches!(
        refused.current_context(),
        meteroid_store::errors::StoreError::FailedPrecondition(_)
    ));

    let mut conn = store.get_conn().await.expect("conn");
    let resolved = resolve_denomination(&mut conn, tenant_id, "CREDITS")
        .await
        .expect("an archived unit must still resolve for existing wallets/prices");
    assert_eq!(resolved.credit_unit_id(), Some(unit_id));
}
