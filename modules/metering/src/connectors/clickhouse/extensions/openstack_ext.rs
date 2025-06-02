// init the clickhouse MV

use crate::connectors::clickhouse::ClickhouseConnector;
use crate::connectors::clickhouse::extensions::ConnectorClickhouseExtension;
use crate::connectors::clickhouse::sql::DATABASE;
use crate::connectors::clickhouse::sql::init::get_events_table_name;
use crate::connectors::errors::ConnectorError;
use crate::domain::WindowSize;
use crate::domain::{QueryMeterParams, Usage};
use chrono::{DateTime, Utc};
use clickhouse::Client;

/**
 * OpenstackClickhouseExtension is a Clickhouse extension that provides
 * custom queries for some Openstack-related events with custom aggregation/query requirements.
 * Currently, it only supports the "openstack.instance.uptime" event. Other events are supported via standard queries (ex: bandwidth)
 */
pub(crate) struct OpenstackClickhouseExtension {}

#[async_trait::async_trait]
impl ConnectorClickhouseExtension for OpenstackClickhouseExtension {
    fn prefix(&self) -> String {
        "openstack".to_string()
    }

    async fn init(&self, _client: Arc<Client>) -> error_stack::Result<(), ConnectorError> {
        // let instance_mv_dll = get_instance_mv_dll();
        // self.clickhouse_connector.execute_ddl(instance_mv_dll).await?;
        log::info!("Openstack extension enabled");
        Ok(())
    }

