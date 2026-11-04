pub mod invoicing_entities {
    use common_domain::country::CountryCode;
    use common_domain::ids::{InvoicingEntityId, StoredDocumentId, TaxCategoryId};

    use meteroid_grpc::meteroid::api::invoicingentities::v1 as server;
    use meteroid_grpc::meteroid::api::invoicingentities::v1::{EInvoicingFormat, TaxResolver};
    use meteroid_store::domain::invoicing_entities as domain;
    use meteroid_store::domain::invoicing_entities::{FooterTranslation, FooterTranslations};
    use meteroid_store::services::customer_locale::customer_enabled_locales;

    fn non_blank(s: String) -> Option<String> {
        Some(s).filter(|s| !s.trim().is_empty())
    }

    fn footer_translations_from_proto(proto: server::FooterTranslations) -> FooterTranslations {
        let mut by_locale = std::collections::BTreeMap::new();
        for (locale, t) in proto.by_locale {
            let info = t.info.and_then(non_blank);
            let legal = t.legal.and_then(non_blank);
            // Drop fully-empty entries so "unset" has one representation.
            if info.is_some() || legal.is_some() {
                by_locale.insert(locale, FooterTranslation { info, legal });
            }
        }
        FooterTranslations(by_locale)
    }

    fn footer_translations_to_proto(domain: &FooterTranslations) -> server::FooterTranslations {
        server::FooterTranslations {
            by_locale: domain
                .0
                .iter()
                .map(|(locale, t)| {
                    (
                        locale.clone(),
                        server::FooterTranslation {
                            info: t.info.clone(),
                            legal: t.legal.clone(),
                        },
                    )
                })
                .collect(),
        }
    }

    /// Reject a language we ship no bundle for at the point of writing it, rather than
    /// storing it and rendering documents in English with only a log line to show for it.
    fn reject_unsupported_language(
        language: Option<String>,
    ) -> Result<Option<String>, tonic::Status> {
        meteroid_invoicing::lang::reject_unsupported(language.as_deref()).map_err(|tag| {
            tonic::Status::invalid_argument(format!(
                "Unsupported invoicing_language '{tag}'; supported: {:?}",
                meteroid_invoicing::lang::SUPPORTED
            ))
        })?;
        Ok(language)
    }

    pub fn proto_to_domain(
        proto: server::InvoicingEntityData,
    ) -> Result<domain::InvoicingEntityNew, tonic::Status> {
        Ok(domain::InvoicingEntityNew {
            legal_name: proto.legal_name,
            invoice_number_pattern: proto.invoice_number_pattern,
            credit_note_number_pattern: proto.credit_note_number_pattern,
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
            portal_theme_mode: proto.portal_theme_mode,
            portal_roundness: proto.portal_roundness,
            require_vies_valid_for_reverse_charge: proto.require_vies_valid_for_reverse_charge,
            invoicing_language: reject_unsupported_language(proto.invoicing_language)?,
            invoice_footer_translations: proto
                .invoice_footer_translations
                .map(footer_translations_from_proto),
            customer_locale: proto.customer_locale,
            customer_extra_locales: proto.customer_extra_locales.map(|l| l.values),
            brand_color_light: proto.brand_color_light,
            portal_credit_top_up_enabled: proto.portal_top_up_enabled,
            portal_auto_top_up_enabled: proto.portal_auto_top_up_enabled,
            portal_budgets_enabled: proto.portal_budgets_enabled,
            require_billing_information: proto.require_billing_information,
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
            meteroid_store::domain::enums::TaxResolverEnum::External => TaxResolver::External,
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
                TaxResolver::External => meteroid_store::domain::enums::TaxResolverEnum::External,
                TaxResolver::None => meteroid_store::domain::enums::TaxResolverEnum::None,
            })
        })
    }

    pub fn proto_to_patch_domain(
        proto: server::InvoicingEntityData,
        id: InvoicingEntityId,
    ) -> Result<domain::InvoicingEntityPatch, tonic::Status> {
        reject_unsupported_language(proto.invoicing_language.clone())?;
        Ok(domain::InvoicingEntityPatch {
            id,
            legal_name: proto.legal_name,
            invoice_number_pattern: proto.invoice_number_pattern,
            credit_note_number_pattern: proto.credit_note_number_pattern,
            // next_invoice_number: proto.next_invoice_number,
            // next_credit_note_number: proto.next_credit_note_number,
            grace_period_hours: proto.grace_period_hours,
            net_terms: proto.net_terms,
            invoice_footer_info: proto.invoice_footer_info,
            invoice_footer_legal: proto.invoice_footer_legal,
            logo_attachment_id: None, // managed via a different api
            // Absent (None) means "leave unchanged" so the Merchant and Branding
            // tabs can each PATCH their own subset of fields without clobbering
            // the other's. A cleared value is sent as an empty string, not absence.
            brand_color: proto.brand_color.map(Some),
            address_line1: proto.address_line1,
            address_line2: proto.address_line2,
            zip_code: proto.zip_code,
            state: proto.state,
            city: proto.city,
            vat_number: proto.vat_number,
            country: None, // country is not editable, TODO remove from proto
            tax_resolver: tax_resolver_server_to_domain(proto.tax_resolver),
            // Set through InvoiceConsolidationService.
            consolidate_recurring_invoices: None,
            portal_theme_mode: proto.portal_theme_mode.map(Some),
            portal_roundness: proto.portal_roundness.map(Some),
            require_vies_valid_for_reverse_charge: proto.require_vies_valid_for_reverse_charge,
            invoicing_language: proto.invoicing_language.map(Some),
            invoice_footer_translations: proto
                .invoice_footer_translations
                .map(footer_translations_from_proto),
            default_tax_category_id: match proto.default_tax_category_id {
                None => None,
                Some(s) if s.is_empty() => Some(None),
                Some(s) => Some(Some(TaxCategoryId::from_proto(s)?)),
            },
            einvoicing_format: proto
                .einvoicing_format
                .map(einvoicing_format_server_to_domain),
            legal_number: proto.legal_number.map(Some),
            // Empty string clears it (follow the invoice language again).
            customer_locale: proto.customer_locale.map(Some),
            customer_extra_locales: proto.customer_extra_locales.map(|l| l.values),
            // Empty string clears it (emails derive it from the brand colour again).
            brand_color_light: proto.brand_color_light.map(Some),
            portal_credit_top_up_enabled: proto.portal_top_up_enabled,
            portal_auto_top_up_enabled: proto.portal_auto_top_up_enabled,
            portal_budgets_enabled: proto.portal_budgets_enabled,
            // Empty string clears them.
            contact_name: proto.contact_name.map(Some),
            contact_phone: proto.contact_phone.map(Some),
            contact_email: proto.contact_email.map(Some),
            require_billing_information: proto.require_billing_information,
        })
    }

    fn einvoicing_format_server_to_domain(value: i32) -> Option<domain::EInvoicingFormat> {
        match EInvoicingFormat::try_from(value).ok()? {
            EInvoicingFormat::EinvoicingFormatUnspecified => None,
            EInvoicingFormat::FacturX => Some(domain::EInvoicingFormat::FacturX),
            EInvoicingFormat::Xrechnung => Some(domain::EInvoicingFormat::XRechnung),
            EInvoicingFormat::PeppolBis => Some(domain::EInvoicingFormat::PeppolBis),
        }
    }

    fn einvoicing_format_domain_to_server(
        value: Option<domain::EInvoicingFormat>,
    ) -> EInvoicingFormat {
        match value {
            None => EInvoicingFormat::EinvoicingFormatUnspecified,
            Some(domain::EInvoicingFormat::FacturX) => EInvoicingFormat::FacturX,
            Some(domain::EInvoicingFormat::XRechnung) => EInvoicingFormat::Xrechnung,
            Some(domain::EInvoicingFormat::PeppolBis) => EInvoicingFormat::PeppolBis,
        }
    }

    pub fn domain_to_proto(domain: domain::InvoicingEntity) -> server::InvoicingEntity {
        // Two locales the UI keeps apart: what documents render in, and the language
        // the base footer is written in (the seller's home language, which an explicit
        // `invoicing_language` does not change). Borrowed before the struct moves.
        let template_lang_for = |explicit: Option<&str>| {
            let resolved = meteroid_store::constants::Countries::resolve_document_language(
                None,
                explicit,
                &domain.country.code,
            );
            meteroid_invoicing::lang::template_lang(&resolved).to_string()
        };
        let effective_document_language = template_lang_for(domain.invoicing_language.as_deref());
        let country_document_language = template_lang_for(None);
        // Normalized customer locales: the resolved default first, then the effective extras.
        let customer_locales = customer_enabled_locales(&domain);
        server::InvoicingEntity {
            id: domain.id.as_proto(),
            local_id: domain.id.as_proto(), // todo remove me
            is_default: domain.is_default,
            legal_name: domain.legal_name,
            invoice_number_pattern: domain.invoice_number_pattern,
            credit_note_number_pattern: domain.credit_note_number_pattern,
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
            portal_theme_mode: domain.portal_theme_mode,
            portal_roundness: domain.portal_roundness,
            require_vies_valid_for_reverse_charge: domain.require_vies_valid_for_reverse_charge,
            invoicing_language: domain.invoicing_language,
            invoice_footer_translations: Some(footer_translations_to_proto(
                &domain.invoice_footer_translations,
            )),
            effective_document_language,
            country_document_language,
            default_tax_category_id: domain.default_tax_category_id.map(|id| id.as_proto()),
            einvoicing_format: einvoicing_format_domain_to_server(domain.einvoicing_format).into(),
            legal_number: domain.legal_number,
            customer_locale: domain.customer_locale,
            customer_extra_locales: customer_locales[1..]
                .iter()
                .map(|l| l.to_string())
                .collect(),
            effective_customer_locale: customer_locales[0].to_string(),
            brand_color_light: domain.brand_color_light,
            portal_top_up_enabled: domain.portal_credit_top_up_enabled,
            portal_auto_top_up_enabled: domain.portal_auto_top_up_enabled,
            portal_budgets_enabled: domain.portal_budgets_enabled,
            contact_name: domain.contact_name,
            contact_phone: domain.contact_phone,
            contact_email: domain.contact_email,
            require_billing_information: domain.require_billing_information,
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
            legal_number: domain.legal_number,
        }
    }
}
