use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, EntityActivityId, InvoiceId, InvoicingEntityId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::customeremails::v1::{
    ClearEmailKindAudienceRequest, ClearEmailKindAudienceResponse, EmailVariable,
    GetEmailSettingsRequest, GetEmailSettingsResponse, GetSentEmailRequest, GetSentEmailResponse,
    ListEmailVariablesRequest, ListEmailVariablesResponse, ListSentEmailsRequest,
    ListSentEmailsResponse, ListSkippedEmailsRequest, ListSkippedEmailsResponse,
    PreviewEmailKindAudienceRequest, PreviewEmailKindAudienceResponse, PreviewEmailRequest,
    PreviewEmailResponse, PreviewInvoiceEmailRequest, PreviewInvoiceEmailResponse, RenderedEmail,
    ResendEmailRequest, ResendEmailResponse, ResendUnknownEmailRequest, ResendUnknownEmailResponse,
    SendTestEmailRequest, SendTestEmailResponse, SentEmail as ProtoSentEmail,
    SetEmailKindAudienceRequest, SetEmailKindAudienceResponse, SetEmailKindEnabledRequest,
    SetEmailKindEnabledResponse, UpdateEmailSettingsRequest, UpdateEmailSettingsResponse,
    customer_emails_service_server::CustomerEmailsService,
};
use meteroid_mailer::copy::{self, VariableKind};
use meteroid_mailer::model::EmailRecipient;
use meteroid_mailer::render::render;
use meteroid_store::domain::InvoicingEntity;
use meteroid_store::domain::customer_emails::{
    AudienceSubject, EmailContext, EmailKind, EmailSettings,
};
use meteroid_store::domain::entity_activity::EntityType;
use meteroid_store::domain::payment_reminders::EmailCopy;
use meteroid_store::repositories::customer_emails::{
    SENT_EMAILS_DEFAULT_LIMIT, SentEmailFilter, SkippedEmailFilter,
};
use meteroid_store::repositories::entity_activity::EntityActivityInterfaceResolveEntities;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::{CustomerEmailsInterface, UserInterface};
use meteroid_store::services::customer_emails::{BuildOutcome, EmailRenderInput, tenant_test_mode};
use meteroid_store::services::customer_locale::customer_default_locale;
use meteroid_store::services::payment_reminders::audience::parse_audience;
use std::collections::HashMap;
use std::str::FromStr;
use tonic::{Request, Response, Status};

use super::CustomerEmailsServiceComponents;
use super::error::CustomerEmailsApiError;
use super::mapping;
use meteroid_app::conversions::ProtoConv;
use meteroid_app::sharable::generate_entity_share_key;

/// Kinds that render from an invoice; the others need a credit note or a quote.
fn is_invoice_kind(kind: EmailKind) -> bool {
    matches!(
        kind,
        EmailKind::InvoiceReady
            | EmailKind::InvoicePaid
            | EmailKind::PaymentFailed
            | EmailKind::AuthenticationRequired
            | EmailKind::PaymentReminder
    )
}

fn locale_from_request(
    entity: &InvoicingEntity,
    requested: Option<&str>,
) -> Result<&'static str, Status> {
    match requested.map(str::trim).filter(|l| !l.is_empty()) {
        None => Ok(customer_default_locale(entity)),
        Some(l) if common_domain::lang::is_supported(l) => {
            Ok(common_domain::lang::template_lang(l))
        }
        Some(l) => Err(Status::invalid_argument(format!(
            "unsupported locale {l:?}; supported: {:?}",
            common_domain::lang::SUPPORTED
        ))),
    }
}

type EntityNames = HashMap<(EntityType, uuid::Uuid), String>;

/// A listed row's entity label and customer name.
fn row_labels(
    names: &EntityNames,
    entity_type: &str,
    entity_id: uuid::Uuid,
    customer_id: Option<CustomerId>,
) -> (Option<String>, Option<String>) {
    let entity_label = EntityType::from_str(entity_type)
        .ok()
        .and_then(|t| names.get(&(t, entity_id)).cloned());
    let customer_name =
        customer_id.and_then(|c| names.get(&(EntityType::Customer, c.as_uuid())).cloned());
    (entity_label, customer_name)
}