    fn build_query(&self, params: &QueryMeterParams) -> Option<String> {
        if params.code == "openstack.instance.uptime" {
            let params = params.clone();
            let cust = params.customer_ids[0].clone();

            let query = build_openstack_instance_query(&QueryOpenStackInstanceParams {
                customer_id: cust.id,
                tenant_id: params.namespace,
                from: params.from,
                to: params.to,
                window_size: params.window_size,
                group_by_flavor: params.group_by.iter().find(|&x| x == "flavor").is_some(),
            });

            Some(query)
        } else {
            // we rely on standard query
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryOpenStackInstanceParams {
    pub customer_id: String,
    pub tenant_id: String,
    pub from: DateTime<Utc>,
    pub to: Option<DateTime<Utc>>,
    pub window_size: Option<WindowSize>,
    pub group_by_flavor: bool,
}

pub fn build_openstack_instance_query(params: &QueryOpenStackInstanceParams) -> String {
    let to = params.to.unwrap_or_else(Utc::now);

    let window_function = match params.window_size {
        Some(WindowSize::Minute) => "toStartOfMinute",
        Some(WindowSize::Hour) => "toStartOfHour",
        Some(WindowSize::Day) => "toStartOfDay",
        None => "toStartOfDay",
    };

    let seconds_in_interval = match params.window_size {
        Some(WindowSize::Minute) => 60,
        Some(WindowSize::Hour) => 3600,
        Some(WindowSize::Day) => 86400,
        None => to.timestamp() - params.from.timestamp(),
    };

    let flavor_select = if params.group_by_flavor {
        ", ip.flavor as flavor"
    } else {
        ""
    };
    let flavor_group_by = if params.group_by_flavor {
        ", ip.flavor"
    } else {
        ""
    };
    let flavor_order_by = if params.group_by_flavor {
        ", ip.flavor"
    } else {
        ""
    };

    format!(
        r#"
WITH
    params AS (
        SELECT
            toDateTime('{from}') AS window_start,
            toDateTime('{to}') AS window_end,
            '{customer_id}' AS param_customer_id,
            '{tenant_id}' AS param_tenant_id
    ),
    event_lifecycle AS (
        SELECT
            customer_id,
            tenant_id,
            properties['instance_id'] as instance_id,
            properties['flavor'] as flavor,
            timestamp AS start_time,
            leadInFrame(toNullable(timestamp)) OVER (
                PARTITION BY customer_id, tenant_id, properties['instance_id']
                ORDER BY timestamp
                ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING
            ) AS end_time,
            code
        FROM meteroid.raw_events
        WHERE raw_events.code IN ('compute.instance.create.end', 'compute.instance.delete.end', 'compute.instance.resize.confirm.end')
    ),
    instance_periods AS (
        SELECT
            mv.customer_id,
            mv.tenant_id,
            mv.instance_id,
            mv.flavor,
            greatest(mv.start_time, params.window_start) AS period_start,
            least(COALESCE(mv.end_time, params.window_end), params.window_end) AS period_end
        FROM event_lifecycle mv
        CROSS JOIN params
        WHERE mv.customer_id = params.param_customer_id
          AND mv.tenant_id = params.param_tenant_id
          AND mv.start_time < params.window_end
          AND (mv.end_time > params.window_start OR mv.end_time IS NULL)
    ),
    time_series AS (
        SELECT
            toDateTime({window_function}(window_start) + number * {seconds_in_interval}) AS windowstart,
            toDateTime({window_function}(window_start) + (number + 1) * {seconds_in_interval}) AS windowend
        FROM params
        ARRAY JOIN range(toUInt32((toUnixTimestamp(window_end) - toUnixTimestamp({window_function}(window_start))) / {seconds_in_interval}) + 1) AS number
    )
SELECT
    ts.windowstart as windowstart,
    ts.windowend as windowend,
    sum(
        if(ip.period_start < ts.windowend AND ip.period_end > ts.windowstart,
           dateDiff(
               'second',
               greatest(ts.windowstart, ip.period_start),
               least(ts.windowend, ip.period_end)
           ),
           0
        )
    ) AS value,
    params.param_customer_id AS customer_id,
    params.param_tenant_id AS tenant_id{flavor_select}
FROM time_series ts
CROSS JOIN params
CROSS JOIN instance_periods ip
WHERE ip.period_start < ts.windowend AND ip.period_end > ts.windowstart
GROUP BY ts.windowstart, ts.windowend, customer_id, tenant_id{flavor_group_by}
ORDER BY ts.windowstart{flavor_order_by}
"#,
        from = params.from.format("%Y-%m-%d %H:%M:%S"),
        to = to.format("%Y-%m-%d %H:%M:%S"),
        customer_id = params.customer_id,
        tenant_id = params.tenant_id,
        window_function = window_function,
        seconds_in_interval = seconds_in_interval,
        flavor_select = flavor_select,
        flavor_group_by = flavor_group_by,
        flavor_order_by = flavor_order_by,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_hourly_window_with_flavor_grouping() {
        let params = QueryOpenStackInstanceParams {
            customer_id: "customer1".to_string(),
            tenant_id: "tenant1".to_string(),
            from: Utc.with_ymd_and_hms(2023, 6, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2023, 6, 2, 0, 0, 0).unwrap()),
            window_size: Some(WindowSize::Hour),
            group_by_flavor: true,
        };

        let query = build_openstack_instance_query(&params);

        println!("{}", query);

        assert!(query.contains("toStartOfHour"));
        assert!(
            query.contains(
                "GROUP BY ts.windowstart, ts.windowend, customer_id, tenant_id, ip.flavor"
            )
        );
        assert!(query.contains("ORDER BY ts.windowstart, ip.flavor"));
    }

    #[test]
    fn test_daily_window_without_flavor_grouping() {
        let params = QueryOpenStackInstanceParams {
            customer_id: "customer2".to_string(),
            tenant_id: "tenant2".to_string(),
            from: Utc.with_ymd_and_hms(2023, 7, 1, 0, 0, 0).unwrap(),
            to: None,
            window_size: Some(WindowSize::Day),
            group_by_flavor: false,
        };

        let query = build_openstack_instance_query(&params);
        println!("{}", query);

        assert!(query.contains("toStartOfDay"));
        assert!(query.contains("GROUP BY ts.windowstart, ts.windowend, customer_id, tenant_id"));
        assert!(!query.contains("ip.flavor"));
        assert!(query.contains("ORDER BY ts.windowstart"));
    }

    #[test]
    fn test_minute_window_with_specific_time_range() {
        let params = QueryOpenStackInstanceParams {
            customer_id: "customer3".to_string(),
            tenant_id: "tenant3".to_string(),
            from: Utc.with_ymd_and_hms(2023, 8, 1, 12, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2023, 8, 1, 13, 0, 0).unwrap()),
            window_size: Some(WindowSize::Minute),
            group_by_flavor: true,
        };

        let query = build_openstack_instance_query(&params);
        println!("{}", query);

        assert!(query.contains("toStartOfMinute"));
        assert!(query.contains("toDateTime('2023-08-01 12:00:00') AS window_start"));
        assert!(query.contains("toDateTime('2023-08-01 13:00:00') AS window_end"));
        assert!(
            query.contains(
                "GROUP BY ts.windowstart, ts.windowend, customer_id, tenant_id, ip.flavor"
            )
        );
    }
}
