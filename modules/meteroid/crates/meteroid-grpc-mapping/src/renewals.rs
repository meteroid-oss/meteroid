//! Renewals (R-160) and their notices, for the core renewals service and the retention overview.
use chrono::NaiveDate;
use meteroid_grpc::meteroid::api::renewals::v1 as proto;
use meteroid_store::domain::renewals::{
    NoticeAudience, Renewal, RenewalMode, RenewalNotice, RenewalSettings, RenewalSource,
};

use crate::wallets::timestamp;

pub fn mode_to_proto(mode: RenewalMode) -> proto::RenewalMode {
    match mode {
        RenewalMode::Renewing => proto::RenewalMode::Renewing,
        RenewalMode::Ending => proto::RenewalMode::Ending,
        RenewalMode::Cancelling => proto::RenewalMode::Cancelling,
    }
}

fn source_to_proto(source: RenewalSource) -> proto::RenewalSource {
    match source {
        RenewalSource::Term => proto::RenewalSource::Term,
        RenewalSource::Period => proto::RenewalSource::Period,
        RenewalSource::EndDate => proto::RenewalSource::EndDate,
        RenewalSource::Cancellation => proto::RenewalSource::Cancellation,
    }
}

pub fn renewal_to_proto(renewal: Renewal, today: NaiveDate) -> proto::Renewal {
    proto::Renewal {
        subscription_id: renewal.subscription_id.to_string(),
        customer_id: renewal.customer_id.to_string(),
        customer_name: renewal.customer_name,
        plan_name: renewal.plan_name,
        currency: renewal.currency,
        mrr_cents: renewal.mrr_cents,
        date: renewal.date.to_string(),
        mode: mode_to_proto(renewal.mode).into(),
        source: source_to_proto(renewal.source).into(),
        days_left: u32::try_from((renewal.date - today).num_days()).unwrap_or(0),
        opt_out_deadline: renewal.opt_out_deadline.map(|d| d.to_string()),
    }
}

pub fn notice_to_proto(notice: RenewalNotice) -> proto::RenewalNotice {
    proto::RenewalNotice {
        renewal_date: notice.renewal_date.to_string(),
        mode: mode_to_proto(notice.mode).into(),
        audience: match notice.audience {
            NoticeAudience::Team => proto::NoticeAudience::Team,
            NoticeAudience::Customer => proto::NoticeAudience::Customer,
        }
        .into(),
        days_before: notice.days_before,
        notified_at: Some(timestamp(notice.notified_at)),
    }
}

pub fn settings_to_proto(settings: RenewalSettings) -> proto::RenewalSettings {
    proto::RenewalSettings {
        checkpoints: settings.checkpoints,
        customer_reminder_days: settings.customer_reminder_days,
    }
}