fn unknown_variables(draft: Option<&EmailCopy>) -> Vec<String> {
    let Some(draft) = draft else {
        return vec![];
    };
    let mut unknown = copy::unknown_variables(&draft.subject);
    for name in copy::unknown_variables(&draft.body) {
        if !unknown.contains(&name) {
            unknown.push(name);
        }
    }
    unknown
}

impl CustomerEmailsServiceComponents {
    async fn entity(
        &self,
        tenant_id: common_domain::ids::TenantId,
        invoicing_entity_id: Option<InvoicingEntityId>,
    ) -> Result<InvoicingEntity, Status> {
        self.store
            .get_invoicing_entity(tenant_id, invoicing_entity_id)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)
            .map_err(Into::into)
    }

    async fn settings_response(
        &self,
        tenant_id: common_domain::ids::TenantId,
        entity: &InvoicingEntity,
        settings: &EmailSettings,
    ) -> Result<meteroid_grpc::meteroid::api::customeremails::v1::EmailSettings, Status> {
        let audiences = self
            .store
            .list_email_kind_audiences(tenant_id, entity.id)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        mapping::settings_to_proto(
            settings,
            &audiences,
            settings.sender(entity, self.store.mailer.customer_from_addr()),
            entity,
        )
    }

    async fn settings_for(
        &self,
        tenant_id: common_domain::ids::TenantId,
        entity: &InvoicingEntity,
    ) -> Result<meteroid_grpc::meteroid::api::customeremails::v1::EmailSettings, Status> {
        let settings = self
            .store
            .get_email_settings(tenant_id, entity.id)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        self.settings_response(tenant_id, entity, &settings).await
    }

    fn rendered_response(
        &self,
        input: &EmailRenderInput,
        unknown_variables: Vec<String>,
    ) -> Result<RenderedEmail, Status> {
        let rendered = render(&input.layout, input.subject.clone(), input.body.clone())
            .map_err(|e| Status::internal(format!("failed to render email: {e}")))?;
        let sender = input
            .sender(self.store.mailer.customer_from_addr())
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        Ok(RenderedEmail {
            subject: rendered.subject,
            body_html: rendered.body_html,
            from: sender.from,
            reply_to: sender.reply_to,
            bcc: sender.bcc,
            attachment_names: input
                .attachments
                .iter()
                .map(|a| a.filename.clone())
                .collect(),
            unknown_variables,
            locale: input.lang.to_string(),
        })
    }

    /// Display names of the listed rows' entities and customers, in one lookup.
    async fn row_names(
        &self,
        tenant_id: common_domain::ids::TenantId,
        rows: impl Iterator<Item = (&str, uuid::Uuid, Option<CustomerId>)>,
    ) -> Result<EntityNames, Status> {
        let mut refs: Vec<(EntityType, uuid::Uuid)> = Vec::new();
        for (entity_type, entity_id, customer_id) in rows {
            if let Ok(t) = EntityType::from_str(entity_type)
                && t.resolves_display_name()
            {
                refs.push((t, entity_id));
            }
            if let Some(c) = customer_id {
                refs.push((EntityType::Customer, c.as_uuid()));
            }
        }
        self.store
            .resolve_entity_names(tenant_id, &refs)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)
            .map_err(Into::into)
    }

    /// Synthetic preview shared by `PreviewEmail` and `SendTestEmail`. Only the test send carries
    /// the non-production banner; on-screen previews leave it out (the UI notes it instead).
    async fn preview_input(
        &self,
        tenant_id: common_domain::ids::TenantId,
        invoicing_entity_id: &str,
        kind: i32,
        locale: Option<&str>,
        draft: Option<&EmailCopy>,
        offset_days: Option<i32>,
        with_test_banner: bool,
    ) -> Result<EmailRenderInput, Status> {
        let entity_id = mapping::entity_id_from_proto(invoicing_entity_id)?;
        let kind = mapping::kind_from_proto(kind)?;
        let entity = self.entity(tenant_id, Some(entity_id)).await?;
        let settings = self
            .store
            .get_email_settings(tenant_id, entity.id)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        let lang = locale_from_request(&entity, locale)?;
        let draft = draft.filter(|_| kind.editable_copy());
        let test_mode = with_test_banner
            && tenant_test_mode(&self.store, tenant_id)
                .await
                .map_err(Into::<CustomerEmailsApiError>::into)?;
        self.services
            .build_preview(
                &entity,
                &settings,
                kind,
                lang,
                draft,
                offset_days,
                false,
                test_mode,
            )
            .map_err(Into::<CustomerEmailsApiError>::into)
            .map_err(Into::into)
    }
}

