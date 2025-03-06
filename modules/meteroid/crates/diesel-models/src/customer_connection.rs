use crate::connectors::ConnectorRow;
use crate::customers::CustomerRow;
use crate::enums::PaymentMethodTypeEnum;
use crate::schema::customer_connection;
use common_domain::ids::{ConnectorId, CustomerConnectionId, CustomerId};
use diesel::{Insertable, Queryable, Selectable};

#[derive(Queryable, Insertable, Debug, Selectable)]
#[diesel(table_name = customer_connection)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerConnectionRow {
    pub id: CustomerConnectionId,
    pub customer_id: CustomerId,
    pub connector_id: ConnectorId,
    pub supported_payment_types: Option<Vec<Option<PaymentMethodTypeEnum>>>,
    pub external_customer_id: String,
}

#[derive(Queryable, Debug, Selectable)]
#[diesel(table_name = customer_connection)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerConnectionDetailsRow {
    #[diesel(select_expression = customer_connection::id)]
    #[diesel(select_expression_type = customer_connection::id)]
    pub id: CustomerConnectionId,
    #[diesel(select_expression = customer_connection::supported_payment_types)]
    #[diesel(select_expression_type = customer_connection::supported_payment_types)]
    pub supported_payment_types: Option<Vec<Option<PaymentMethodTypeEnum>>>,
    #[diesel(select_expression = customer_connection::external_customer_id)]
    #[diesel(select_expression_type = customer_connection::external_customer_id)]
    pub external_customer_id: String,
    #[diesel(embed)]
    pub customer: CustomerRow,
    #[diesel(embed)]
    pub connector: ConnectorRow,
}
