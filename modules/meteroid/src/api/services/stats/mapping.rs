use crate::services::stats::stats_service::{
    CountAndValue, MRRBreakdown, MRRBreakdownScope, MrrMovementType, Trend, TrendScope,
};
use meteroid_grpc::meteroid::api::stats::v1 as proto;
use meteroid_grpc::meteroid::api::stats::v1::{BreakdownStat, MrrBreakdown, MrrBreakdownScope};

pub fn trend_to_server(trend: &Trend) -> proto::Trend {
    proto::Trend {
        current: trend.current,
        change_amount: trend.change_amount,
        change_percent: trend.change_percent,
        positive_is_good: trend.positive_is_good,
        scope: trend_scope_to_server(&trend.scope).into(),
    }
}

pub fn trend_scope_to_server(scope: &TrendScope) -> proto::TrendScope {
    match scope {
        TrendScope::Trend24h => proto::TrendScope::Trend24h,
        TrendScope::Trend7d => proto::TrendScope::Trend7d,
        TrendScope::Trend30d => proto::TrendScope::Trend30d,
        TrendScope::Trend90d => proto::TrendScope::Trend90d,
        TrendScope::Trend1y => proto::TrendScope::Trend1y,
        TrendScope::Trend2y => proto::TrendScope::Trend2y,
    }
}

pub fn mrr_breakdown_scope_from_server(scope: proto::MrrBreakdownScope) -> MRRBreakdownScope {
    match scope {
        MrrBreakdownScope::ThisWeek => MRRBreakdownScope::ThisWeek,
        MrrBreakdownScope::ThisMonth => MRRBreakdownScope::ThisMonth,
        MrrBreakdownScope::ThisQuarter => MRRBreakdownScope::ThisQuarter,
        MrrBreakdownScope::ThisYear => MRRBreakdownScope::ThisYear,
        MrrBreakdownScope::LastWeek => MRRBreakdownScope::LastWeek,
        MrrBreakdownScope::LastMonth => MRRBreakdownScope::LastMonth,
        MrrBreakdownScope::LastQuarter => MRRBreakdownScope::LastQuarter,
        MrrBreakdownScope::LastYear => MRRBreakdownScope::LastYear,
    }
}

pub fn breakdown_stat_to_server(stat: &CountAndValue) -> BreakdownStat {
    BreakdownStat {
        count: stat.count,
        value: stat.value,
    }
}
pub fn mrr_breakdown_to_server(breakdown: &MRRBreakdown) -> MrrBreakdown {
    MrrBreakdown {
        churn: Some(breakdown_stat_to_server(&breakdown.churn)),
        contraction: Some(breakdown_stat_to_server(&breakdown.contraction)),
        expansion: Some(breakdown_stat_to_server(&breakdown.expansion)),
        new_business: Some(breakdown_stat_to_server(&breakdown.new_business)),
        reactivation: Some(breakdown_stat_to_server(&breakdown.reactivation)),
        net_new_mrr: breakdown.net_new_mrr.clone(),
    }
}

pub fn map_mrr_type(m: MrrMovementType) -> proto::MrrMovementType {
    match m {
        MrrMovementType::Churn => proto::MrrMovementType::Churn,
        MrrMovementType::Contraction => proto::MrrMovementType::Contraction,
        MrrMovementType::Expansion => proto::MrrMovementType::Expansion,
        MrrMovementType::NewBusiness => proto::MrrMovementType::NewBusiness,
        MrrMovementType::Reactivation => proto::MrrMovementType::Reactivation,
    }
}
