pub mod invoicing_entities {
    use common_domain::country::CountryCode;
    use common_domain::ids::{InvoicingEntityId, StoredDocumentId};

    use meteroid_grpc::meteroid::api::invoicingentities::v1 as server;
    use meteroid_grpc::meteroid::api::invoicingentities::v1::TaxResolver;
    use meteroid_store::domain::invoicing_entities as domain;

    pub fn proto_to_domain(
        proto: server::InvoicingEntityData,
    ) -> Result<domain::InvoicingEntityNew, tonic::Status> {
        Ok(domain::InvoicingEntityNew {
            legal_name: proto.legal_name,
            invoice_number_pattern: proto.invoice_number_pattern,
            next_invoice_number: None,
            next_credit_note_number: None,
            grace_period_hours: proto.grace_period_hours,
            net_terms: proto.net_terms,
            invoice_footer_info: proto.invoice_footer_info,
            invoice_footer_legal: proto.invoice_footer_legal,
            logo_attachment_id: StoredDocumentId::from_proto_opt(proto.logo_attachment_id)?,
            brand_color: proto.brand_color,
            address_line1: proto.address_line1,
            address_line2: proto.address_line2,
            zip_code: proto.zip_code,
            state: proto.state,
            city: proto.city,
            vat_number: proto.vat_number,
            country: CountryCode::from_proto_opt(proto.country)?,
            tax_resolver: tax_resolver_server_to_domain(proto.tax_resolver)
                .unwrap_or(meteroid_store::domain::enums::TaxResolverEnum::None),
        })
    }

    fn tax_resolver_domain_to_server(
        value: meteroid_store::domain::enums::TaxResolverEnum,
    ) -> TaxResolver {
        match value {
            meteroid_store::domain::enums::TaxResolverEnum::Manual => TaxResolver::Manual,
            meteroid_store::domain::enums::TaxResolverEnum::MeteroidEuVat => {
                TaxResolver::MeteroidEuVat
            }
            meteroid_store::domain::enums::TaxResolverEnum::None => TaxResolver::None,
        }
    }

    pub fn tax_resolver_server_to_domain(
        tr: Option<i32>,
    ) -> Option<meteroid_store::domain::enums::TaxResolverEnum> {
        tr.and_then(|tr_int| {
            TaxResolver::try_from(tr_int).ok().map(|tr| match tr {
                TaxResolver::Manual => meteroid_store::domain::enums::TaxResolverEnum::Manual,
                TaxResolver::MeteroidEuVat => {
                    meteroid_store::domain::enums::TaxResolverEnum::MeteroidEuVat
                }
                TaxResolver::None => meteroid_store::domain::enums::TaxResolverEnum::None,
            })
        })
    }

    pub fn proto_to_patch_domain(
        proto: server::InvoicingEntityData,
        id: InvoicingEntityId,
    ) -> domain::InvoicingEntityPatch {
        domain::InvoicingEntityPatch {
            id,
            legal_name: proto.legal_name,
            invoice_number_pattern: proto.invoice_number_pattern,
            // next_invoice_number: proto.next_invoice_number,
            // next_credit_note_number: proto.next_credit_note_number,
            grace_period_hours: proto.grace_period_hours,
            net_terms: proto.net_terms,
            invoice_footer_info: proto.invoice_footer_info,
            invoice_footer_legal: proto.invoice_footer_legal,
            logo_attachment_id: None, // managed via a different api
            brand_color: Some(proto.brand_color),
            address_line1: proto.address_line1,
            address_line2: proto.address_line2,
            zip_code: proto.zip_code,
            state: proto.state,
            city: proto.city,
            vat_number: proto.vat_number,
            country: None, // country is not editable, TODO remove from proto
            tax_resolver: tax_resolver_server_to_domain(proto.tax_resolver),
        }
    }

    pub fn domain_to_proto(domain: domain::InvoicingEntity) -> server::InvoicingEntity {
        server::InvoicingEntity {
            id: domain.id.as_proto(),
            local_id: domain.id.as_proto(), // todo remove me
            is_default: domain.is_default,
            legal_name: domain.legal_name,
            invoice_number_pattern: domain.invoice_number_pattern,
            next_invoice_number: domain.next_invoice_number,
            next_credit_note_number: domain.next_credit_note_number,
            grace_period_hours: domain.grace_period_hours,
            net_terms: domain.net_terms,
            invoice_footer_info: domain.invoice_footer_info,
            invoice_footer_legal: domain.invoice_footer_legal,
            logo_attachment_id: domain.logo_attachment_id.map(|id| id.as_proto()),
            brand_color: domain.brand_color,
            address_line1: domain.address_line1,
            address_line2: domain.address_line2,
            zip_code: domain.zip_code,
            state: domain.state,
            city: domain.city,
            vat_number: domain.vat_number,
            country: domain.country.as_proto(),
            accounting_currency: domain.accounting_currency,
            tax_resolver: tax_resolver_domain_to_server(domain.tax_resolver).into(),
        }
    }

    pub fn domain_to_public_proto(
        domain: domain::InvoicingEntity,
    ) -> server::InvoicingEntityPublic {
        server::InvoicingEntityPublic {
            legal_name: domain.legal_name,
            net_terms: domain.net_terms,
            invoice_footer_info: domain.invoice_footer_info,
            invoice_footer_legal: domain.invoice_footer_legal,
            logo_attachment_id: domain.logo_attachment_id.map(|id| id.as_proto()),
            brand_color: domain.brand_color,
            address_line1: domain.address_line1,
            address_line2: domain.address_line2,
            zip_code: domain.zip_code,
            state: domain.state,
            city: domain.city,
            vat_number: domain.vat_number,
            country: domain.country.as_proto(),
        }
    }
}