#[tonic::async_trait]
impl CustomerEmailsService for CustomerEmailsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_email_settings(
        &self,
        request: Request<GetEmailSettingsRequest>,
    ) -> Result<Response<GetEmailSettingsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let entity_id = InvoicingEntityId::from_proto_opt(req.invoicing_entity_id)?;
        let entity = self.entity(tenant_id, entity_id).await?;
        Ok(Response::new(GetEmailSettingsResponse {
            settings: Some(self.settings_for(tenant_id, &entity).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_email_settings(
        &self,
        request: Request<UpdateEmailSettingsRequest>,
    ) -> Result<Response<UpdateEmailSettingsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let entity_id = mapping::entity_id_from_proto(&req.invoicing_entity_id)?;
        let patch = mapping::patch_from_proto(req.data.unwrap_or_default())?;
        let entity = self.entity(tenant_id, Some(entity_id)).await?;
        let settings = self
            .store
            .update_email_settings(actor, tenant_id, entity.id, patch)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        Ok(Response::new(UpdateEmailSettingsResponse {
            settings: Some(
                self.settings_response(tenant_id, &entity, &settings)
                    .await?,
            ),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_email_kind_enabled(
        &self,
        request: Request<SetEmailKindEnabledRequest>,
    ) -> Result<Response<SetEmailKindEnabledResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let entity_id = mapping::entity_id_from_proto(&req.invoicing_entity_id)?;
        let kind = mapping::kind_from_proto(req.kind)?;
        let entity = self.entity(tenant_id, Some(entity_id)).await?;
        let settings = self
            .store
            .set_email_kind_enabled(actor, tenant_id, entity.id, kind, req.enabled)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        Ok(Response::new(SetEmailKindEnabledResponse {
            settings: Some(
                self.settings_response(tenant_id, &entity, &settings)
                    .await?,
            ),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_email_kind_audience(
        &self,
        request: Request<SetEmailKindAudienceRequest>,
    ) -> Result<Response<SetEmailKindAudienceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let entity_id = mapping::entity_id_from_proto(&req.invoicing_entity_id)?;
        let kind = mapping::kind_from_proto(req.kind)?;
        let mode = mapping::mode_from_proto(req.mode)?;
        let audience =
            parse_audience(&req.audience).map_err(Into::<CustomerEmailsApiError>::into)?;
        let entity = self.entity(tenant_id, Some(entity_id)).await?;
        self.store
            .set_email_kind_audience(actor, tenant_id, entity.id, kind, mode, audience)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        Ok(Response::new(SetEmailKindAudienceResponse {
            settings: Some(self.settings_for(tenant_id, &entity).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn clear_email_kind_audience(
        &self,
        request: Request<ClearEmailKindAudienceRequest>,
    ) -> Result<Response<ClearEmailKindAudienceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let entity_id = mapping::entity_id_from_proto(&req.invoicing_entity_id)?;
        let kind = mapping::kind_from_proto(req.kind)?;
        let entity = self.entity(tenant_id, Some(entity_id)).await?;
        self.store
            .clear_email_kind_audience(actor, tenant_id, entity.id, kind)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        Ok(Response::new(ClearEmailKindAudienceResponse {
            settings: Some(self.settings_for(tenant_id, &entity).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_email_kind_audience(
        &self,
        request: Request<PreviewEmailKindAudienceRequest>,
    ) -> Result<Response<PreviewEmailKindAudienceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let entity_id = mapping::entity_id_from_proto(&req.invoicing_entity_id)?;
        let kind = mapping::kind_from_proto(req.kind)?;
        let mode = mapping::mode_from_proto(req.mode)?;
        let audience =
            parse_audience(&req.audience).map_err(Into::<CustomerEmailsApiError>::into)?;
        let preview = self
            .store
            .preview_email_kind_audience(tenant_id, entity_id, kind, mode, &audience)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        Ok(Response::new(PreviewEmailKindAudienceResponse {
            preview: Some(mapping::audience_preview_to_proto(preview)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_email_variables(
        &self,
        request: Request<ListEmailVariablesRequest>,
    ) -> Result<Response<ListEmailVariablesResponse>, Status> {
        request.tenant()?;
        Ok(Response::new(ListEmailVariablesResponse {
            variables: copy::VARIABLES
                .iter()
                .map(|v| EmailVariable {
                    name: v.name.to_string(),
                    kind: match v.kind {
                        VariableKind::Text => "text",
                        VariableKind::Date => "date",
                        VariableKind::Money => "money",
                        VariableKind::Int => "int",
                    }
                    .to_string(),
                    example: v.example.to_string(),
                })
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_email(
        &self,
        request: Request<PreviewEmailRequest>,
    ) -> Result<Response<PreviewEmailResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let draft = mapping::copy_from_proto(req.draft);
        let input = self
            .preview_input(
                tenant_id,
                &req.invoicing_entity_id,
                req.kind,
                req.locale.as_deref(),
                draft.as_ref(),
                req.offset_days,
                false,
            )
            .await?;
        let email = self.rendered_response(&input, unknown_variables(draft.as_ref()))?;
        Ok(Response::new(PreviewEmailResponse { email: Some(email) }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn preview_invoice_email(
        &self,
        request: Request<PreviewInvoiceEmailRequest>,
    ) -> Result<Response<PreviewInvoiceEmailResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let invoice_id = InvoiceId::from_proto(&req.invoice_id)?;
        let kind = mapping::kind_from_proto(req.kind)?;
        if !is_invoice_kind(kind) {
            return Err(Status::invalid_argument(format!(
                "{kind} is not rendered from an invoice"
            )));
        }
        if let Some(l) = req.locale.as_deref().filter(|l| !l.trim().is_empty())
            && !common_domain::lang::is_supported(l)
        {
            return Err(Status::invalid_argument(format!(
                "unsupported locale {l:?}"
            )));
        }
        let context = match kind {
            EmailKind::PaymentReminder => self
                .services
                .preview_reminder_context(tenant_id, invoice_id, req.offset_days)
                .await
                .map_err(Into::<CustomerEmailsApiError>::into)?,
            EmailKind::PaymentFailed | EmailKind::AuthenticationRequired => {
                use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
                let last = self
                    .store
                    .list_payment_tx_by_invoice_id(tenant_id, invoice_id)
                    .await
                    .map_err(Into::<CustomerEmailsApiError>::into)?
                    .into_iter()
                    .filter(|t| t.transaction.payment_method_id.is_some())
                    .max_by_key(|t| t.transaction.processed_at)
                    .ok_or_else(|| {
                        Status::failed_precondition(
                            "The invoice has no payment attempt to render this email from",
                        )
                    })?;
                let payment_transaction_id = last.transaction.id;
                if kind == EmailKind::PaymentFailed {
                    EmailContext::PaymentFailed {
                        invoice_id,
                        payment_transaction_id,
                    }
                } else {
                    EmailContext::AuthenticationRequired {
                        invoice_id,
                        payment_transaction_id,
                    }
                }
            }
            _ => EmailContext::Invoice { invoice_id },
        };
        // A placeholder link like `build_preview`: a preview never mints a real portal token.
        let public_url = self
            .store
            .settings
            .public_url
            .trim_end_matches('/')
            .to_string();
        let pay_link = move |_: InvoiceId| -> meteroid_store::StoreResult<String> {
            Ok(format!("{public_url}/portal/invoice-payment?token=preview"))
        };
        let outcome = self
            .services
            .preview_customer_email(
                tenant_id,
                kind,
                &context,
                req.locale.as_deref(),
                None,
                &pay_link,
                false,
            )
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        let mut input = match outcome {
            BuildOutcome::Render(input) => input,
            BuildOutcome::Skip(reason, _) => {
                return Err(Status::failed_precondition(format!(
                    "This email would not be sent: {reason}"
                )));
            }
            BuildOutcome::Defer(reason, _) => {
                return Err(Status::failed_precondition(format!(
                    "This email cannot be rendered yet: {reason:?}"
                )));
            }
        };
        // On-screen preview: no non-production banner (the UI notes it instead).
        input.layout.test_mode = false;
        let email = self.rendered_response(&input, vec![])?;
        let suppressed_by_audience = self
            .store
            .email_audience_suppresses(
                tenant_id,
                input.entity.id,
                kind,
                AudienceSubject::Invoice(invoice_id),
            )
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        Ok(Response::new(PreviewInvoiceEmailResponse {
            email: Some(email),
            suppressed_by_audience,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn send_test_email(
        &self,
        request: Request<SendTestEmailRequest>,
    ) -> Result<Response<SendTestEmailResponse>, Status> {
        let tenant_id = request.tenant()?;
        let Actor::User { id: user_id } = request.actor_typed()? else {
            return Err(Status::failed_precondition(
                "Test emails need a user session",
            ));
        };
        let req = request.into_inner();
        let user = self
            .store
            .find_user_by_id_and_tenant(user_id, tenant_id)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        let draft = mapping::copy_from_proto(req.draft);
        let input = self
            .preview_input(
                tenant_id,
                &req.invoicing_entity_id,
                req.kind,
                req.locale.as_deref(),
                draft.as_ref(),
                req.offset_days,
                true,
            )
            .await?;
        let rendered = render(&input.layout, input.subject.clone(), input.body.clone())
            .map_err(|e| Status::internal(format!("failed to render email: {e}")))?;
        let mut sender = input
            .sender(self.store.mailer.customer_from_addr())
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        sender.bcc.clear();
        let mut email = sender.email(
            vec![EmailRecipient {
                email: user.email.clone(),
                first_name: None,
                last_name: None,
            }],
            rendered,
            vec![],
        );
        email.subject = format!("[Test] {}", email.subject);
        // Without SMTP the mailer is a stub: the send goes through but nothing is delivered.
        let sent = self
            .store
            .mailer
            .send(email)
            .await
            .map_err(|e| Status::internal(format!("failed to send test email: {e}")))?;
        let delivered = sent && self.store.settings.mailer_enabled;
        Ok(Response::new(SendTestEmailResponse {
            delivered,
            to: user.email,
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = request.get_ref().customer_id.as_deref()))]
    async fn list_sent_emails(
        &self,
        request: Request<ListSentEmailsRequest>,
    ) -> Result<Response<ListSentEmailsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let (entity_type, entity_id) = match (req.entity_type.as_deref(), req.entity_id.as_deref())
        {
            (None, None) => (None, None),
            (Some(t), Some(id)) => {
                let entity_type = EntityType::from_str(t)
                    .map_err(|_| Status::invalid_argument(format!("unknown entity_type: {t}")))?;
                let entity_id = entity_type
                    .parse_id_proto(id)
                    .map_err(|e| Status::invalid_argument(format!("invalid entity_id: {e}")))?;
                (Some(entity_type.to_string()), Some(entity_id))
            }
            _ => {
                return Err(Status::invalid_argument(
                    "entity_type and entity_id must be given together",
                ));
            }
        };
        let filter = SentEmailFilter {
            invoicing_entity_id: InvoicingEntityId::from_proto_opt(req.invoicing_entity_id)?,
            kind: req
                .kind
                .map(mapping::kind_from_proto)
                .transpose()?
                .map(|k| k.to_string()),
            entity_type,
            entity_id,
            customer_id: CustomerId::from_proto_opt(req.customer_id)?.map(|c| c.as_uuid()),
        };
        let cursor = req
            .cursor
            .as_deref()
            .filter(|c| !c.is_empty())
            .map(mapping::decode_cursor)
            .transpose()?;
        let limit = if req.limit == 0 {
            SENT_EMAILS_DEFAULT_LIMIT
        } else {
            req.limit
        };
        let page = self
            .store
            .list_sent_emails(tenant_id, filter, cursor, limit)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;

        let names = self
            .row_names(
                tenant_id,
                page.items
                    .iter()
                    .map(|i| (i.entity_type.as_str(), i.entity_id, i.customer_id)),
            )
            .await?;

        let emails = page
            .items
            .into_iter()
            .map(|item| {
                let (entity_label, customer_name) =
                    row_labels(&names, &item.entity_type, item.entity_id, item.customer_id);
                mapping::summary_to_proto(item, entity_label, customer_name)
            })
            .collect();
        Ok(Response::new(ListSentEmailsResponse {
            emails,
            next_cursor: page.next_cursor.map(mapping::encode_cursor),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_skipped_emails(
        &self,
        request: Request<ListSkippedEmailsRequest>,
    ) -> Result<Response<ListSkippedEmailsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let filter = SkippedEmailFilter {
            invoicing_entity_id: InvoicingEntityId::from_proto_opt(req.invoicing_entity_id)?,
            kind: req
                .kind
                .map(mapping::kind_from_proto)
                .transpose()?
                .map(|k| k.to_string()),
        };
        let cursor = req
            .cursor
            .as_deref()
            .filter(|c| !c.is_empty())
            .map(mapping::decode_cursor)
            .transpose()?;
        let limit = if req.limit == 0 {
            SENT_EMAILS_DEFAULT_LIMIT
        } else {
            req.limit
        };
        let page = self
            .store
            .list_skipped_emails(tenant_id, filter, cursor, limit)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;

        let names = self
            .row_names(
                tenant_id,
                page.items
                    .iter()
                    .map(|i| (i.entity_type.as_str(), i.entity_id, i.customer_id)),
            )
            .await?;
        let emails = page
            .items
            .into_iter()
            .map(|item| {
                let (entity_label, customer_name) =
                    row_labels(&names, &item.entity_type, item.entity_id, item.customer_id);
                mapping::skipped_summary_to_proto(item, entity_label, customer_name)
            })
            .collect();
        Ok(Response::new(ListSkippedEmailsResponse {
            emails,
            next_cursor: page.next_cursor.map(mapping::encode_cursor),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_sent_email(
        &self,
        request: Request<GetSentEmailRequest>,
    ) -> Result<Response<GetSentEmailResponse>, Status> {
        let tenant_id = request.tenant()?;
        let id = EntityActivityId::from_proto(&request.into_inner().id)?;
        let detail = self
            .store
            .get_sent_email_detail(tenant_id, id)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        let attachments = mapping::parse_attachments(detail.email.attachments.as_ref());
        // Only mint a token when something is actually downloadable.
        let attachments_share_key = if attachments.iter().any(|a| !a.id.is_empty()) {
            let exp = (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize;
            Some(
                generate_entity_share_key(
                    detail.email.id.as_uuid(),
                    tenant_id,
                    &self.store.settings.jwt_secret,
                    exp,
                )
                .map_err(Into::<CustomerEmailsApiError>::into)?,
            )
        } else {
            None
        };
        Ok(Response::new(GetSentEmailResponse {
            email: Some(ProtoSentEmail {
                activity_id: detail.email.id.as_proto(),
                subject: detail.email.subject,
                from_addr: detail.email.from_addr,
                reply_to: detail.email.reply_to,
                recipients: detail.email.recipients,
                body_html: detail.email.body_html,
                attachments,
                sent_at: detail.email.sent_at.as_proto(),
                attachments_share_key,
                kind: mapping::kind_to_i32(detail.kind),
                locale: detail.locale,
                resent_from: detail.resent_from.map(|r| r.as_proto()),
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn resend_email(
        &self,
        request: Request<ResendEmailRequest>,
    ) -> Result<Response<ResendEmailResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let sent_email_id = EntityActivityId::from_proto(&req.sent_email_id)?;
        self.store
            .resend_email(actor, tenant_id, sent_email_id, &req.request_id)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        Ok(Response::new(ResendEmailResponse {}))
    }

    async fn resend_unknown_email(
        &self,
        request: Request<ResendUnknownEmailRequest>,
    ) -> Result<Response<ResendUnknownEmailResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let activity_id = EntityActivityId::from_proto(&req.activity_id)?;
        self.store
            .resend_unknown_email(actor, tenant_id, activity_id, &req.request_id)
            .await
            .map_err(Into::<CustomerEmailsApiError>::into)?;
        Ok(Response::new(ResendUnknownEmailResponse {}))
    }
}
