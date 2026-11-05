//! E-invoicing is part of every edition: any entity can turn it on, change the format and turn it off.
use meteroid::module::Modules;
use meteroid_grpc::meteroid::api::invoicingentities::v1::{
    EInvoicingFormat, GetInvoicingEntityRequest, InvoicingEntityData, UpdateInvoicingEntityRequest,
};
use tonic::Code;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;

async fn set_format(clients: &AllClients, id: &str, format: EInvoicingFormat) -> Result<(), Code> {
    clients
        .invoicing_entities
        .clone()
        .update_invoicing_entity(UpdateInvoicingEntityRequest {
            id: id.to_string(),
            data: Some(InvoicingEntityData {
                einvoicing_format: Some(format as i32),
                ..Default::default()
            }),
        })
        .await
        .map(|_| ())
        .map_err(|status| status.code())
}

#[tokio::test]
async fn oss_turns_einvoicing_on() {
    helpers::init::logging();
    let modules = Modules::core();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let instance = modules.edition.instance();
    let setup = meteroid_it::container::start_meteroid_with_modules(
        database,
        SeedLevel::MINIMAL,
        instance,
        modules,
    )
    .await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let id = clients
        .invoicing_entities
        .clone()
        .get_invoicing_entity(GetInvoicingEntityRequest { id: None })
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
        .id;

    assert_eq!(
        set_format(&clients, &id, EInvoicingFormat::FacturX).await,
        Ok(())
    );
    assert_eq!(
        set_format(&clients, &id, EInvoicingFormat::Xrechnung).await,
        Ok(())
    );
    assert_eq!(
        set_format(&clients, &id, EInvoicingFormat::EinvoicingFormatUnspecified).await,
        Ok(())
    );
}
