pub mod invoicing_entities {
    use common_domain::ids::InvoicingEntityId;
    use meteroid_grpc::meteroid::api::invoicingentities::v1 as server;
    use meteroid_store::domain::invoicing_entities as domain;

    pub fn proto_to_domain(proto: server::InvoicingEntityData) -> domain::InvoicingEntityNew {
        domain::InvoicingEntityNew {
            legal_name: proto.legal_name,
            invoice_number_pattern: proto.invoice_number_pattern,
            next_invoice_number: None,
            next_credit_note_number: None,
            grace_period_hours: proto.grace_period_hours,
            net_terms: proto.net_terms,
            invoice_footer_info: proto.invoice_footer_info,
            invoice_footer_legal: proto.invoice_footer_legal,
            logo_attachment_id: proto.logo_attachment_id,
            brand_color: proto.brand_color,
            address_line1: proto.address_line1,
            address_line2: proto.address_line2,
            zip_code: proto.zip_code,
            state: proto.state,
            city: proto.city,
            vat_number: proto.vat_number,
            country: proto.country,
        }
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
            country: proto.country,
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
            logo_attachment_id: domain.logo_attachment_id,
            brand_color: domain.brand_color,
            address_line1: domain.address_line1,
            address_line2: domain.address_line2,
            zip_code: domain.zip_code,
            state: domain.state,
            city: domain.city,
            vat_number: domain.vat_number,
            country: domain.country,
            accounting_currency: domain.accounting_currency,
        }
    }
}
