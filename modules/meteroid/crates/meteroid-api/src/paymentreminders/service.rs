use common_domain::ids::{CustomerId, InvoiceId, InvoicingEntityId, PaymentReminderSequenceId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::features::{EMAIL_CUSTOMIZATION_NOT_IN_PLAN, ensure_change_allowed, gates};
use meteroid_grpc::meteroid::api::paymentreminders::v1::{
    ArchivePaymentReminderSequenceRequest, ArchivePaymentReminderSequenceResponse,
    CreatePaymentReminderSequenceRequest, CreatePaymentReminderSequenceResponse,
    GetCustomerReminderPreferenceRequest, GetCustomerReminderPreferenceResponse,
    GetInvoiceRemindersRequest, GetInvoiceRemindersResponse, GetPaymentReminderSequenceRequest,
    GetPaymentReminderSequenceResponse, ListPaymentReminderSequencesRequest,
    ListPaymentReminderSequencesResponse, PauseInvoiceRemindersRequest,
    PauseInvoiceRemindersResponse, PreviewInvoiceReminderResolutionRequest,
    PreviewInvoiceReminderResolutionResponse, PreviewPaymentReminderAudienceRequest,
    PreviewPaymentReminderAudienceResponse, ReorderPaymentReminderSequencesRequest,
    ReorderPaymentReminderSequencesResponse, RestartInvoiceRemindersRequest,
    RestartInvoiceRemindersResponse, ResumeInvoiceRemindersRequest, ResumeInvoiceRemindersResponse,
    SendInvoiceReminderNowRequest, SendInvoiceReminderNowResponse,
    SetCustomerReminderPreferenceRequest, SetCustomerReminderPreferenceResponse,
    StopInvoiceRemindersRequest, StopInvoiceRemindersResponse,
    UpdatePaymentReminderSequenceRequest, UpdatePaymentReminderSequenceResponse,
    payment_reminders_service_server::PaymentRemindersService,
};
use meteroid_store::domain::payment_reminders::{
    CustomerReminderPreference, PaymentReminderSequenceNew, PaymentReminderSequencePatch,
    PaymentReminderStep, PaymentReminderStepInput,
};
use meteroid_store::repositories::PaymentRemindersInterface;
use tonic::{Request, Response, Status};

use super::PaymentRemindersServiceComponents;
use super::error::PaymentRemindersApiError;
use super::mapping::sequences as mapping;

const AUDIENCE_PREVIEW_SAMPLE: usize = 10;

#[tonic::async_trait]
impl PaymentRemindersService for PaymentRemindersServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_payment_reminder_sequences(
        &self,
        request: Request<ListPaymentReminderSequencesRequest>,
    ) -> Result<Response<ListPaymentReminderSequencesResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();
        let entity = InvoicingEntityId::from_proto(req.invoicing_entity_id)?;

        let sequences = self
            .store
            .list_reminder_sequences(tenant, entity, req.include_archived)
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?
            .into_iter()
            .map(mapping::domain_to_proto)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Response::new(ListPaymentReminderSequencesResponse {
            sequences,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_payment_reminder_sequence(
        &self,
        request: Request<GetPaymentReminderSequenceRequest>,
    ) -> Result<Response<GetPaymentReminderSequenceResponse>, Status> {
        let tenant = request.tenant()?;
        let id = PaymentReminderSequenceId::from_proto(request.into_inner().id)?;

        let sequence = self
            .store
            .get_reminder_sequence(tenant, id)
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(GetPaymentReminderSequenceResponse {
            sequence: Some(mapping::domain_to_proto(sequence)?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_payment_reminder_sequence(
        &self,
        request: Request<CreatePaymentReminderSequenceRequest>,
    ) -> Result<Response<CreatePaymentReminderSequenceResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let organization = request.organization()?;
        let req = request.into_inner();
        let audience = req
            .audience
            .as_deref()
            .map(mapping::audience_from_proto)
            .transpose()?;
        let steps = mapping::steps_from_proto(req.steps);
        self.ensure_copy_allowed(organization, introduces_copy(&[], &steps))
            .await?;

        let sequence = self
            .store
            .create_reminder_sequence(
                actor,
                tenant,
                PaymentReminderSequenceNew {
                    invoicing_entity_id: InvoicingEntityId::from_proto(req.invoicing_entity_id)?,
                    name: req.name,
                    enabled: req.enabled,
                    audience,
                    steps,
                },
            )
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(CreatePaymentReminderSequenceResponse {
            sequence: Some(mapping::domain_to_proto(sequence)?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_payment_reminder_sequence(
        &self,
        request: Request<UpdatePaymentReminderSequenceRequest>,
    ) -> Result<Response<UpdatePaymentReminderSequenceResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let organization = request.organization()?;
        let req = request.into_inner();
        let id = PaymentReminderSequenceId::from_proto(req.id)?;
        let audience = match req.audience.as_deref() {
            None => None,
            Some("") => Some(None),
            Some(json) => Some(Some(mapping::audience_from_proto(json)?)),
        };
        let steps = req.steps.map(|list| mapping::steps_from_proto(list.steps));
        if let Some(steps) = steps.as_deref()
            && steps.iter().any(|s| !s.content.0.is_empty())
        {
            let stored = self
                .store
                .get_reminder_sequence(tenant, id)
                .await
                .map_err(Into::<PaymentRemindersApiError>::into)?;
            self.ensure_copy_allowed(organization, introduces_copy(&stored.sequence.steps, steps))
                .await?;
        }

        let sequence = self
            .store
            .update_reminder_sequence(
                actor,
                tenant,
                PaymentReminderSequencePatch {
                    id,
                    name: req.name,
                    enabled: req.enabled,
                    audience,
                    steps,
                },
                chrono::Utc::now().naive_utc(),
            )
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(UpdatePaymentReminderSequenceResponse {
            sequence: Some(mapping::domain_to_proto(sequence)?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn reorder_payment_reminder_sequences(
        &self,
        request: Request<ReorderPaymentReminderSequencesRequest>,
    ) -> Result<Response<ReorderPaymentReminderSequencesResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let ids = req
            .ids
            .into_iter()
            .map(PaymentReminderSequenceId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let sequences = self
            .store
            .reorder_reminder_sequences(
                actor,
                tenant,
                InvoicingEntityId::from_proto(req.invoicing_entity_id)?,
                ids,
            )
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?
            .into_iter()
            .map(mapping::domain_to_proto)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Response::new(ReorderPaymentReminderSequencesResponse {
            sequences,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn archive_payment_reminder_sequence(
        &self,
        request: Request<ArchivePaymentReminderSequenceRequest>,
    ) -> Result<Response<ArchivePaymentReminderSequenceResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let id = PaymentReminderSequenceId::from_proto(request.into_inner().id)?;

        let stopped = self
            .store
            .archive_reminder_sequence(actor, tenant, id, chrono::Utc::now().naive_utc())
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(ArchivePaymentReminderSequenceResponse {
            stopped_invoices: stopped as i32,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_payment_reminder_audience(
        &self,
        request: Request<PreviewPaymentReminderAudienceRequest>,
    ) -> Result<Response<PreviewPaymentReminderAudienceResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();
        let audience = mapping::audience_from_proto(&req.audience)?;

        let preview = self
            .store
            .preview_audience(
                tenant,
                InvoicingEntityId::from_proto(req.invoicing_entity_id)?,
                &audience,
                PaymentReminderSequenceId::from_proto_opt(req.sequence_id)?,
                AUDIENCE_PREVIEW_SAMPLE,
            )
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(PreviewPaymentReminderAudienceResponse {
            preview: Some(mapping::preview_to_proto(preview)),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn get_invoice_reminders(
        &self,
        request: Request<GetInvoiceRemindersRequest>,
    ) -> Result<Response<GetInvoiceRemindersResponse>, Status> {
        let tenant = request.tenant()?;
        let invoice_id = InvoiceId::from_proto(request.into_inner().invoice_id)?;

        let state = self
            .store
            .get_invoice_reminder_state(tenant, invoice_id, chrono::Utc::now().naive_utc())
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(GetInvoiceRemindersResponse {
            state: Some(mapping::invoice_state_to_proto(state)),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn preview_invoice_reminder_resolution(
        &self,
        request: Request<PreviewInvoiceReminderResolutionRequest>,
    ) -> Result<Response<PreviewInvoiceReminderResolutionResponse>, Status> {
        let tenant = request.tenant()?;
        let invoice_id = InvoiceId::from_proto(request.into_inner().invoice_id)?;

        let rows = self
            .store
            .preview_invoice_reminder_resolution(tenant, invoice_id)
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?
            .into_iter()
            .map(mapping::resolution_row_to_proto)
            .collect();

        Ok(Response::new(PreviewInvoiceReminderResolutionResponse {
            rows,
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn pause_invoice_reminders(
        &self,
        request: Request<PauseInvoiceRemindersRequest>,
    ) -> Result<Response<PauseInvoiceRemindersResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let until = mapping::parse_timestamp("until", &req.until)?;

        let state = self
            .store
            .pause_invoice_reminders(
                actor,
                tenant,
                InvoiceId::from_proto(req.invoice_id)?,
                until,
                chrono::Utc::now().naive_utc(),
            )
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(PauseInvoiceRemindersResponse {
            state: Some(mapping::invoice_state_to_proto(state)),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn resume_invoice_reminders(
        &self,
        request: Request<ResumeInvoiceRemindersRequest>,
    ) -> Result<Response<ResumeInvoiceRemindersResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let invoice_id = InvoiceId::from_proto(request.into_inner().invoice_id)?;

        let state = self
            .store
            .resume_invoice_reminders(actor, tenant, invoice_id, chrono::Utc::now().naive_utc())
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(ResumeInvoiceRemindersResponse {
            state: Some(mapping::invoice_state_to_proto(state)),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn stop_invoice_reminders(
        &self,
        request: Request<StopInvoiceRemindersRequest>,
    ) -> Result<Response<StopInvoiceRemindersResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let invoice_id = InvoiceId::from_proto(request.into_inner().invoice_id)?;

        let state = self
            .store
            .stop_invoice_reminders(actor, tenant, invoice_id, chrono::Utc::now().naive_utc())
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(StopInvoiceRemindersResponse {
            state: Some(mapping::invoice_state_to_proto(state)),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn restart_invoice_reminders(
        &self,
        request: Request<RestartInvoiceRemindersRequest>,
    ) -> Result<Response<RestartInvoiceRemindersResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let invoice_id = InvoiceId::from_proto(request.into_inner().invoice_id)?;

        let state = self
            .store
            .restart_invoice_reminders(actor, tenant, invoice_id, chrono::Utc::now().naive_utc())
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(RestartInvoiceRemindersResponse {
            state: Some(mapping::invoice_state_to_proto(state)),
        }))
    }

    #[tracing::instrument(skip_all, fields(invoice_id = %request.get_ref().invoice_id))]
    async fn send_invoice_reminder_now(
        &self,
        request: Request<SendInvoiceReminderNowRequest>,
    ) -> Result<Response<SendInvoiceReminderNowResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        self.store
            .send_invoice_reminder_now(
                actor,
                tenant,
                InvoiceId::from_proto(req.invoice_id)?,
                req.offset_days,
                &req.request_id,
                chrono::Utc::now().naive_utc(),
            )
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(SendInvoiceReminderNowResponse {}))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn get_customer_reminder_preference(
        &self,
        request: Request<GetCustomerReminderPreferenceRequest>,
    ) -> Result<Response<GetCustomerReminderPreferenceResponse>, Status> {
        let tenant = request.tenant()?;
        let customer = CustomerId::from_proto(request.into_inner().customer_id)?;

        let view = self
            .store
            .get_customer_reminder_preference(tenant, customer)
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(GetCustomerReminderPreferenceResponse {
            preference: Some(mapping::preference_to_proto(view)),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn set_customer_reminder_preference(
        &self,
        request: Request<SetCustomerReminderPreferenceRequest>,
    ) -> Result<Response<SetCustomerReminderPreferenceResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let view = self
            .store
            .set_customer_reminder_preference(
                actor,
                tenant,
                CustomerId::from_proto(req.customer_id)?,
                CustomerReminderPreference {
                    opt_out: req.opt_out,
                    sequence_id: PaymentReminderSequenceId::from_proto_opt(req.sequence_id)?,
                },
                chrono::Utc::now().naive_utc(),
            )
            .await
            .map_err(Into::<PaymentRemindersApiError>::into)?;

        Ok(Response::new(SetCustomerReminderPreferenceResponse {
            preference: Some(mapping::preference_to_proto(view)),
        }))
    }
}

impl PaymentRemindersServiceComponents {
    async fn ensure_copy_allowed(
        &self,
        organization_id: common_domain::ids::OrganizationId,
        changes: bool,
    ) -> Result<(), Status> {
        ensure_change_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            gates::EMAIL_CUSTOMIZATION,
            changes,
            EMAIL_CUSTOMIZATION_NOT_IN_PLAN,
        )
        .await
    }
}

/// Whether a step carries copy that no stored step of the sequence already holds. Keeping, moving,
/// or dropping some or all languages of stored copy is not a change: it keeps being sent.
fn introduces_copy(stored: &[PaymentReminderStep], steps: &[PaymentReminderStepInput]) -> bool {
    steps.iter().any(|step| {
        !step.content.0.is_empty()
            && !stored.iter().any(|s| {
                step.content
                    .0
                    .iter()
                    .all(|(locale, copy)| s.content.0.get(locale) == Some(copy))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::{BaseId, PaymentReminderStepId};
    use meteroid_store::domain::payment_reminders::{
        EmailCopy, EmailCopyByLocale, PaymentReminderStepAction,
    };
    use std::collections::BTreeMap;

    fn copy(subject: &str) -> EmailCopyByLocale {
        copy_in(&["en-US"], subject)
    }

    fn copy_in(locales: &[&str], subject: &str) -> EmailCopyByLocale {
        EmailCopyByLocale(
            locales
                .iter()
                .map(|locale| {
                    (
                        locale.to_string(),
                        EmailCopy {
                            subject: subject.to_string(),
                            body: "Please pay".to_string(),
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        )
    }

    fn input(offset_days: i32, content: EmailCopyByLocale) -> PaymentReminderStepInput {
        PaymentReminderStepInput {
            offset_days,
            content,
        }
    }

    fn stored(offset_days: i32, content: EmailCopyByLocale) -> PaymentReminderStep {
        PaymentReminderStep {
            id: PaymentReminderStepId::new(),
            offset_days,
            action: PaymentReminderStepAction::Notify,
            content,
        }
    }

    #[test]
    fn built_in_copy_is_never_a_copy_change() {
        assert!(!introduces_copy(
            &[],
            &[input(-3, EmailCopyByLocale::default())]
        ));
    }

    #[test]
    fn new_copy_is_a_change() {
        assert!(introduces_copy(&[], &[input(7, copy("Due"))]));
        assert!(introduces_copy(
            &[stored(7, copy("Due"))],
            &[input(7, copy("Overdue"))]
        ));
    }

    #[test]
    fn stored_copy_may_be_kept_moved_or_cleared() {
        let steps = [stored(7, copy("Due"))];
        assert!(!introduces_copy(&steps, &[input(7, copy("Due"))]));
        assert!(!introduces_copy(&steps, &[input(14, copy("Due"))]));
        assert!(!introduces_copy(
            &steps,
            &[input(7, EmailCopyByLocale::default())]
        ));
        assert!(!introduces_copy(&steps, &[]));
    }

    #[test]
    fn languages_of_stored_copy_may_be_dropped_not_added() {
        let steps = [stored(7, copy_in(&["en-US", "fr-FR"], "Due"))];
        assert!(!introduces_copy(
            &steps,
            &[input(7, copy_in(&["fr-FR"], "Due"))]
        ));
        assert!(introduces_copy(
            &steps,
            &[input(7, copy_in(&["en-US", "fr-FR", "de-DE"], "Due"))]
        ));
    }
}
