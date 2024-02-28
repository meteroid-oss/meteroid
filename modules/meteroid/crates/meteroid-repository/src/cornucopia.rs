// This file was generated with `cornucopia`. Do not modify.

#[allow(clippy::all, clippy::pedantic)]
#[allow(unused_variables)]
#[allow(unused_imports)]
#[allow(dead_code)]
pub mod types {
    pub mod public {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum BillingMetricAggregateEnum {
            COUNT,
            LATEST,
            MAX,
            MIN,
            MEAN,
            SUM,
            COUNT_DISTINCT,
        }
        impl<'a> postgres_types::ToSql for BillingMetricAggregateEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    BillingMetricAggregateEnum::COUNT => "COUNT",
                    BillingMetricAggregateEnum::LATEST => "LATEST",
                    BillingMetricAggregateEnum::MAX => "MAX",
                    BillingMetricAggregateEnum::MIN => "MIN",
                    BillingMetricAggregateEnum::MEAN => "MEAN",
                    BillingMetricAggregateEnum::SUM => "SUM",
                    BillingMetricAggregateEnum::COUNT_DISTINCT => "COUNT_DISTINCT",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "BillingMetricAggregateEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 7 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "COUNT" => true,
                            "LATEST" => true,
                            "MAX" => true,
                            "MIN" => true,
                            "MEAN" => true,
                            "SUM" => true,
                            "COUNT_DISTINCT" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for BillingMetricAggregateEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<BillingMetricAggregateEnum, Box<dyn std::error::Error + Sync + Send>>
            {
                match std::str::from_utf8(buf)? {
                    "COUNT" => Ok(BillingMetricAggregateEnum::COUNT),
                    "LATEST" => Ok(BillingMetricAggregateEnum::LATEST),
                    "MAX" => Ok(BillingMetricAggregateEnum::MAX),
                    "MIN" => Ok(BillingMetricAggregateEnum::MIN),
                    "MEAN" => Ok(BillingMetricAggregateEnum::MEAN),
                    "SUM" => Ok(BillingMetricAggregateEnum::SUM),
                    "COUNT_DISTINCT" => Ok(BillingMetricAggregateEnum::COUNT_DISTINCT),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "BillingMetricAggregateEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 7 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "COUNT" => true,
                            "LATEST" => true,
                            "MAX" => true,
                            "MIN" => true,
                            "MEAN" => true,
                            "SUM" => true,
                            "COUNT_DISTINCT" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum UnitConversionRoundingEnum {
            UP,
            DOWN,
            NEAREST,
            NEAREST_HALF,
            NEAREST_DECILE,
            NONE,
        }
        impl<'a> postgres_types::ToSql for UnitConversionRoundingEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    UnitConversionRoundingEnum::UP => "UP",
                    UnitConversionRoundingEnum::DOWN => "DOWN",
                    UnitConversionRoundingEnum::NEAREST => "NEAREST",
                    UnitConversionRoundingEnum::NEAREST_HALF => "NEAREST_HALF",
                    UnitConversionRoundingEnum::NEAREST_DECILE => "NEAREST_DECILE",
                    UnitConversionRoundingEnum::NONE => "NONE",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "UnitConversionRoundingEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 6 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "UP" => true,
                            "DOWN" => true,
                            "NEAREST" => true,
                            "NEAREST_HALF" => true,
                            "NEAREST_DECILE" => true,
                            "NONE" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for UnitConversionRoundingEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<UnitConversionRoundingEnum, Box<dyn std::error::Error + Sync + Send>>
            {
                match std::str::from_utf8(buf)? {
                    "UP" => Ok(UnitConversionRoundingEnum::UP),
                    "DOWN" => Ok(UnitConversionRoundingEnum::DOWN),
                    "NEAREST" => Ok(UnitConversionRoundingEnum::NEAREST),
                    "NEAREST_HALF" => Ok(UnitConversionRoundingEnum::NEAREST_HALF),
                    "NEAREST_DECILE" => Ok(UnitConversionRoundingEnum::NEAREST_DECILE),
                    "NONE" => Ok(UnitConversionRoundingEnum::NONE),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "UnitConversionRoundingEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 6 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "UP" => true,
                            "DOWN" => true,
                            "NEAREST" => true,
                            "NEAREST_HALF" => true,
                            "NEAREST_DECILE" => true,
                            "NONE" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum InvoiceStatusEnum {
            DRAFT,
            FINALIZED,
            PENDING,
            VOID,
        }
        impl<'a> postgres_types::ToSql for InvoiceStatusEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    InvoiceStatusEnum::DRAFT => "DRAFT",
                    InvoiceStatusEnum::FINALIZED => "FINALIZED",
                    InvoiceStatusEnum::PENDING => "PENDING",
                    InvoiceStatusEnum::VOID => "VOID",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "InvoiceStatusEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 4 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "DRAFT" => true,
                            "FINALIZED" => true,
                            "PENDING" => true,
                            "VOID" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for InvoiceStatusEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<InvoiceStatusEnum, Box<dyn std::error::Error + Sync + Send>> {
                match std::str::from_utf8(buf)? {
                    "DRAFT" => Ok(InvoiceStatusEnum::DRAFT),
                    "FINALIZED" => Ok(InvoiceStatusEnum::FINALIZED),
                    "PENDING" => Ok(InvoiceStatusEnum::PENDING),
                    "VOID" => Ok(InvoiceStatusEnum::VOID),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "InvoiceStatusEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 4 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "DRAFT" => true,
                            "FINALIZED" => true,
                            "PENDING" => true,
                            "VOID" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum InvoicingProviderEnum {
            STRIPE,
        }
        impl<'a> postgres_types::ToSql for InvoicingProviderEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    InvoicingProviderEnum::STRIPE => "STRIPE",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "InvoicingProviderEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 1 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "STRIPE" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for InvoicingProviderEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<InvoicingProviderEnum, Box<dyn std::error::Error + Sync + Send>>
            {
                match std::str::from_utf8(buf)? {
                    "STRIPE" => Ok(InvoicingProviderEnum::STRIPE),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "InvoicingProviderEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 1 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "STRIPE" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum InvoiceExternalStatusEnum {
            DELETED,
            DRAFT,
            FINALIZED,
            PAID,
            PAYMENT_FAILED,
            UNCOLLECTIBLE,
            VOID,
        }
        impl<'a> postgres_types::ToSql for InvoiceExternalStatusEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    InvoiceExternalStatusEnum::DELETED => "DELETED",
                    InvoiceExternalStatusEnum::DRAFT => "DRAFT",
                    InvoiceExternalStatusEnum::FINALIZED => "FINALIZED",
                    InvoiceExternalStatusEnum::PAID => "PAID",
                    InvoiceExternalStatusEnum::PAYMENT_FAILED => "PAYMENT_FAILED",
                    InvoiceExternalStatusEnum::UNCOLLECTIBLE => "UNCOLLECTIBLE",
                    InvoiceExternalStatusEnum::VOID => "VOID",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "InvoiceExternalStatusEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 7 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "DELETED" => true,
                            "DRAFT" => true,
                            "FINALIZED" => true,
                            "PAID" => true,
                            "PAYMENT_FAILED" => true,
                            "UNCOLLECTIBLE" => true,
                            "VOID" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for InvoiceExternalStatusEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<InvoiceExternalStatusEnum, Box<dyn std::error::Error + Sync + Send>>
            {
                match std::str::from_utf8(buf)? {
                    "DELETED" => Ok(InvoiceExternalStatusEnum::DELETED),
                    "DRAFT" => Ok(InvoiceExternalStatusEnum::DRAFT),
                    "FINALIZED" => Ok(InvoiceExternalStatusEnum::FINALIZED),
                    "PAID" => Ok(InvoiceExternalStatusEnum::PAID),
                    "PAYMENT_FAILED" => Ok(InvoiceExternalStatusEnum::PAYMENT_FAILED),
                    "UNCOLLECTIBLE" => Ok(InvoiceExternalStatusEnum::UNCOLLECTIBLE),
                    "VOID" => Ok(InvoiceExternalStatusEnum::VOID),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "InvoiceExternalStatusEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 7 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "DELETED" => true,
                            "DRAFT" => true,
                            "FINALIZED" => true,
                            "PAID" => true,
                            "PAYMENT_FAILED" => true,
                            "UNCOLLECTIBLE" => true,
                            "VOID" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum OrganizationUserRole {
            ADMIN,
            MEMBER,
        }
        impl<'a> postgres_types::ToSql for OrganizationUserRole {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    OrganizationUserRole::ADMIN => "ADMIN",
                    OrganizationUserRole::MEMBER => "MEMBER",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "OrganizationUserRole" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 2 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "ADMIN" => true,
                            "MEMBER" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for OrganizationUserRole {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<OrganizationUserRole, Box<dyn std::error::Error + Sync + Send>>
            {
                match std::str::from_utf8(buf)? {
                    "ADMIN" => Ok(OrganizationUserRole::ADMIN),
                    "MEMBER" => Ok(OrganizationUserRole::MEMBER),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "OrganizationUserRole" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 2 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "ADMIN" => true,
                            "MEMBER" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum PlanStatusEnum {
            DRAFT,
            ACTIVE,
            INACTIVE,
            ARCHIVED,
        }
        impl<'a> postgres_types::ToSql for PlanStatusEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    PlanStatusEnum::DRAFT => "DRAFT",
                    PlanStatusEnum::ACTIVE => "ACTIVE",
                    PlanStatusEnum::INACTIVE => "INACTIVE",
                    PlanStatusEnum::ARCHIVED => "ARCHIVED",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "PlanStatusEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 4 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "DRAFT" => true,
                            "ACTIVE" => true,
                            "INACTIVE" => true,
                            "ARCHIVED" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for PlanStatusEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<PlanStatusEnum, Box<dyn std::error::Error + Sync + Send>> {
                match std::str::from_utf8(buf)? {
                    "DRAFT" => Ok(PlanStatusEnum::DRAFT),
                    "ACTIVE" => Ok(PlanStatusEnum::ACTIVE),
                    "INACTIVE" => Ok(PlanStatusEnum::INACTIVE),
                    "ARCHIVED" => Ok(PlanStatusEnum::ARCHIVED),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "PlanStatusEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 4 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "DRAFT" => true,
                            "ACTIVE" => true,
                            "INACTIVE" => true,
                            "ARCHIVED" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum PlanTypeEnum {
            STANDARD,
            FREE,
            CUSTOM,
        }
        impl<'a> postgres_types::ToSql for PlanTypeEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    PlanTypeEnum::STANDARD => "STANDARD",
                    PlanTypeEnum::FREE => "FREE",
                    PlanTypeEnum::CUSTOM => "CUSTOM",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "PlanTypeEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 3 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "STANDARD" => true,
                            "FREE" => true,
                            "CUSTOM" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for PlanTypeEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<PlanTypeEnum, Box<dyn std::error::Error + Sync + Send>> {
                match std::str::from_utf8(buf)? {
                    "STANDARD" => Ok(PlanTypeEnum::STANDARD),
                    "FREE" => Ok(PlanTypeEnum::FREE),
                    "CUSTOM" => Ok(PlanTypeEnum::CUSTOM),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "PlanTypeEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 3 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "STANDARD" => true,
                            "FREE" => true,
                            "CUSTOM" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum BillingPeriodEnum {
            MONTHLY,
            QUARTERLY,
            ANNUAL,
        }
        impl<'a> postgres_types::ToSql for BillingPeriodEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    BillingPeriodEnum::MONTHLY => "MONTHLY",
                    BillingPeriodEnum::QUARTERLY => "QUARTERLY",
                    BillingPeriodEnum::ANNUAL => "ANNUAL",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "BillingPeriodEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 3 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "MONTHLY" => true,
                            "QUARTERLY" => true,
                            "ANNUAL" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for BillingPeriodEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<BillingPeriodEnum, Box<dyn std::error::Error + Sync + Send>> {
                match std::str::from_utf8(buf)? {
                    "MONTHLY" => Ok(BillingPeriodEnum::MONTHLY),
                    "QUARTERLY" => Ok(BillingPeriodEnum::QUARTERLY),
                    "ANNUAL" => Ok(BillingPeriodEnum::ANNUAL),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "BillingPeriodEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 3 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "MONTHLY" => true,
                            "QUARTERLY" => true,
                            "ANNUAL" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum MrrMovementType {
            NEW_BUSINESS,
            EXPANSION,
            CONTRACTION,
            CHURN,
            REACTIVATION,
        }
        impl<'a> postgres_types::ToSql for MrrMovementType {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    MrrMovementType::NEW_BUSINESS => "NEW_BUSINESS",
                    MrrMovementType::EXPANSION => "EXPANSION",
                    MrrMovementType::CONTRACTION => "CONTRACTION",
                    MrrMovementType::CHURN => "CHURN",
                    MrrMovementType::REACTIVATION => "REACTIVATION",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "MRRMovementType" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 5 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "NEW_BUSINESS" => true,
                            "EXPANSION" => true,
                            "CONTRACTION" => true,
                            "CHURN" => true,
                            "REACTIVATION" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for MrrMovementType {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<MrrMovementType, Box<dyn std::error::Error + Sync + Send>> {
                match std::str::from_utf8(buf)? {
                    "NEW_BUSINESS" => Ok(MrrMovementType::NEW_BUSINESS),
                    "EXPANSION" => Ok(MrrMovementType::EXPANSION),
                    "CONTRACTION" => Ok(MrrMovementType::CONTRACTION),
                    "CHURN" => Ok(MrrMovementType::CHURN),
                    "REACTIVATION" => Ok(MrrMovementType::REACTIVATION),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "MRRMovementType" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 5 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "NEW_BUSINESS" => true,
                            "EXPANSION" => true,
                            "CONTRACTION" => true,
                            "CHURN" => true,
                            "REACTIVATION" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum SubscriptionStatusEnum {
            PENDING,
            ACTIVE,
            CANCELLED,
        }
        impl<'a> postgres_types::ToSql for SubscriptionStatusEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    SubscriptionStatusEnum::PENDING => "PENDING",
                    SubscriptionStatusEnum::ACTIVE => "ACTIVE",
                    SubscriptionStatusEnum::CANCELLED => "CANCELLED",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "SubscriptionStatusEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 3 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "PENDING" => true,
                            "ACTIVE" => true,
                            "CANCELLED" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for SubscriptionStatusEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<SubscriptionStatusEnum, Box<dyn std::error::Error + Sync + Send>>
            {
                match std::str::from_utf8(buf)? {
                    "PENDING" => Ok(SubscriptionStatusEnum::PENDING),
                    "ACTIVE" => Ok(SubscriptionStatusEnum::ACTIVE),
                    "CANCELLED" => Ok(SubscriptionStatusEnum::CANCELLED),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "SubscriptionStatusEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 3 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "PENDING" => true,
                            "ACTIVE" => true,
                            "CANCELLED" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum WebhookOutEventTypeEnum {
            CUSTOMER_CREATED,
            SUBSCRIPTION_CREATED,
            INVOICE_CREATED,
            INVOICE_FINALIZED,
        }
        impl<'a> postgres_types::ToSql for WebhookOutEventTypeEnum {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                buf: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                let s = match *self {
                    WebhookOutEventTypeEnum::CUSTOMER_CREATED => "CUSTOMER_CREATED",
                    WebhookOutEventTypeEnum::SUBSCRIPTION_CREATED => "SUBSCRIPTION_CREATED",
                    WebhookOutEventTypeEnum::INVOICE_CREATED => "INVOICE_CREATED",
                    WebhookOutEventTypeEnum::INVOICE_FINALIZED => "INVOICE_FINALIZED",
                };
                buf.extend_from_slice(s.as_bytes());
                std::result::Result::Ok(postgres_types::IsNull::No)
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "WebhookOutEventTypeEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 4 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "CUSTOMER_CREATED" => true,
                            "SUBSCRIPTION_CREATED" => true,
                            "INVOICE_CREATED" => true,
                            "INVOICE_FINALIZED" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            fn to_sql_checked(
                &self,
                ty: &postgres_types::Type,
                out: &mut postgres_types::private::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
            {
                postgres_types::__to_sql_checked(self, ty, out)
            }
        }
        impl<'a> postgres_types::FromSql<'a> for WebhookOutEventTypeEnum {
            fn from_sql(
                ty: &postgres_types::Type,
                buf: &'a [u8],
            ) -> Result<WebhookOutEventTypeEnum, Box<dyn std::error::Error + Sync + Send>>
            {
                match std::str::from_utf8(buf)? {
                    "CUSTOMER_CREATED" => Ok(WebhookOutEventTypeEnum::CUSTOMER_CREATED),
                    "SUBSCRIPTION_CREATED" => Ok(WebhookOutEventTypeEnum::SUBSCRIPTION_CREATED),
                    "INVOICE_CREATED" => Ok(WebhookOutEventTypeEnum::INVOICE_CREATED),
                    "INVOICE_FINALIZED" => Ok(WebhookOutEventTypeEnum::INVOICE_FINALIZED),
                    s => Result::Err(Into::into(format!("invalid variant `{}`", s))),
                }
            }
            fn accepts(ty: &postgres_types::Type) -> bool {
                if ty.name() != "WebhookOutEventTypeEnum" {
                    return false;
                }
                match *ty.kind() {
                    postgres_types::Kind::Enum(ref variants) => {
                        if variants.len() != 4 {
                            return false;
                        }
                        variants.iter().all(|v| match &**v {
                            "CUSTOMER_CREATED" => true,
                            "SUBSCRIPTION_CREATED" => true,
                            "INVOICE_CREATED" => true,
                            "INVOICE_FINALIZED" => true,
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
        }
    }
}
#[allow(clippy::all, clippy::pedantic)]
#[allow(unused_variables)]
#[allow(unused_imports)]
#[allow(dead_code)]
pub mod queries {
    pub mod api_tokens {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateApiTokenParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub hint: T2,
            pub hash: T3,
            pub tenant_id: uuid::Uuid,
            pub created_by: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ApiToken {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub name: String,
            pub hint: String,
            pub created_at: time::PrimitiveDateTime,
            pub created_by: uuid::Uuid,
        }
        pub struct ApiTokenBorrowed<'a> {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub name: &'a str,
            pub hint: &'a str,
            pub created_at: time::PrimitiveDateTime,
            pub created_by: uuid::Uuid,
        }
        impl<'a> From<ApiTokenBorrowed<'a>> for ApiToken {
            fn from(
                ApiTokenBorrowed {
                    id,
                    tenant_id,
                    name,
                    hint,
                    created_at,
                    created_by,
                }: ApiTokenBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    tenant_id,
                    name: name.into(),
                    hint: hint.into(),
                    created_at,
                    created_by,
                }
            }
        }
        pub struct ApiTokenQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ApiTokenBorrowed,
            mapper: fn(ApiTokenBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ApiTokenQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(ApiTokenBorrowed) -> R) -> ApiTokenQuery<'a, C, R, N> {
                ApiTokenQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct GetApiTokenById {
            pub hash: String,
            pub tenant_id: uuid::Uuid,
        }
        pub struct GetApiTokenByIdBorrowed<'a> {
            pub hash: &'a str,
            pub tenant_id: uuid::Uuid,
        }
        impl<'a> From<GetApiTokenByIdBorrowed<'a>> for GetApiTokenById {
            fn from(
                GetApiTokenByIdBorrowed { hash, tenant_id }: GetApiTokenByIdBorrowed<'a>,
            ) -> Self {
                Self {
                    hash: hash.into(),
                    tenant_id,
                }
            }
        }
        pub struct GetApiTokenByIdQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> GetApiTokenByIdBorrowed,
            mapper: fn(GetApiTokenByIdBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> GetApiTokenByIdQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(GetApiTokenByIdBorrowed) -> R,
            ) -> GetApiTokenByIdQuery<'a, C, R, N> {
                GetApiTokenByIdQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn list_api_tokens() -> ListApiTokensStmt {
            ListApiTokensStmt(cornucopia_async :: private :: Stmt :: new("SELECT id, tenant_id, name, hint, created_at, created_by FROM api_token WHERE tenant_id = $1"))
        }
        pub struct ListApiTokensStmt(cornucopia_async::private::Stmt);
        impl ListApiTokensStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> ApiTokenQuery<'a, C, ApiToken, 1> {
                ApiTokenQuery {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| ApiTokenBorrowed {
                        id: row.get(0),
                        tenant_id: row.get(1),
                        name: row.get(2),
                        hint: row.get(3),
                        created_at: row.get(4),
                        created_by: row.get(5),
                    },
                    mapper: |it| <ApiToken>::from(it),
                }
            }
        }
        pub fn create_api_token() -> CreateApiTokenStmt {
            CreateApiTokenStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO api_token (id, name, hint, hash, tenant_id, created_by) 
VALUES ($1, $2, $3, $4, $5, $6)
RETURNING id, tenant_id, name, hint, created_at, created_by",
            ))
        }
        pub struct CreateApiTokenStmt(cornucopia_async::private::Stmt);
        impl CreateApiTokenStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                hint: &'a T2,
                hash: &'a T3,
                tenant_id: &'a uuid::Uuid,
                created_by: &'a uuid::Uuid,
            ) -> ApiTokenQuery<'a, C, ApiToken, 6> {
                ApiTokenQuery {
                    client,
                    params: [id, name, hint, hash, tenant_id, created_by],
                    stmt: &mut self.0,
                    extractor: |row| ApiTokenBorrowed {
                        id: row.get(0),
                        tenant_id: row.get(1),
                        name: row.get(2),
                        hint: row.get(3),
                        created_at: row.get(4),
                        created_by: row.get(5),
                    },
                    mapper: |it| <ApiToken>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                CreateApiTokenParams<T1, T2, T3>,
                ApiTokenQuery<'a, C, ApiToken, 6>,
                C,
            > for CreateApiTokenStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateApiTokenParams<T1, T2, T3>,
            ) -> ApiTokenQuery<'a, C, ApiToken, 6> {
                self.bind(
                    client,
                    &params.id,
                    &params.name,
                    &params.hint,
                    &params.hash,
                    &params.tenant_id,
                    &params.created_by,
                )
            }
        }
        pub fn get_api_token_by_id() -> GetApiTokenByIdStmt {
            GetApiTokenByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT hash, tenant_id FROM api_token WHERE id = $1",
            ))
        }
        pub struct GetApiTokenByIdStmt(cornucopia_async::private::Stmt);
        impl GetApiTokenByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
            ) -> GetApiTokenByIdQuery<'a, C, GetApiTokenById, 1> {
                GetApiTokenByIdQuery {
                    client,
                    params: [id],
                    stmt: &mut self.0,
                    extractor: |row| GetApiTokenByIdBorrowed {
                        hash: row.get(0),
                        tenant_id: row.get(1),
                    },
                    mapper: |it| <GetApiTokenById>::from(it),
                }
            }
        }
    }
    pub mod billable_metrics {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateBillableMetricParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
            T4: cornucopia_async::StringSql,
            T5: cornucopia_async::JsonSql,
            T6: cornucopia_async::StringSql,
            T7: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub description: Option<T2>,
            pub code: T3,
            pub aggregation_type: super::super::types::public::BillingMetricAggregateEnum,
            pub aggregation_key: Option<T4>,
            pub unit_conversion_factor: Option<i32>,
            pub unit_conversion_rounding:
                Option<super::super::types::public::UnitConversionRoundingEnum>,
            pub segmentation_matrix: Option<T5>,
            pub usage_group_key: Option<T6>,
            pub tenant_id: uuid::Uuid,
            pub created_by: uuid::Uuid,
            pub product_family_external_id: T7,
        }
        #[derive(Debug)]
        pub struct ListBillableMetricsParams<T1: cornucopia_async::StringSql> {
            pub product_family_external_id: T1,
            pub tenant_id: uuid::Uuid,
            pub limit: i64,
            pub offset: i64,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetBillableMetricByIdParams {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct GetBillableMetricByIdsParams<T1: cornucopia_async::ArraySql<Item = uuid::Uuid>> {
            pub ids: T1,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct BillableMetric {
            pub id: uuid::Uuid,
            pub name: String,
            pub description: Option<String>,
            pub code: String,
            pub aggregation_type: super::super::types::public::BillingMetricAggregateEnum,
            pub aggregation_key: Option<String>,
            pub unit_conversion_factor: Option<i32>,
            pub unit_conversion_rounding:
                Option<super::super::types::public::UnitConversionRoundingEnum>,
            pub segmentation_matrix: Option<serde_json::Value>,
            pub usage_group_key: Option<String>,
            pub created_at: time::PrimitiveDateTime,
            pub created_by: uuid::Uuid,
            pub archived_at: Option<time::PrimitiveDateTime>,
        }
        pub struct BillableMetricBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub description: Option<&'a str>,
            pub code: &'a str,
            pub aggregation_type: super::super::types::public::BillingMetricAggregateEnum,
            pub aggregation_key: Option<&'a str>,
            pub unit_conversion_factor: Option<i32>,
            pub unit_conversion_rounding:
                Option<super::super::types::public::UnitConversionRoundingEnum>,
            pub segmentation_matrix: Option<postgres_types::Json<&'a serde_json::value::RawValue>>,
            pub usage_group_key: Option<&'a str>,
            pub created_at: time::PrimitiveDateTime,
            pub created_by: uuid::Uuid,
            pub archived_at: Option<time::PrimitiveDateTime>,
        }
        impl<'a> From<BillableMetricBorrowed<'a>> for BillableMetric {
            fn from(
                BillableMetricBorrowed {
                    id,
                    name,
                    description,
                    code,
                    aggregation_type,
                    aggregation_key,
                    unit_conversion_factor,
                    unit_conversion_rounding,
                    segmentation_matrix,
                    usage_group_key,
                    created_at,
                    created_by,
                    archived_at,
                }: BillableMetricBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    description: description.map(|v| v.into()),
                    code: code.into(),
                    aggregation_type,
                    aggregation_key: aggregation_key.map(|v| v.into()),
                    unit_conversion_factor,
                    unit_conversion_rounding,
                    segmentation_matrix: segmentation_matrix
                        .map(|v| serde_json::from_str(v.0.get()).unwrap()),
                    usage_group_key: usage_group_key.map(|v| v.into()),
                    created_at,
                    created_by,
                    archived_at,
                }
            }
        }
        pub struct BillableMetricQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> BillableMetricBorrowed,
            mapper: fn(BillableMetricBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> BillableMetricQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(BillableMetricBorrowed) -> R,
            ) -> BillableMetricQuery<'a, C, R, N> {
                BillableMetricQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ListBillableMetrics {
            pub id: uuid::Uuid,
            pub name: String,
            pub description: String,
            pub code: String,
            pub aggregation_type: super::super::types::public::BillingMetricAggregateEnum,
            pub aggregation_key: Option<String>,
            pub created_at: time::PrimitiveDateTime,
            pub created_by: uuid::Uuid,
            pub archived_at: Option<time::PrimitiveDateTime>,
            pub total_count: i64,
        }
        pub struct ListBillableMetricsBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub description: &'a str,
            pub code: &'a str,
            pub aggregation_type: super::super::types::public::BillingMetricAggregateEnum,
            pub aggregation_key: Option<&'a str>,
            pub created_at: time::PrimitiveDateTime,
            pub created_by: uuid::Uuid,
            pub archived_at: Option<time::PrimitiveDateTime>,
            pub total_count: i64,
        }
        impl<'a> From<ListBillableMetricsBorrowed<'a>> for ListBillableMetrics {
            fn from(
                ListBillableMetricsBorrowed {
                    id,
                    name,
                    description,
                    code,
                    aggregation_type,
                    aggregation_key,
                    created_at,
                    created_by,
                    archived_at,
                    total_count,
                }: ListBillableMetricsBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    description: description.into(),
                    code: code.into(),
                    aggregation_type,
                    aggregation_key: aggregation_key.map(|v| v.into()),
                    created_at,
                    created_by,
                    archived_at,
                    total_count,
                }
            }
        }
        pub struct ListBillableMetricsQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ListBillableMetricsBorrowed,
            mapper: fn(ListBillableMetricsBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ListBillableMetricsQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(ListBillableMetricsBorrowed) -> R,
            ) -> ListBillableMetricsQuery<'a, C, R, N> {
                ListBillableMetricsQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn create_billable_metric() -> CreateBillableMetricStmt {
            CreateBillableMetricStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO billable_metric (id,
                             name,
                             description,
                             code,
                             aggregation_type,
                             aggregation_key,
                             unit_conversion_factor,
                             unit_conversion_rounding,
                             segmentation_matrix,
                             usage_group_key,
                             tenant_id,
                             created_by,
                             product_family_id)
VALUES ($1,
        $2,
        $3,
        $4,
        $5,
        $6,
        $7,
        $8,
        $9,
        $10,
        $11,
        $12,
        (SELECT id
         FROM product_family
         WHERE external_id = $13
           AND tenant_id = $11))
RETURNING id,
    name,
    description,
    code,
    aggregation_type,
    aggregation_key,
    unit_conversion_factor,
    unit_conversion_rounding,
    segmentation_matrix,
    usage_group_key,
    created_at,
    created_by,
    archived_at",
            ))
        }
        pub struct CreateBillableMetricStmt(cornucopia_async::private::Stmt);
        impl CreateBillableMetricStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::StringSql,
                T5: cornucopia_async::JsonSql,
                T6: cornucopia_async::StringSql,
                T7: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                description: &'a Option<T2>,
                code: &'a T3,
                aggregation_type: &'a super::super::types::public::BillingMetricAggregateEnum,
                aggregation_key: &'a Option<T4>,
                unit_conversion_factor: &'a Option<i32>,
                unit_conversion_rounding: &'a Option<
                    super::super::types::public::UnitConversionRoundingEnum,
                >,
                segmentation_matrix: &'a Option<T5>,
                usage_group_key: &'a Option<T6>,
                tenant_id: &'a uuid::Uuid,
                created_by: &'a uuid::Uuid,
                product_family_external_id: &'a T7,
            ) -> BillableMetricQuery<'a, C, BillableMetric, 13> {
                BillableMetricQuery {
                    client,
                    params: [
                        id,
                        name,
                        description,
                        code,
                        aggregation_type,
                        aggregation_key,
                        unit_conversion_factor,
                        unit_conversion_rounding,
                        segmentation_matrix,
                        usage_group_key,
                        tenant_id,
                        created_by,
                        product_family_external_id,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| BillableMetricBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        description: row.get(2),
                        code: row.get(3),
                        aggregation_type: row.get(4),
                        aggregation_key: row.get(5),
                        unit_conversion_factor: row.get(6),
                        unit_conversion_rounding: row.get(7),
                        segmentation_matrix: row.get(8),
                        usage_group_key: row.get(9),
                        created_at: row.get(10),
                        created_by: row.get(11),
                        archived_at: row.get(12),
                    },
                    mapper: |it| <BillableMetric>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::StringSql,
                T5: cornucopia_async::JsonSql,
                T6: cornucopia_async::StringSql,
                T7: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                CreateBillableMetricParams<T1, T2, T3, T4, T5, T6, T7>,
                BillableMetricQuery<'a, C, BillableMetric, 13>,
                C,
            > for CreateBillableMetricStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateBillableMetricParams<T1, T2, T3, T4, T5, T6, T7>,
            ) -> BillableMetricQuery<'a, C, BillableMetric, 13> {
                self.bind(
                    client,
                    &params.id,
                    &params.name,
                    &params.description,
                    &params.code,
                    &params.aggregation_type,
                    &params.aggregation_key,
                    &params.unit_conversion_factor,
                    &params.unit_conversion_rounding,
                    &params.segmentation_matrix,
                    &params.usage_group_key,
                    &params.tenant_id,
                    &params.created_by,
                    &params.product_family_external_id,
                )
            }
        }
        pub fn list_billable_metrics() -> ListBillableMetricsStmt {
            ListBillableMetricsStmt(cornucopia_async::private::Stmt::new(
                "SELECT bm.id,
       bm.name,
       bm.description,
       bm.code,
       bm.aggregation_type,
       bm.aggregation_key,
       bm.created_at,
       bm.created_by,
       bm.archived_at,
       COUNT(*) OVER () AS total_count
FROM billable_metric AS bm
         JOIN product_family AS pf ON bm.product_family_id = pf.id
WHERE pf.external_id = $1
  AND bm.tenant_id = $2
ORDER BY bm.created_at ASC
LIMIT $3 OFFSET $4",
            ))
        }
        pub struct ListBillableMetricsStmt(cornucopia_async::private::Stmt);
        impl ListBillableMetricsStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                product_family_external_id: &'a T1,
                tenant_id: &'a uuid::Uuid,
                limit: &'a i64,
                offset: &'a i64,
            ) -> ListBillableMetricsQuery<'a, C, ListBillableMetrics, 4> {
                ListBillableMetricsQuery {
                    client,
                    params: [product_family_external_id, tenant_id, limit, offset],
                    stmt: &mut self.0,
                    extractor: |row| ListBillableMetricsBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        description: row.get(2),
                        code: row.get(3),
                        aggregation_type: row.get(4),
                        aggregation_key: row.get(5),
                        created_at: row.get(6),
                        created_by: row.get(7),
                        archived_at: row.get(8),
                        total_count: row.get(9),
                    },
                    mapper: |it| <ListBillableMetrics>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                ListBillableMetricsParams<T1>,
                ListBillableMetricsQuery<'a, C, ListBillableMetrics, 4>,
                C,
            > for ListBillableMetricsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListBillableMetricsParams<T1>,
            ) -> ListBillableMetricsQuery<'a, C, ListBillableMetrics, 4> {
                self.bind(
                    client,
                    &params.product_family_external_id,
                    &params.tenant_id,
                    &params.limit,
                    &params.offset,
                )
            }
        }
        pub fn get_billable_metric_by_id() -> GetBillableMetricByIdStmt {
            GetBillableMetricByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT bm.id,
       bm.name,
       bm.description,
       bm.code,
       bm.aggregation_type,
       bm.aggregation_key,
       bm.unit_conversion_factor,
       bm.unit_conversion_rounding,
       bm.segmentation_matrix,
       bm.usage_group_key,
       bm.created_at,
       bm.created_by,
       bm.archived_at
FROM billable_metric AS bm
WHERE bm.id = $1
  AND bm.tenant_id = $2",
            ))
        }
        pub struct GetBillableMetricByIdStmt(cornucopia_async::private::Stmt);
        impl GetBillableMetricByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> BillableMetricQuery<'a, C, BillableMetric, 2> {
                BillableMetricQuery {
                    client,
                    params: [id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| BillableMetricBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        description: row.get(2),
                        code: row.get(3),
                        aggregation_type: row.get(4),
                        aggregation_key: row.get(5),
                        unit_conversion_factor: row.get(6),
                        unit_conversion_rounding: row.get(7),
                        segmentation_matrix: row.get(8),
                        usage_group_key: row.get(9),
                        created_at: row.get(10),
                        created_by: row.get(11),
                        archived_at: row.get(12),
                    },
                    mapper: |it| <BillableMetric>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetBillableMetricByIdParams,
                BillableMetricQuery<'a, C, BillableMetric, 2>,
                C,
            > for GetBillableMetricByIdStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetBillableMetricByIdParams,
            ) -> BillableMetricQuery<'a, C, BillableMetric, 2> {
                self.bind(client, &params.id, &params.tenant_id)
            }
        }
        pub fn get_billable_metric_by_ids() -> GetBillableMetricByIdsStmt {
            GetBillableMetricByIdsStmt(cornucopia_async::private::Stmt::new(
                "SELECT bm.id,
       bm.name,
       bm.description,
       bm.code,
       bm.aggregation_type,
       bm.aggregation_key,
       bm.unit_conversion_factor,
       bm.unit_conversion_rounding,
       bm.segmentation_matrix,
       bm.usage_group_key,
       bm.created_at,
       bm.created_by,
       bm.archived_at
FROM billable_metric AS bm
WHERE bm.id = ANY ($1)
  AND bm.tenant_id = $2",
            ))
        }
        pub struct GetBillableMetricByIdsStmt(cornucopia_async::private::Stmt);
        impl GetBillableMetricByIdsStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::ArraySql<Item = uuid::Uuid>>(
                &'a mut self,
                client: &'a C,
                ids: &'a T1,
                tenant_id: &'a uuid::Uuid,
            ) -> BillableMetricQuery<'a, C, BillableMetric, 2> {
                BillableMetricQuery {
                    client,
                    params: [ids, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| BillableMetricBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        description: row.get(2),
                        code: row.get(3),
                        aggregation_type: row.get(4),
                        aggregation_key: row.get(5),
                        unit_conversion_factor: row.get(6),
                        unit_conversion_rounding: row.get(7),
                        segmentation_matrix: row.get(8),
                        usage_group_key: row.get(9),
                        created_at: row.get(10),
                        created_by: row.get(11),
                        archived_at: row.get(12),
                    },
                    mapper: |it| <BillableMetric>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::ArraySql<Item = uuid::Uuid>>
            cornucopia_async::Params<
                'a,
                GetBillableMetricByIdsParams<T1>,
                BillableMetricQuery<'a, C, BillableMetric, 2>,
                C,
            > for GetBillableMetricByIdsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetBillableMetricByIdsParams<T1>,
            ) -> BillableMetricQuery<'a, C, BillableMetric, 2> {
                self.bind(client, &params.ids, &params.tenant_id)
            }
        }
    }
    pub mod customers {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateCustomerParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
            T4: cornucopia_async::JsonSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub alias: Option<T2>,
            pub email: Option<T3>,
            pub tenant_id: uuid::Uuid,
            pub created_by: uuid::Uuid,
            pub billing_config: T4,
        }
        #[derive(Debug)]
        pub struct ListCustomersParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
        > {
            pub tenant_id: uuid::Uuid,
            pub search: Option<T1>,
            pub order_by: T2,
            pub limit: i64,
            pub offset: i64,
        }
        #[derive(Debug)]
        pub struct CountCustomersParams<T1: cornucopia_async::StringSql> {
            pub tenant_id: uuid::Uuid,
            pub search: Option<T1>,
        }
        #[derive(Debug)]
        pub struct GetCustomerByAliasParams<T1: cornucopia_async::StringSql> {
            pub tenant_id: uuid::Uuid,
            pub alias: T1,
        }
        #[derive(Debug)]
        pub struct GetCustomerIdsByAliasParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::ArraySql<Item = T1>,
        > {
            pub tenant_id: uuid::Uuid,
            pub aliases: T2,
        }
        #[derive(Debug)]
        pub struct PatchCustomerParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
            T4: cornucopia_async::StringSql,
            T5: cornucopia_async::StringSql,
            T6: cornucopia_async::StringSql,
            T7: cornucopia_async::JsonSql,
            T8: cornucopia_async::JsonSql,
        > {
            pub name: T1,
            pub alias: Option<T2>,
            pub email: Option<T3>,
            pub invoicing_email: Option<T4>,
            pub phone: Option<T5>,
            pub balance_value_cents: i32,
            pub balance_currency: T6,
            pub billing_address: Option<T7>,
            pub shipping_address: Option<T8>,
            pub id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct CreateCustomer {
            pub id: uuid::Uuid,
            pub name: String,
            pub email: Option<String>,
            pub alias: Option<String>,
        }
        pub struct CreateCustomerBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub email: Option<&'a str>,
            pub alias: Option<&'a str>,
        }
        impl<'a> From<CreateCustomerBorrowed<'a>> for CreateCustomer {
            fn from(
                CreateCustomerBorrowed {
                    id,
                    name,
                    email,
                    alias,
                }: CreateCustomerBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    email: email.map(|v| v.into()),
                    alias: alias.map(|v| v.into()),
                }
            }
        }
        pub struct CreateCustomerQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> CreateCustomerBorrowed,
            mapper: fn(CreateCustomerBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> CreateCustomerQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(CreateCustomerBorrowed) -> R,
            ) -> CreateCustomerQuery<'a, C, R, N> {
                CreateCustomerQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct CustomerList {
            pub id: uuid::Uuid,
            pub name: String,
            pub email: Option<String>,
            pub alias: Option<String>,
            pub total_count: i64,
        }
        pub struct CustomerListBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub email: Option<&'a str>,
            pub alias: Option<&'a str>,
            pub total_count: i64,
        }
        impl<'a> From<CustomerListBorrowed<'a>> for CustomerList {
            fn from(
                CustomerListBorrowed {
                    id,
                    name,
                    email,
                    alias,
                    total_count,
                }: CustomerListBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    email: email.map(|v| v.into()),
                    alias: alias.map(|v| v.into()),
                    total_count,
                }
            }
        }
        pub struct CustomerListQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> CustomerListBorrowed,
            mapper: fn(CustomerListBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> CustomerListQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(CustomerListBorrowed) -> R,
            ) -> CustomerListQuery<'a, C, R, N> {
                CustomerListQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub struct I64Query<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> i64,
            mapper: fn(i64) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> I64Query<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(i64) -> R) -> I64Query<'a, C, R, N> {
                I64Query {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct Customer {
            pub id: uuid::Uuid,
            pub name: String,
            pub alias: Option<String>,
            pub billing_config: Option<serde_json::Value>,
            pub email: Option<String>,
            pub invoicing_email: Option<String>,
            pub phone: Option<String>,
            pub balance_value_cents: i32,
            pub balance_currency: String,
            pub archived_at: Option<time::PrimitiveDateTime>,
            pub created_at: Option<time::PrimitiveDateTime>,
            pub billing_address: Option<serde_json::Value>,
            pub shipping_address: Option<serde_json::Value>,
        }
        pub struct CustomerBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub alias: Option<&'a str>,
            pub billing_config: Option<postgres_types::Json<&'a serde_json::value::RawValue>>,
            pub email: Option<&'a str>,
            pub invoicing_email: Option<&'a str>,
            pub phone: Option<&'a str>,
            pub balance_value_cents: i32,
            pub balance_currency: &'a str,
            pub archived_at: Option<time::PrimitiveDateTime>,
            pub created_at: Option<time::PrimitiveDateTime>,
            pub billing_address: Option<postgres_types::Json<&'a serde_json::value::RawValue>>,
            pub shipping_address: Option<postgres_types::Json<&'a serde_json::value::RawValue>>,
        }
        impl<'a> From<CustomerBorrowed<'a>> for Customer {
            fn from(
                CustomerBorrowed {
                    id,
                    name,
                    alias,
                    billing_config,
                    email,
                    invoicing_email,
                    phone,
                    balance_value_cents,
                    balance_currency,
                    archived_at,
                    created_at,
                    billing_address,
                    shipping_address,
                }: CustomerBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    alias: alias.map(|v| v.into()),
                    billing_config: billing_config
                        .map(|v| serde_json::from_str(v.0.get()).unwrap()),
                    email: email.map(|v| v.into()),
                    invoicing_email: invoicing_email.map(|v| v.into()),
                    phone: phone.map(|v| v.into()),
                    balance_value_cents,
                    balance_currency: balance_currency.into(),
                    archived_at,
                    created_at,
                    billing_address: billing_address
                        .map(|v| serde_json::from_str(v.0.get()).unwrap()),
                    shipping_address: shipping_address
                        .map(|v| serde_json::from_str(v.0.get()).unwrap()),
                }
            }
        }
        pub struct CustomerQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> CustomerBorrowed,
            mapper: fn(CustomerBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> CustomerQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(CustomerBorrowed) -> R) -> CustomerQuery<'a, C, R, N> {
                CustomerQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct GetCustomerIdsByAlias {
            pub id: uuid::Uuid,
            pub alias: String,
        }
        pub struct GetCustomerIdsByAliasBorrowed<'a> {
            pub id: uuid::Uuid,
            pub alias: &'a str,
        }
        impl<'a> From<GetCustomerIdsByAliasBorrowed<'a>> for GetCustomerIdsByAlias {
            fn from(
                GetCustomerIdsByAliasBorrowed { id, alias }: GetCustomerIdsByAliasBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    alias: alias.into(),
                }
            }
        }
        pub struct GetCustomerIdsByAliasQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> GetCustomerIdsByAliasBorrowed,
            mapper: fn(GetCustomerIdsByAliasBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> GetCustomerIdsByAliasQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(GetCustomerIdsByAliasBorrowed) -> R,
            ) -> GetCustomerIdsByAliasQuery<'a, C, R, N> {
                GetCustomerIdsByAliasQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn create_customer() -> CreateCustomerStmt {
            CreateCustomerStmt(cornucopia_async :: private :: Stmt :: new("INSERT INTO customer (id, name, alias, email, tenant_id, created_by, billing_config)
VALUES ($1,
        $2,
        $3,
        $4,
        $5,
        $6,
        $7)
RETURNING id, name, email, alias"))
        }
        pub struct CreateCustomerStmt(cornucopia_async::private::Stmt);
        impl CreateCustomerStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::JsonSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                alias: &'a Option<T2>,
                email: &'a Option<T3>,
                tenant_id: &'a uuid::Uuid,
                created_by: &'a uuid::Uuid,
                billing_config: &'a T4,
            ) -> CreateCustomerQuery<'a, C, CreateCustomer, 7> {
                CreateCustomerQuery {
                    client,
                    params: [
                        id,
                        name,
                        alias,
                        email,
                        tenant_id,
                        created_by,
                        billing_config,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| CreateCustomerBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        email: row.get(2),
                        alias: row.get(3),
                    },
                    mapper: |it| <CreateCustomer>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::JsonSql,
            >
            cornucopia_async::Params<
                'a,
                CreateCustomerParams<T1, T2, T3, T4>,
                CreateCustomerQuery<'a, C, CreateCustomer, 7>,
                C,
            > for CreateCustomerStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateCustomerParams<T1, T2, T3, T4>,
            ) -> CreateCustomerQuery<'a, C, CreateCustomer, 7> {
                self.bind(
                    client,
                    &params.id,
                    &params.name,
                    &params.alias,
                    &params.email,
                    &params.tenant_id,
                    &params.created_by,
                    &params.billing_config,
                )
            }
        }
        pub fn list_customers() -> ListCustomersStmt {
            ListCustomersStmt(cornucopia_async::private::Stmt::new(
                "SELECT id,
       name,
       email,
       alias,
       COUNT(*) OVER () AS total_count
FROM customer
WHERE tenant_id = $1
  AND (
    $2 :: TEXT IS NULL
        OR name ILIKE '%' || $2 || '%'
        OR alias ILIKE '%' || $2 || '%'
    )
ORDER BY CASE
             WHEN $3 = 'DATE_DESC' THEN id
             END DESC,
         CASE
             WHEN $3 = 'DATE_ASC' THEN id
             END ASC,
         CASE
             WHEN $3 = 'NAME_DESC' THEN name
             END DESC,
         CASE
             WHEN $3 = 'NAME_ASC' THEN name
             END ASC
LIMIT $4 OFFSET $5",
            ))
        }
        pub struct ListCustomersStmt(cornucopia_async::private::Stmt);
        impl ListCustomersStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                search: &'a Option<T1>,
                order_by: &'a T2,
                limit: &'a i64,
                offset: &'a i64,
            ) -> CustomerListQuery<'a, C, CustomerList, 5> {
                CustomerListQuery {
                    client,
                    params: [tenant_id, search, order_by, limit, offset],
                    stmt: &mut self.0,
                    extractor: |row| CustomerListBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        email: row.get(2),
                        alias: row.get(3),
                        total_count: row.get(4),
                    },
                    mapper: |it| <CustomerList>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                ListCustomersParams<T1, T2>,
                CustomerListQuery<'a, C, CustomerList, 5>,
                C,
            > for ListCustomersStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListCustomersParams<T1, T2>,
            ) -> CustomerListQuery<'a, C, CustomerList, 5> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.search,
                    &params.order_by,
                    &params.limit,
                    &params.offset,
                )
            }
        }
        pub fn count_customers() -> CountCustomersStmt {
            CountCustomersStmt(cornucopia_async::private::Stmt::new(
                "SELECT COUNT(*) AS total_customers
FROM customer
WHERE tenant_id = $1
  AND (
    $2 :: TEXT IS NULL
        OR to_tsvector('english', name || ' ' || alias) @@ to_tsquery('english', $2)
    )",
            ))
        }
        pub struct CountCustomersStmt(cornucopia_async::private::Stmt);
        impl CountCustomersStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                search: &'a Option<T1>,
            ) -> I64Query<'a, C, i64, 2> {
                I64Query {
                    client,
                    params: [tenant_id, search],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<'a, CountCustomersParams<T1>, I64Query<'a, C, i64, 2>, C>
            for CountCustomersStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CountCustomersParams<T1>,
            ) -> I64Query<'a, C, i64, 2> {
                self.bind(client, &params.tenant_id, &params.search)
            }
        }
        pub fn get_customer_by_id() -> GetCustomerByIdStmt {
            GetCustomerByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT id,
       name,
       alias,
       billing_config,
       email,
       invoicing_email,
       phone,
       balance_value_cents,
       balance_currency,
       archived_at,
       created_at,
       billing_address,
       shipping_address
FROM customer
WHERE id = $1",
            ))
        }
        pub struct GetCustomerByIdStmt(cornucopia_async::private::Stmt);
        impl GetCustomerByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
            ) -> CustomerQuery<'a, C, Customer, 1> {
                CustomerQuery {
                    client,
                    params: [id],
                    stmt: &mut self.0,
                    extractor: |row| CustomerBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        alias: row.get(2),
                        billing_config: row.get(3),
                        email: row.get(4),
                        invoicing_email: row.get(5),
                        phone: row.get(6),
                        balance_value_cents: row.get(7),
                        balance_currency: row.get(8),
                        archived_at: row.get(9),
                        created_at: row.get(10),
                        billing_address: row.get(11),
                        shipping_address: row.get(12),
                    },
                    mapper: |it| <Customer>::from(it),
                }
            }
        }
        pub fn get_customer_by_alias() -> GetCustomerByAliasStmt {
            GetCustomerByAliasStmt(cornucopia_async::private::Stmt::new(
                "SELECT id,
       name,
       alias,
       billing_config,
       email,
       invoicing_email,
       phone,
       balance_value_cents,
       balance_currency,
       archived_at,
       created_at,
       billing_address,
       shipping_address
FROM customer
WHERE tenant_id = $1
  AND alias = $2",
            ))
        }
        pub struct GetCustomerByAliasStmt(cornucopia_async::private::Stmt);
        impl GetCustomerByAliasStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                alias: &'a T1,
            ) -> CustomerQuery<'a, C, Customer, 2> {
                CustomerQuery {
                    client,
                    params: [tenant_id, alias],
                    stmt: &mut self.0,
                    extractor: |row| CustomerBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        alias: row.get(2),
                        billing_config: row.get(3),
                        email: row.get(4),
                        invoicing_email: row.get(5),
                        phone: row.get(6),
                        balance_value_cents: row.get(7),
                        balance_currency: row.get(8),
                        archived_at: row.get(9),
                        created_at: row.get(10),
                        billing_address: row.get(11),
                        shipping_address: row.get(12),
                    },
                    mapper: |it| <Customer>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                GetCustomerByAliasParams<T1>,
                CustomerQuery<'a, C, Customer, 2>,
                C,
            > for GetCustomerByAliasStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetCustomerByAliasParams<T1>,
            ) -> CustomerQuery<'a, C, Customer, 2> {
                self.bind(client, &params.tenant_id, &params.alias)
            }
        }
        pub fn get_customer_ids_by_alias() -> GetCustomerIdsByAliasStmt {
            GetCustomerIdsByAliasStmt(cornucopia_async::private::Stmt::new(
                "SELECT id,
       alias
FROM customer
WHERE tenant_id = $1
  AND alias = ANY ($2)",
            ))
        }
        pub struct GetCustomerIdsByAliasStmt(cornucopia_async::private::Stmt);
        impl GetCustomerIdsByAliasStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::ArraySql<Item = T1>,
            >(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                aliases: &'a T2,
            ) -> GetCustomerIdsByAliasQuery<'a, C, GetCustomerIdsByAlias, 2> {
                GetCustomerIdsByAliasQuery {
                    client,
                    params: [tenant_id, aliases],
                    stmt: &mut self.0,
                    extractor: |row| GetCustomerIdsByAliasBorrowed {
                        id: row.get(0),
                        alias: row.get(1),
                    },
                    mapper: |it| <GetCustomerIdsByAlias>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::ArraySql<Item = T1>,
            >
            cornucopia_async::Params<
                'a,
                GetCustomerIdsByAliasParams<T1, T2>,
                GetCustomerIdsByAliasQuery<'a, C, GetCustomerIdsByAlias, 2>,
                C,
            > for GetCustomerIdsByAliasStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetCustomerIdsByAliasParams<T1, T2>,
            ) -> GetCustomerIdsByAliasQuery<'a, C, GetCustomerIdsByAlias, 2> {
                self.bind(client, &params.tenant_id, &params.aliases)
            }
        }
        pub fn patch_customer() -> PatchCustomerStmt {
            PatchCustomerStmt(cornucopia_async::private::Stmt::new(
                "UPDATE customer
SET
    name = $1,
    alias = $2,
    email = $3,
    invoicing_email = $4,
    phone = $5,
    balance_value_cents = $6,
    balance_currency = $7,
    billing_address = $8,
    shipping_address = $9
WHERE id = $10",
            ))
        }
        pub struct PatchCustomerStmt(cornucopia_async::private::Stmt);
        impl PatchCustomerStmt {
            pub async fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::StringSql,
                T5: cornucopia_async::StringSql,
                T6: cornucopia_async::StringSql,
                T7: cornucopia_async::JsonSql,
                T8: cornucopia_async::JsonSql,
            >(
                &'a mut self,
                client: &'a C,
                name: &'a T1,
                alias: &'a Option<T2>,
                email: &'a Option<T3>,
                invoicing_email: &'a Option<T4>,
                phone: &'a Option<T5>,
                balance_value_cents: &'a i32,
                balance_currency: &'a T6,
                billing_address: &'a Option<T7>,
                shipping_address: &'a Option<T8>,
                id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client
                    .execute(
                        stmt,
                        &[
                            name,
                            alias,
                            email,
                            invoicing_email,
                            phone,
                            balance_value_cents,
                            balance_currency,
                            billing_address,
                            shipping_address,
                            id,
                        ],
                    )
                    .await
            }
        }
        impl<
                'a,
                C: GenericClient + Send + Sync,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::StringSql,
                T5: cornucopia_async::StringSql,
                T6: cornucopia_async::StringSql,
                T7: cornucopia_async::JsonSql,
                T8: cornucopia_async::JsonSql,
            >
            cornucopia_async::Params<
                'a,
                PatchCustomerParams<T1, T2, T3, T4, T5, T6, T7, T8>,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for PatchCustomerStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a PatchCustomerParams<T1, T2, T3, T4, T5, T6, T7, T8>,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(
                    client,
                    &params.name,
                    &params.alias,
                    &params.email,
                    &params.invoicing_email,
                    &params.phone,
                    &params.balance_value_cents,
                    &params.balance_currency,
                    &params.billing_address,
                    &params.shipping_address,
                    &params.id,
                ))
            }
        }
    }
    pub mod invoices {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateInvoiceParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::JsonSql,
        > {
            pub id: uuid::Uuid,
            pub status: super::super::types::public::InvoiceStatusEnum,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub invoice_date: time::Date,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub subscription_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub currency: T1,
            pub days_until_due: i32,
            pub line_items: T2,
            pub amount_cents: Option<i64>,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct UpdateInvoiceStatusParams {
            pub status: super::super::types::public::InvoiceStatusEnum,
            pub id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct UpdateInvoiceExternalStatusParams {
            pub external_status: super::super::types::public::InvoiceExternalStatusEnum,
            pub id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct UpdateInvoiceLinesParams<T1: cornucopia_async::JsonSql> {
            pub line_items: T1,
            pub id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct PatchInvoiceParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::JsonSql,
        > {
            pub status: Option<super::super::types::public::InvoiceStatusEnum>,
            pub invoicing_provider: Option<super::super::types::public::InvoicingProviderEnum>,
            pub invoice_date: Option<time::Date>,
            pub currency: Option<T1>,
            pub days_until_due: i32,
            pub line_items: Option<T2>,
            pub amount_cents: i64,
            pub id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct ListTenantInvoicesParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
        > {
            pub tenant_id: uuid::Uuid,
            pub status: Option<super::super::types::public::InvoiceStatusEnum>,
            pub search: Option<T1>,
            pub customer_id: Option<uuid::Uuid>,
            pub order_by: T2,
            pub limit: i64,
            pub offset: i64,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct UpdateInvoiceIssueSuccessParams {
            pub issue_attempts: i32,
            pub id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct UpdateInvoiceIssueErrorParams<T1: cornucopia_async::StringSql> {
            pub issue_attempts: i32,
            pub last_issue_error: T1,
            pub id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetTenantInvoiceByIdParams {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct Invoice {
            pub id: uuid::Uuid,
            pub status: super::super::types::public::InvoiceStatusEnum,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub invoice_date: time::Date,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub subscription_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub currency: String,
            pub days_until_due: Option<i32>,
            pub line_items: serde_json::Value,
            pub amount_cents: Option<i64>,
            pub issued: bool,
            pub issue_attempts: i32,
            pub last_issue_attempt_at: Option<time::OffsetDateTime>,
            pub last_issue_error: Option<String>,
        }
        pub struct InvoiceBorrowed<'a> {
            pub id: uuid::Uuid,
            pub status: super::super::types::public::InvoiceStatusEnum,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub invoice_date: time::Date,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub subscription_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub currency: &'a str,
            pub days_until_due: Option<i32>,
            pub line_items: postgres_types::Json<&'a serde_json::value::RawValue>,
            pub amount_cents: Option<i64>,
            pub issued: bool,
            pub issue_attempts: i32,
            pub last_issue_attempt_at: Option<time::OffsetDateTime>,
            pub last_issue_error: Option<&'a str>,
        }
        impl<'a> From<InvoiceBorrowed<'a>> for Invoice {
            fn from(
                InvoiceBorrowed {
                    id,
                    status,
                    invoicing_provider,
                    invoice_date,
                    tenant_id,
                    customer_id,
                    subscription_id,
                    plan_version_id,
                    currency,
                    days_until_due,
                    line_items,
                    amount_cents,
                    issued,
                    issue_attempts,
                    last_issue_attempt_at,
                    last_issue_error,
                }: InvoiceBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    status,
                    invoicing_provider,
                    invoice_date,
                    tenant_id,
                    customer_id,
                    subscription_id,
                    plan_version_id,
                    currency: currency.into(),
                    days_until_due,
                    line_items: serde_json::from_str(line_items.0.get()).unwrap(),
                    amount_cents,
                    issued,
                    issue_attempts,
                    last_issue_attempt_at,
                    last_issue_error: last_issue_error.map(|v| v.into()),
                }
            }
        }
        pub struct InvoiceQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> InvoiceBorrowed,
            mapper: fn(InvoiceBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> InvoiceQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(InvoiceBorrowed) -> R) -> InvoiceQuery<'a, C, R, N> {
                InvoiceQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct UpdateInvoiceExternalStatus {
            pub id: uuid::Uuid,
            pub external_status: super::super::types::public::InvoiceExternalStatusEnum,
        }
        pub struct UpdateInvoiceExternalStatusQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> UpdateInvoiceExternalStatus,
            mapper: fn(UpdateInvoiceExternalStatus) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> UpdateInvoiceExternalStatusQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(UpdateInvoiceExternalStatus) -> R,
            ) -> UpdateInvoiceExternalStatusQuery<'a, C, R, N> {
                UpdateInvoiceExternalStatusQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct UpdatePendingFinalizationInvoices {
            pub id: uuid::Uuid,
            pub status: super::super::types::public::InvoiceStatusEnum,
        }
        pub struct UpdatePendingFinalizationInvoicesQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> UpdatePendingFinalizationInvoices,
            mapper: fn(UpdatePendingFinalizationInvoices) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> UpdatePendingFinalizationInvoicesQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(UpdatePendingFinalizationInvoices) -> R,
            ) -> UpdatePendingFinalizationInvoicesQuery<'a, C, R, N> {
                UpdatePendingFinalizationInvoicesQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ListInvoice {
            pub id: uuid::Uuid,
            pub status: super::super::types::public::InvoiceStatusEnum,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub created_at: time::OffsetDateTime,
            pub invoice_date: time::Date,
            pub customer_id: uuid::Uuid,
            pub subscription_id: uuid::Uuid,
            pub currency: String,
            pub days_until_due: Option<i32>,
            pub amount_cents: Option<i64>,
            pub customer_name: String,
            pub total_count: i64,
        }
        pub struct ListInvoiceBorrowed<'a> {
            pub id: uuid::Uuid,
            pub status: super::super::types::public::InvoiceStatusEnum,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub created_at: time::OffsetDateTime,
            pub invoice_date: time::Date,
            pub customer_id: uuid::Uuid,
            pub subscription_id: uuid::Uuid,
            pub currency: &'a str,
            pub days_until_due: Option<i32>,
            pub amount_cents: Option<i64>,
            pub customer_name: &'a str,
            pub total_count: i64,
        }
        impl<'a> From<ListInvoiceBorrowed<'a>> for ListInvoice {
            fn from(
                ListInvoiceBorrowed {
                    id,
                    status,
                    invoicing_provider,
                    created_at,
                    invoice_date,
                    customer_id,
                    subscription_id,
                    currency,
                    days_until_due,
                    amount_cents,
                    customer_name,
                    total_count,
                }: ListInvoiceBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    status,
                    invoicing_provider,
                    created_at,
                    invoice_date,
                    customer_id,
                    subscription_id,
                    currency: currency.into(),
                    days_until_due,
                    amount_cents,
                    customer_name: customer_name.into(),
                    total_count,
                }
            }
        }
        pub struct ListInvoiceQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ListInvoiceBorrowed,
            mapper: fn(ListInvoiceBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ListInvoiceQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(ListInvoiceBorrowed) -> R,
            ) -> ListInvoiceQuery<'a, C, R, N> {
                ListInvoiceQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct DetailedInvoice {
            pub id: uuid::Uuid,
            pub status: super::super::types::public::InvoiceStatusEnum,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub created_at: time::OffsetDateTime,
            pub updated_at: Option<time::OffsetDateTime>,
            pub invoice_date: time::Date,
            pub customer_id: uuid::Uuid,
            pub subscription_id: uuid::Uuid,
            pub currency: String,
            pub days_until_due: Option<i32>,
            pub issued: bool,
            pub issue_attempts: i32,
            pub last_issue_attempt_at: Option<time::OffsetDateTime>,
            pub last_issue_error: Option<String>,
            pub amount_cents: Option<i64>,
            pub customer_name: String,
            pub plan_name: String,
            pub plan_external_id: String,
            pub plan_version: i32,
        }
        pub struct DetailedInvoiceBorrowed<'a> {
            pub id: uuid::Uuid,
            pub status: super::super::types::public::InvoiceStatusEnum,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub created_at: time::OffsetDateTime,
            pub updated_at: Option<time::OffsetDateTime>,
            pub invoice_date: time::Date,
            pub customer_id: uuid::Uuid,
            pub subscription_id: uuid::Uuid,
            pub currency: &'a str,
            pub days_until_due: Option<i32>,
            pub issued: bool,
            pub issue_attempts: i32,
            pub last_issue_attempt_at: Option<time::OffsetDateTime>,
            pub last_issue_error: Option<&'a str>,
            pub amount_cents: Option<i64>,
            pub customer_name: &'a str,
            pub plan_name: &'a str,
            pub plan_external_id: &'a str,
            pub plan_version: i32,
        }
        impl<'a> From<DetailedInvoiceBorrowed<'a>> for DetailedInvoice {
            fn from(
                DetailedInvoiceBorrowed {
                    id,
                    status,
                    invoicing_provider,
                    created_at,
                    updated_at,
                    invoice_date,
                    customer_id,
                    subscription_id,
                    currency,
                    days_until_due,
                    issued,
                    issue_attempts,
                    last_issue_attempt_at,
                    last_issue_error,
                    amount_cents,
                    customer_name,
                    plan_name,
                    plan_external_id,
                    plan_version,
                }: DetailedInvoiceBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    status,
                    invoicing_provider,
                    created_at,
                    updated_at,
                    invoice_date,
                    customer_id,
                    subscription_id,
                    currency: currency.into(),
                    days_until_due,
                    issued,
                    issue_attempts,
                    last_issue_attempt_at,
                    last_issue_error: last_issue_error.map(|v| v.into()),
                    amount_cents,
                    customer_name: customer_name.into(),
                    plan_name: plan_name.into(),
                    plan_external_id: plan_external_id.into(),
                    plan_version,
                }
            }
        }
        pub struct DetailedInvoiceQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> DetailedInvoiceBorrowed,
            mapper: fn(DetailedInvoiceBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> DetailedInvoiceQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(DetailedInvoiceBorrowed) -> R,
            ) -> DetailedInvoiceQuery<'a, C, R, N> {
                DetailedInvoiceQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn create_invoice() -> CreateInvoiceStmt {
            CreateInvoiceStmt(cornucopia_async :: private :: Stmt :: new("INSERT INTO invoice (id,
                     status,
                     invoicing_provider,
                     invoice_date,
                     tenant_id,
                     customer_id,
                     subscription_id,
                     plan_version_id,
                     currency,
                     days_until_due,
                     line_items,
                     amount_cents,
                     finalized_at)
VALUES ($1,
        $2::\"InvoiceStatusEnum\",
        $3,
        $4,
        $5,
        $6,
        $7,
        $8,
        $9,
        $10,
        $11,
        $12,
        CASE WHEN $2::\"InvoiceStatusEnum\" = 'FINALIZED' THEN NOW() else null END
       )
RETURNING id, status, invoicing_provider, invoice_date, tenant_id, customer_id, subscription_id, plan_version_id, currency, days_until_due, line_items, amount_cents, issued, issue_attempts,last_issue_attempt_at,last_issue_error"))
        }
        pub struct CreateInvoiceStmt(cornucopia_async::private::Stmt);
        impl CreateInvoiceStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::JsonSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                status: &'a super::super::types::public::InvoiceStatusEnum,
                invoicing_provider: &'a super::super::types::public::InvoicingProviderEnum,
                invoice_date: &'a time::Date,
                tenant_id: &'a uuid::Uuid,
                customer_id: &'a uuid::Uuid,
                subscription_id: &'a uuid::Uuid,
                plan_version_id: &'a uuid::Uuid,
                currency: &'a T1,
                days_until_due: &'a i32,
                line_items: &'a T2,
                amount_cents: &'a Option<i64>,
            ) -> InvoiceQuery<'a, C, Invoice, 12> {
                InvoiceQuery {
                    client,
                    params: [
                        id,
                        status,
                        invoicing_provider,
                        invoice_date,
                        tenant_id,
                        customer_id,
                        subscription_id,
                        plan_version_id,
                        currency,
                        days_until_due,
                        line_items,
                        amount_cents,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| InvoiceBorrowed {
                        id: row.get(0),
                        status: row.get(1),
                        invoicing_provider: row.get(2),
                        invoice_date: row.get(3),
                        tenant_id: row.get(4),
                        customer_id: row.get(5),
                        subscription_id: row.get(6),
                        plan_version_id: row.get(7),
                        currency: row.get(8),
                        days_until_due: row.get(9),
                        line_items: row.get(10),
                        amount_cents: row.get(11),
                        issued: row.get(12),
                        issue_attempts: row.get(13),
                        last_issue_attempt_at: row.get(14),
                        last_issue_error: row.get(15),
                    },
                    mapper: |it| <Invoice>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::JsonSql,
            >
            cornucopia_async::Params<
                'a,
                CreateInvoiceParams<T1, T2>,
                InvoiceQuery<'a, C, Invoice, 12>,
                C,
            > for CreateInvoiceStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateInvoiceParams<T1, T2>,
            ) -> InvoiceQuery<'a, C, Invoice, 12> {
                self.bind(
                    client,
                    &params.id,
                    &params.status,
                    &params.invoicing_provider,
                    &params.invoice_date,
                    &params.tenant_id,
                    &params.customer_id,
                    &params.subscription_id,
                    &params.plan_version_id,
                    &params.currency,
                    &params.days_until_due,
                    &params.line_items,
                    &params.amount_cents,
                )
            }
        }
        pub fn update_invoice_status() -> UpdateInvoiceStatusStmt {
            UpdateInvoiceStatusStmt(cornucopia_async :: private :: Stmt :: new("UPDATE invoice
SET status     = $1::\"InvoiceStatusEnum\",
    updated_at = NOW(),
    finalized_at = CASE WHEN $1::\"InvoiceStatusEnum\" = 'FINALIZED' THEN NOW() ELSE finalized_at END
WHERE id = $2
  AND status NOT IN ('FINALIZED', 'VOID')"))
        }
        pub struct UpdateInvoiceStatusStmt(cornucopia_async::private::Stmt);
        impl UpdateInvoiceStatusStmt {
            pub async fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                status: &'a super::super::types::public::InvoiceStatusEnum,
                id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[status, id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync>
            cornucopia_async::Params<
                'a,
                UpdateInvoiceStatusParams,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for UpdateInvoiceStatusStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpdateInvoiceStatusParams,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.status, &params.id))
            }
        }
        pub fn update_invoice_external_status() -> UpdateInvoiceExternalStatusStmt {
            UpdateInvoiceExternalStatusStmt(cornucopia_async::private::Stmt::new(
                "UPDATE invoice
SET external_status = $1,
    updated_at      = NOW()
WHERE id = $2
RETURNING id, external_status",
            ))
        }
        pub struct UpdateInvoiceExternalStatusStmt(cornucopia_async::private::Stmt);
        impl UpdateInvoiceExternalStatusStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                external_status: &'a super::super::types::public::InvoiceExternalStatusEnum,
                id: &'a uuid::Uuid,
            ) -> UpdateInvoiceExternalStatusQuery<'a, C, UpdateInvoiceExternalStatus, 2>
            {
                UpdateInvoiceExternalStatusQuery {
                    client,
                    params: [external_status, id],
                    stmt: &mut self.0,
                    extractor: |row| UpdateInvoiceExternalStatus {
                        id: row.get(0),
                        external_status: row.get(1),
                    },
                    mapper: |it| <UpdateInvoiceExternalStatus>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                UpdateInvoiceExternalStatusParams,
                UpdateInvoiceExternalStatusQuery<'a, C, UpdateInvoiceExternalStatus, 2>,
                C,
            > for UpdateInvoiceExternalStatusStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpdateInvoiceExternalStatusParams,
            ) -> UpdateInvoiceExternalStatusQuery<'a, C, UpdateInvoiceExternalStatus, 2>
            {
                self.bind(client, &params.external_status, &params.id)
            }
        }
        pub fn update_invoice_lines() -> UpdateInvoiceLinesStmt {
            UpdateInvoiceLinesStmt(cornucopia_async::private::Stmt::new(
                "UPDATE invoice
SET line_items      = $1,
    updated_at      = NOW(),
    data_updated_at = NOW()
WHERE id = $2
  AND status NOT IN ('FINALIZED', 'VOID')",
            ))
        }
        pub struct UpdateInvoiceLinesStmt(cornucopia_async::private::Stmt);
        impl UpdateInvoiceLinesStmt {
            pub async fn bind<'a, C: GenericClient, T1: cornucopia_async::JsonSql>(
                &'a mut self,
                client: &'a C,
                line_items: &'a T1,
                id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[line_items, id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync, T1: cornucopia_async::JsonSql>
            cornucopia_async::Params<
                'a,
                UpdateInvoiceLinesParams<T1>,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for UpdateInvoiceLinesStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpdateInvoiceLinesParams<T1>,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.line_items, &params.id))
            }
        }
        pub fn patch_invoice() -> PatchInvoiceStmt {
            PatchInvoiceStmt(cornucopia_async :: private :: Stmt :: new("UPDATE invoice
SET status             = COALESCE($1, status),
    invoicing_provider = COALESCE($2, invoicing_provider),
    invoice_date       = COALESCE($3, invoice_date),
    currency           = COALESCE($4, currency),
    days_until_due     = COALESCE($5, days_until_due),
    line_items         = COALESCE($6, line_items),
    amount_cents       = COALESCE($7, amount_cents),
    updated_at         = NOW(),
    finalized_at = CASE WHEN $1::\"InvoiceStatusEnum\" = 'FINALIZED' THEN NOW() ELSE finalized_at END
WHERE id = $8
  AND status NOT IN ('FINALIZED', 'VOID')
RETURNING id, status, invoicing_provider, invoice_date, tenant_id, customer_id, subscription_id, plan_version_id, currency, days_until_due, line_items, amount_cents, issued, issue_attempts,last_issue_attempt_at,last_issue_error"))
        }
        pub struct PatchInvoiceStmt(cornucopia_async::private::Stmt);
        impl PatchInvoiceStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::JsonSql,
            >(
                &'a mut self,
                client: &'a C,
                status: &'a Option<super::super::types::public::InvoiceStatusEnum>,
                invoicing_provider: &'a Option<super::super::types::public::InvoicingProviderEnum>,
                invoice_date: &'a Option<time::Date>,
                currency: &'a Option<T1>,
                days_until_due: &'a i32,
                line_items: &'a Option<T2>,
                amount_cents: &'a i64,
                id: &'a uuid::Uuid,
            ) -> InvoiceQuery<'a, C, Invoice, 8> {
                InvoiceQuery {
                    client,
                    params: [
                        status,
                        invoicing_provider,
                        invoice_date,
                        currency,
                        days_until_due,
                        line_items,
                        amount_cents,
                        id,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| InvoiceBorrowed {
                        id: row.get(0),
                        status: row.get(1),
                        invoicing_provider: row.get(2),
                        invoice_date: row.get(3),
                        tenant_id: row.get(4),
                        customer_id: row.get(5),
                        subscription_id: row.get(6),
                        plan_version_id: row.get(7),
                        currency: row.get(8),
                        days_until_due: row.get(9),
                        line_items: row.get(10),
                        amount_cents: row.get(11),
                        issued: row.get(12),
                        issue_attempts: row.get(13),
                        last_issue_attempt_at: row.get(14),
                        last_issue_error: row.get(15),
                    },
                    mapper: |it| <Invoice>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::JsonSql,
            >
            cornucopia_async::Params<
                'a,
                PatchInvoiceParams<T1, T2>,
                InvoiceQuery<'a, C, Invoice, 8>,
                C,
            > for PatchInvoiceStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a PatchInvoiceParams<T1, T2>,
            ) -> InvoiceQuery<'a, C, Invoice, 8> {
                self.bind(
                    client,
                    &params.status,
                    &params.invoicing_provider,
                    &params.invoice_date,
                    &params.currency,
                    &params.days_until_due,
                    &params.line_items,
                    &params.amount_cents,
                    &params.id,
                )
            }
        }
        pub fn update_pending_finalization_invoices() -> UpdatePendingFinalizationInvoicesStmt {
            UpdatePendingFinalizationInvoicesStmt(cornucopia_async::private::Stmt::new(
                "UPDATE invoice
SET status     = 'PENDING',
    updated_at = NOW()
FROM invoicing_config
WHERE invoice.tenant_id = invoicing_config.tenant_id
  AND invoice.status = 'DRAFT'
  AND invoice.invoice_date < NOW()
  AND NOW() <= (invoice.invoice_date + interval '1 hour' * invoicing_config.grace_period_hours)
RETURNING invoice.id, invoice.status",
            ))
        }
        pub struct UpdatePendingFinalizationInvoicesStmt(cornucopia_async::private::Stmt);
        impl UpdatePendingFinalizationInvoicesStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
            ) -> UpdatePendingFinalizationInvoicesQuery<'a, C, UpdatePendingFinalizationInvoices, 0>
            {
                UpdatePendingFinalizationInvoicesQuery {
                    client,
                    params: [],
                    stmt: &mut self.0,
                    extractor: |row| UpdatePendingFinalizationInvoices {
                        id: row.get(0),
                        status: row.get(1),
                    },
                    mapper: |it| <UpdatePendingFinalizationInvoices>::from(it),
                }
            }
        }
        pub fn get_invoices_to_finalize() -> GetInvoicesToFinalizeStmt {
            GetInvoicesToFinalizeStmt(cornucopia_async::private::Stmt::new(
                "SELECT invoice.id,
       invoice.status,
       invoice.invoicing_provider,
       invoice.invoice_date,
       invoice.tenant_id,
       invoice.customer_id,
       invoice.subscription_id,
       invoice.plan_version_id,
       invoice.currency,
       invoice.days_until_due,
       invoice.line_items,
       invoice.amount_cents,
       invoice.issued,
       invoice.issue_attempts,
       invoice.last_issue_attempt_at,
       invoice.last_issue_error
FROM invoice
         JOIN invoicing_config ON invoice.tenant_id = invoicing_config.tenant_id
WHERE invoice.status NOT IN ('VOID', 'FINALIZED')
  AND NOW() > (invoice.invoice_date + interval '1 hour' * invoicing_config.grace_period_hours)",
            ))
        }
        pub struct GetInvoicesToFinalizeStmt(cornucopia_async::private::Stmt);
        impl GetInvoicesToFinalizeStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
            ) -> InvoiceQuery<'a, C, Invoice, 0> {
                InvoiceQuery {
                    client,
                    params: [],
                    stmt: &mut self.0,
                    extractor: |row| InvoiceBorrowed {
                        id: row.get(0),
                        status: row.get(1),
                        invoicing_provider: row.get(2),
                        invoice_date: row.get(3),
                        tenant_id: row.get(4),
                        customer_id: row.get(5),
                        subscription_id: row.get(6),
                        plan_version_id: row.get(7),
                        currency: row.get(8),
                        days_until_due: row.get(9),
                        line_items: row.get(10),
                        amount_cents: row.get(11),
                        issued: row.get(12),
                        issue_attempts: row.get(13),
                        last_issue_attempt_at: row.get(14),
                        last_issue_error: row.get(15),
                    },
                    mapper: |it| <Invoice>::from(it),
                }
            }
        }
        pub fn get_outdated_invoices() -> GetOutdatedInvoicesStmt {
            GetOutdatedInvoicesStmt(cornucopia_async :: private :: Stmt :: new("SELECT invoice.id,
       invoice.status,
       invoice.invoicing_provider,
       invoice.invoice_date,
       invoice.tenant_id,
       invoice.customer_id,
       invoice.subscription_id,
       invoice.plan_version_id,
       invoice.currency,
       invoice.days_until_due,
       invoice.line_items,
       invoice.amount_cents,
       invoice.issued,
       invoice.issue_attempts,
       invoice.last_issue_attempt_at,
       invoice.last_issue_error
FROM invoice
WHERE invoice.status NOT IN ('VOID', 'FINALIZED')
  AND (
    invoice.data_updated_at IS NULL
        OR invoice.data_updated_at < NOW() -
                                     interval '1 hour' -- TODO configurable, per org plan (via invoicing_config) & store skew metric for alerting
    )
ORDER BY invoice.data_updated_at IS NULL DESC,
         invoice.data_updated_at ASC"))
        }
        pub struct GetOutdatedInvoicesStmt(cornucopia_async::private::Stmt);
        impl GetOutdatedInvoicesStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
            ) -> InvoiceQuery<'a, C, Invoice, 0> {
                InvoiceQuery {
                    client,
                    params: [],
                    stmt: &mut self.0,
                    extractor: |row| InvoiceBorrowed {
                        id: row.get(0),
                        status: row.get(1),
                        invoicing_provider: row.get(2),
                        invoice_date: row.get(3),
                        tenant_id: row.get(4),
                        customer_id: row.get(5),
                        subscription_id: row.get(6),
                        plan_version_id: row.get(7),
                        currency: row.get(8),
                        days_until_due: row.get(9),
                        line_items: row.get(10),
                        amount_cents: row.get(11),
                        issued: row.get(12),
                        issue_attempts: row.get(13),
                        last_issue_attempt_at: row.get(14),
                        last_issue_error: row.get(15),
                    },
                    mapper: |it| <Invoice>::from(it),
                }
            }
        }
        pub fn get_invoices_to_issue() -> GetInvoicesToIssueStmt {
            GetInvoicesToIssueStmt(cornucopia_async::private::Stmt::new(
                "SELECT id,
       status,
       invoicing_provider,
       invoice_date,
       tenant_id,
       customer_id,
       subscription_id,
       plan_version_id,
       currency,
       days_until_due,
       line_items,
       amount_cents,
       issued,
       issue_attempts,
       last_issue_attempt_at,
       last_issue_error
FROM invoice
WHERE status = 'FINALIZED'
  AND issued = false
  AND issue_attempts < $1",
            ))
        }
        pub struct GetInvoicesToIssueStmt(cornucopia_async::private::Stmt);
        impl GetInvoicesToIssueStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                issue_max_attempts: &'a i32,
            ) -> InvoiceQuery<'a, C, Invoice, 1> {
                InvoiceQuery {
                    client,
                    params: [issue_max_attempts],
                    stmt: &mut self.0,
                    extractor: |row| InvoiceBorrowed {
                        id: row.get(0),
                        status: row.get(1),
                        invoicing_provider: row.get(2),
                        invoice_date: row.get(3),
                        tenant_id: row.get(4),
                        customer_id: row.get(5),
                        subscription_id: row.get(6),
                        plan_version_id: row.get(7),
                        currency: row.get(8),
                        days_until_due: row.get(9),
                        line_items: row.get(10),
                        amount_cents: row.get(11),
                        issued: row.get(12),
                        issue_attempts: row.get(13),
                        last_issue_attempt_at: row.get(14),
                        last_issue_error: row.get(15),
                    },
                    mapper: |it| <Invoice>::from(it),
                }
            }
        }
        pub fn list_tenant_invoices() -> ListTenantInvoicesStmt {
            ListTenantInvoicesStmt(cornucopia_async::private::Stmt::new(
                "SELECT invoice.id,
       invoice.status,
       invoice.invoicing_provider,
       invoice.created_at,
       invoice.invoice_date,
       invoice.customer_id,
       invoice.subscription_id,
       invoice.currency,
       invoice.days_until_due,
       invoice.amount_cents,
       customer.name    AS customer_name,
       COUNT(*) OVER () AS total_count
FROM invoice
         JOIN customer ON customer_id = customer.id
WHERE invoice.tenant_id = $1
  AND ($2 :: \"InvoiceStatusEnum\" IS NULL OR invoice.status = $2)
  AND ($3 :: TEXT IS NULL OR customer.name ILIKE '%' || $3 || '%')
  AND ($4 :: UUID IS NULL OR customer_id = $4)
ORDER BY CASE
             WHEN $5 = 'DATE_DESC' THEN invoice.created_at
             END DESC,
         CASE
             WHEN $5 = 'DATE_ASC' THEN invoice.created_at
             END ASC,
         CASE
             WHEN $5 = 'ID_DESC' THEN invoice.invoice_id
             END DESC,
         CASE
             WHEN $5 = 'ID_ASC' THEN invoice.invoice_id
             END ASC
LIMIT $6 OFFSET $7",
            ))
        }
        pub struct ListTenantInvoicesStmt(cornucopia_async::private::Stmt);
        impl ListTenantInvoicesStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                status: &'a Option<super::super::types::public::InvoiceStatusEnum>,
                search: &'a Option<T1>,
                customer_id: &'a Option<uuid::Uuid>,
                order_by: &'a T2,
                limit: &'a i64,
                offset: &'a i64,
            ) -> ListInvoiceQuery<'a, C, ListInvoice, 7> {
                ListInvoiceQuery {
                    client,
                    params: [
                        tenant_id,
                        status,
                        search,
                        customer_id,
                        order_by,
                        limit,
                        offset,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| ListInvoiceBorrowed {
                        id: row.get(0),
                        status: row.get(1),
                        invoicing_provider: row.get(2),
                        created_at: row.get(3),
                        invoice_date: row.get(4),
                        customer_id: row.get(5),
                        subscription_id: row.get(6),
                        currency: row.get(7),
                        days_until_due: row.get(8),
                        amount_cents: row.get(9),
                        customer_name: row.get(10),
                        total_count: row.get(11),
                    },
                    mapper: |it| <ListInvoice>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                ListTenantInvoicesParams<T1, T2>,
                ListInvoiceQuery<'a, C, ListInvoice, 7>,
                C,
            > for ListTenantInvoicesStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListTenantInvoicesParams<T1, T2>,
            ) -> ListInvoiceQuery<'a, C, ListInvoice, 7> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.status,
                    &params.search,
                    &params.customer_id,
                    &params.order_by,
                    &params.limit,
                    &params.offset,
                )
            }
        }
        pub fn update_invoice_issue_success() -> UpdateInvoiceIssueSuccessStmt {
            UpdateInvoiceIssueSuccessStmt(cornucopia_async::private::Stmt::new(
                "UPDATE invoice
SET issued                = true,
    issue_attempts        = $1,
    last_issue_attempt_at = NOW(),
    updated_at            = NOW()
WHERE id = $2
  AND status = 'FINALIZED'
  AND issued = false",
            ))
        }
        pub struct UpdateInvoiceIssueSuccessStmt(cornucopia_async::private::Stmt);
        impl UpdateInvoiceIssueSuccessStmt {
            pub async fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                issue_attempts: &'a i32,
                id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[issue_attempts, id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync>
            cornucopia_async::Params<
                'a,
                UpdateInvoiceIssueSuccessParams,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for UpdateInvoiceIssueSuccessStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpdateInvoiceIssueSuccessParams,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.issue_attempts, &params.id))
            }
        }
        pub fn update_invoice_issue_error() -> UpdateInvoiceIssueErrorStmt {
            UpdateInvoiceIssueErrorStmt(cornucopia_async::private::Stmt::new(
                "UPDATE invoice
SET issue_attempts        = $1,
    last_issue_attempt_at = NOW(),
    updated_at            = NOW(),
    last_issue_error      = $2
WHERE id = $3
  AND status = 'FINALIZED'
  AND issued = false",
            ))
        }
        pub struct UpdateInvoiceIssueErrorStmt(cornucopia_async::private::Stmt);
        impl UpdateInvoiceIssueErrorStmt {
            pub async fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                issue_attempts: &'a i32,
                last_issue_error: &'a T1,
                id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client
                    .execute(stmt, &[issue_attempts, last_issue_error, id])
                    .await
            }
        }
        impl<'a, C: GenericClient + Send + Sync, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                UpdateInvoiceIssueErrorParams<T1>,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for UpdateInvoiceIssueErrorStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpdateInvoiceIssueErrorParams<T1>,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(
                    client,
                    &params.issue_attempts,
                    &params.last_issue_error,
                    &params.id,
                ))
            }
        }
        pub fn invoice_by_id() -> InvoiceByIdStmt {
            InvoiceByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT id,
       status,
       invoicing_provider,
       invoice_date,
       tenant_id,
       customer_id,
       subscription_id,
       plan_version_id,
       currency,
       days_until_due,
       line_items,
       amount_cents,
       issued,
       issue_attempts,
       last_issue_attempt_at,
       last_issue_error
FROM invoice
WHERE id = $1",
            ))
        }
        pub struct InvoiceByIdStmt(cornucopia_async::private::Stmt);
        impl InvoiceByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
            ) -> InvoiceQuery<'a, C, Invoice, 1> {
                InvoiceQuery {
                    client,
                    params: [id],
                    stmt: &mut self.0,
                    extractor: |row| InvoiceBorrowed {
                        id: row.get(0),
                        status: row.get(1),
                        invoicing_provider: row.get(2),
                        invoice_date: row.get(3),
                        tenant_id: row.get(4),
                        customer_id: row.get(5),
                        subscription_id: row.get(6),
                        plan_version_id: row.get(7),
                        currency: row.get(8),
                        days_until_due: row.get(9),
                        line_items: row.get(10),
                        amount_cents: row.get(11),
                        issued: row.get(12),
                        issue_attempts: row.get(13),
                        last_issue_attempt_at: row.get(14),
                        last_issue_error: row.get(15),
                    },
                    mapper: |it| <Invoice>::from(it),
                }
            }
        }
        pub fn get_tenant_invoice_by_id() -> GetTenantInvoiceByIdStmt {
            GetTenantInvoiceByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT invoice.id,
       invoice.status,
       invoice.invoicing_provider,
       invoice.created_at,
       invoice.updated_at,
       invoice.invoice_date,
       invoice.customer_id,
       invoice.subscription_id,
       invoice.currency,
       invoice.days_until_due,
       invoice.issued,
       invoice.issue_attempts,
       invoice.last_issue_attempt_at,
       invoice.last_issue_error,
       invoice.amount_cents,
       customer.name        AS customer_name,
       plan.name            AS plan_name,
       plan.external_id     AS plan_external_id,
       plan_version.version AS plan_version
FROM invoice
         JOIN customer ON customer_id = customer.id
         JOIN subscription ON subscription_id = subscription.id
         JOIN plan_version ON subscription.plan_version_id = plan_version.id
         JOIN plan ON plan_version.plan_id = plan.id
WHERE invoice.id = $1
  AND invoice.tenant_id = $2",
            ))
        }
        pub struct GetTenantInvoiceByIdStmt(cornucopia_async::private::Stmt);
        impl GetTenantInvoiceByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> DetailedInvoiceQuery<'a, C, DetailedInvoice, 2> {
                DetailedInvoiceQuery {
                    client,
                    params: [id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| DetailedInvoiceBorrowed {
                        id: row.get(0),
                        status: row.get(1),
                        invoicing_provider: row.get(2),
                        created_at: row.get(3),
                        updated_at: row.get(4),
                        invoice_date: row.get(5),
                        customer_id: row.get(6),
                        subscription_id: row.get(7),
                        currency: row.get(8),
                        days_until_due: row.get(9),
                        issued: row.get(10),
                        issue_attempts: row.get(11),
                        last_issue_attempt_at: row.get(12),
                        last_issue_error: row.get(13),
                        amount_cents: row.get(14),
                        customer_name: row.get(15),
                        plan_name: row.get(16),
                        plan_external_id: row.get(17),
                        plan_version: row.get(18),
                    },
                    mapper: |it| <DetailedInvoice>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetTenantInvoiceByIdParams,
                DetailedInvoiceQuery<'a, C, DetailedInvoice, 2>,
                C,
            > for GetTenantInvoiceByIdStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetTenantInvoiceByIdParams,
            ) -> DetailedInvoiceQuery<'a, C, DetailedInvoice, 2> {
                self.bind(client, &params.id, &params.tenant_id)
            }
        }
    }
    pub mod organizations {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct GetOrganizationBySlugForUserParams<T1: cornucopia_async::StringSql> {
            pub slug: T1,
            pub user_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct CreateOrganizationParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub slug: T2,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct CreateOrganizationMemberParams {
            pub user_id: uuid::Uuid,
            pub organization_id: uuid::Uuid,
            pub role: super::super::types::public::OrganizationUserRole,
        }
        #[derive(Debug)]
        pub struct SetInviteParams<T1: cornucopia_async::StringSql> {
            pub invite_link_hash: T1,
            pub organization_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct Organization {
            pub id: uuid::Uuid,
            pub name: String,
            pub slug: String,
        }
        pub struct OrganizationBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub slug: &'a str,
        }
        impl<'a> From<OrganizationBorrowed<'a>> for Organization {
            fn from(OrganizationBorrowed { id, name, slug }: OrganizationBorrowed<'a>) -> Self {
                Self {
                    id,
                    name: name.into(),
                    slug: slug.into(),
                }
            }
        }
        pub struct OrganizationQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> OrganizationBorrowed,
            mapper: fn(OrganizationBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> OrganizationQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(OrganizationBorrowed) -> R,
            ) -> OrganizationQuery<'a, C, R, N> {
                OrganizationQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct OrganizationWithRole {
            pub id: uuid::Uuid,
            pub name: String,
            pub slug: String,
            pub role: super::super::types::public::OrganizationUserRole,
        }
        pub struct OrganizationWithRoleBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub slug: &'a str,
            pub role: super::super::types::public::OrganizationUserRole,
        }
        impl<'a> From<OrganizationWithRoleBorrowed<'a>> for OrganizationWithRole {
            fn from(
                OrganizationWithRoleBorrowed {
                    id,
                    name,
                    slug,
                    role,
                }: OrganizationWithRoleBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    slug: slug.into(),
                    role,
                }
            }
        }
        pub struct OrganizationWithRoleQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> OrganizationWithRoleBorrowed,
            mapper: fn(OrganizationWithRoleBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> OrganizationWithRoleQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(OrganizationWithRoleBorrowed) -> R,
            ) -> OrganizationWithRoleQuery<'a, C, R, N> {
                OrganizationWithRoleQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ListOrganizationMembers {
            pub user_id: uuid::Uuid,
            pub user_email: String,
            pub organization_role: super::super::types::public::OrganizationUserRole,
        }
        pub struct ListOrganizationMembersBorrowed<'a> {
            pub user_id: uuid::Uuid,
            pub user_email: &'a str,
            pub organization_role: super::super::types::public::OrganizationUserRole,
        }
        impl<'a> From<ListOrganizationMembersBorrowed<'a>> for ListOrganizationMembers {
            fn from(
                ListOrganizationMembersBorrowed {
                    user_id,
                    user_email,
                    organization_role,
                }: ListOrganizationMembersBorrowed<'a>,
            ) -> Self {
                Self {
                    user_id,
                    user_email: user_email.into(),
                    organization_role,
                }
            }
        }
        pub struct ListOrganizationMembersQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ListOrganizationMembersBorrowed,
            mapper: fn(ListOrganizationMembersBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ListOrganizationMembersQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(ListOrganizationMembersBorrowed) -> R,
            ) -> ListOrganizationMembersQuery<'a, C, R, N> {
                ListOrganizationMembersQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct CreateOrganizationMember {
            pub user_id: uuid::Uuid,
            pub organization_id: uuid::Uuid,
        }
        pub struct CreateOrganizationMemberQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> CreateOrganizationMember,
            mapper: fn(CreateOrganizationMember) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> CreateOrganizationMemberQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(CreateOrganizationMember) -> R,
            ) -> CreateOrganizationMemberQuery<'a, C, R, N> {
                CreateOrganizationMemberQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct Instance {
            pub id: uuid::Uuid,
            pub name: String,
            pub slug: String,
        }
        pub struct InstanceBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub slug: &'a str,
        }
        impl<'a> From<InstanceBorrowed<'a>> for Instance {
            fn from(InstanceBorrowed { id, name, slug }: InstanceBorrowed<'a>) -> Self {
                Self {
                    id,
                    name: name.into(),
                    slug: slug.into(),
                }
            }
        }
        pub struct InstanceQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> InstanceBorrowed,
            mapper: fn(InstanceBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> InstanceQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(InstanceBorrowed) -> R) -> InstanceQuery<'a, C, R, N> {
                InstanceQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub struct OptionStringQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> Option<&str>,
            mapper: fn(Option<&str>) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> OptionStringQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(Option<&str>) -> R) -> OptionStringQuery<'a, C, R, N> {
                OptionStringQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct GetOrganizationByInviteHash {
            pub id: uuid::Uuid,
            pub name: String,
        }
        pub struct GetOrganizationByInviteHashBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
        }
        impl<'a> From<GetOrganizationByInviteHashBorrowed<'a>> for GetOrganizationByInviteHash {
            fn from(
                GetOrganizationByInviteHashBorrowed { id,name,} : GetOrganizationByInviteHashBorrowed < 'a >,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                }
            }
        }
        pub struct GetOrganizationByInviteHashQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> GetOrganizationByInviteHashBorrowed,
            mapper: fn(GetOrganizationByInviteHashBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> GetOrganizationByInviteHashQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(GetOrganizationByInviteHashBorrowed) -> R,
            ) -> GetOrganizationByInviteHashQuery<'a, C, R, N> {
                GetOrganizationByInviteHashQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn get_organization_by_slug() -> GetOrganizationBySlugStmt {
            GetOrganizationBySlugStmt(cornucopia_async::private::Stmt::new(
                "SELECT id, name, slug
FROM organization
WHERE slug = $1",
            ))
        }
        pub struct GetOrganizationBySlugStmt(cornucopia_async::private::Stmt);
        impl GetOrganizationBySlugStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                slug: &'a T1,
            ) -> OrganizationQuery<'a, C, Organization, 1> {
                OrganizationQuery {
                    client,
                    params: [slug],
                    stmt: &mut self.0,
                    extractor: |row| OrganizationBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                    },
                    mapper: |it| <Organization>::from(it),
                }
            }
        }
        pub fn get_organization_by_slug_for_user() -> GetOrganizationBySlugForUserStmt {
            GetOrganizationBySlugForUserStmt(cornucopia_async::private::Stmt::new(
                "SELECT o.id, o.name, o.slug, om.role
FROM organization as o
JOIN organization_member AS om ON om.organization_id = o.id
WHERE slug = $1
AND om.user_id = $2",
            ))
        }
        pub struct GetOrganizationBySlugForUserStmt(cornucopia_async::private::Stmt);
        impl GetOrganizationBySlugForUserStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                slug: &'a T1,
                user_id: &'a uuid::Uuid,
            ) -> OrganizationWithRoleQuery<'a, C, OrganizationWithRole, 2> {
                OrganizationWithRoleQuery {
                    client,
                    params: [slug, user_id],
                    stmt: &mut self.0,
                    extractor: |row| OrganizationWithRoleBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                        role: row.get(3),
                    },
                    mapper: |it| <OrganizationWithRole>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                GetOrganizationBySlugForUserParams<T1>,
                OrganizationWithRoleQuery<'a, C, OrganizationWithRole, 2>,
                C,
            > for GetOrganizationBySlugForUserStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetOrganizationBySlugForUserParams<T1>,
            ) -> OrganizationWithRoleQuery<'a, C, OrganizationWithRole, 2> {
                self.bind(client, &params.slug, &params.user_id)
            }
        }
        pub fn list_organizations_for_user() -> ListOrganizationsForUserStmt {
            ListOrganizationsForUserStmt(cornucopia_async::private::Stmt::new(
                "SELECT o.id, o.name, o.slug, om.role
FROM organization AS o
JOIN organization_member AS om ON om.organization_id = o.id
WHERE om.user_id = $1",
            ))
        }
        pub struct ListOrganizationsForUserStmt(cornucopia_async::private::Stmt);
        impl ListOrganizationsForUserStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                user_id: &'a uuid::Uuid,
            ) -> OrganizationWithRoleQuery<'a, C, OrganizationWithRole, 1> {
                OrganizationWithRoleQuery {
                    client,
                    params: [user_id],
                    stmt: &mut self.0,
                    extractor: |row| OrganizationWithRoleBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                        role: row.get(3),
                    },
                    mapper: |it| <OrganizationWithRole>::from(it),
                }
            }
        }
        pub fn list_organization_members() -> ListOrganizationMembersStmt {
            ListOrganizationMembersStmt(cornucopia_async::private::Stmt::new(
                "SELECT
  mem.user_id,
  usr.email AS user_email,
  mem.role AS organization_role
FROM
  organization_member AS mem
JOIN
  \"user\" AS usr ON mem.user_id = usr.id
WHERE
  mem.organization_id = $1",
            ))
        }
        pub struct ListOrganizationMembersStmt(cornucopia_async::private::Stmt);
        impl ListOrganizationMembersStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                organization_id: &'a uuid::Uuid,
            ) -> ListOrganizationMembersQuery<'a, C, ListOrganizationMembers, 1> {
                ListOrganizationMembersQuery {
                    client,
                    params: [organization_id],
                    stmt: &mut self.0,
                    extractor: |row| ListOrganizationMembersBorrowed {
                        user_id: row.get(0),
                        user_email: row.get(1),
                        organization_role: row.get(2),
                    },
                    mapper: |it| <ListOrganizationMembers>::from(it),
                }
            }
        }
        pub fn create_organization() -> CreateOrganizationStmt {
            CreateOrganizationStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO organization(id, name, slug)
VALUES ($1, $2, $3)
RETURNING id, name, slug",
            ))
        }
        pub struct CreateOrganizationStmt(cornucopia_async::private::Stmt);
        impl CreateOrganizationStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                slug: &'a T2,
            ) -> OrganizationQuery<'a, C, Organization, 3> {
                OrganizationQuery {
                    client,
                    params: [id, name, slug],
                    stmt: &mut self.0,
                    extractor: |row| OrganizationBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                    },
                    mapper: |it| <Organization>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                CreateOrganizationParams<T1, T2>,
                OrganizationQuery<'a, C, Organization, 3>,
                C,
            > for CreateOrganizationStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateOrganizationParams<T1, T2>,
            ) -> OrganizationQuery<'a, C, Organization, 3> {
                self.bind(client, &params.id, &params.name, &params.slug)
            }
        }
        pub fn create_organization_member() -> CreateOrganizationMemberStmt {
            CreateOrganizationMemberStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO organization_member(user_id, organization_id, role)
VALUES ($1, $2, $3)
RETURNING user_id, organization_id",
            ))
        }
        pub struct CreateOrganizationMemberStmt(cornucopia_async::private::Stmt);
        impl CreateOrganizationMemberStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                user_id: &'a uuid::Uuid,
                organization_id: &'a uuid::Uuid,
                role: &'a super::super::types::public::OrganizationUserRole,
            ) -> CreateOrganizationMemberQuery<'a, C, CreateOrganizationMember, 3> {
                CreateOrganizationMemberQuery {
                    client,
                    params: [user_id, organization_id, role],
                    stmt: &mut self.0,
                    extractor: |row| CreateOrganizationMember {
                        user_id: row.get(0),
                        organization_id: row.get(1),
                    },
                    mapper: |it| <CreateOrganizationMember>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                CreateOrganizationMemberParams,
                CreateOrganizationMemberQuery<'a, C, CreateOrganizationMember, 3>,
                C,
            > for CreateOrganizationMemberStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateOrganizationMemberParams,
            ) -> CreateOrganizationMemberQuery<'a, C, CreateOrganizationMember, 3> {
                self.bind(
                    client,
                    &params.user_id,
                    &params.organization_id,
                    &params.role,
                )
            }
        }
        pub fn instance() -> InstanceStmt {
            InstanceStmt(cornucopia_async::private::Stmt::new(
                "SELECT o.id, o.name, o.slug
FROM organization AS o
LIMIT 1",
            ))
        }
        pub struct InstanceStmt(cornucopia_async::private::Stmt);
        impl InstanceStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
            ) -> InstanceQuery<'a, C, Instance, 0> {
                InstanceQuery {
                    client,
                    params: [],
                    stmt: &mut self.0,
                    extractor: |row| InstanceBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                    },
                    mapper: |it| <Instance>::from(it),
                }
            }
        }
        pub fn get_invite() -> GetInviteStmt {
            GetInviteStmt(cornucopia_async::private::Stmt::new(
                "SELECT invite_link_hash
FROM organization
WHERE id = $1",
            ))
        }
        pub struct GetInviteStmt(cornucopia_async::private::Stmt);
        impl GetInviteStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                organization_id: &'a uuid::Uuid,
            ) -> OptionStringQuery<'a, C, Option<String>, 1> {
                OptionStringQuery {
                    client,
                    params: [organization_id],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it.map(|v| v.into()),
                }
            }
        }
        pub fn get_organization_by_invite_hash() -> GetOrganizationByInviteHashStmt {
            GetOrganizationByInviteHashStmt(cornucopia_async::private::Stmt::new(
                "SELECT id, name
FROM organization
WHERE invite_link_hash = $1",
            ))
        }
        pub struct GetOrganizationByInviteHashStmt(cornucopia_async::private::Stmt);
        impl GetOrganizationByInviteHashStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                invite_hash: &'a T1,
            ) -> GetOrganizationByInviteHashQuery<'a, C, GetOrganizationByInviteHash, 1>
            {
                GetOrganizationByInviteHashQuery {
                    client,
                    params: [invite_hash],
                    stmt: &mut self.0,
                    extractor: |row| GetOrganizationByInviteHashBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                    },
                    mapper: |it| <GetOrganizationByInviteHash>::from(it),
                }
            }
        }
        pub fn set_invite() -> SetInviteStmt {
            SetInviteStmt(cornucopia_async::private::Stmt::new(
                "UPDATE organization
SET invite_link_hash = $1
WHERE id = $2 ",
            ))
        }
        pub struct SetInviteStmt(cornucopia_async::private::Stmt);
        impl SetInviteStmt {
            pub async fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                invite_link_hash: &'a T1,
                organization_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client
                    .execute(stmt, &[invite_link_hash, organization_id])
                    .await
            }
        }
        impl<'a, C: GenericClient + Send + Sync, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                SetInviteParams<T1>,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for SetInviteStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a SetInviteParams<T1>,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.invite_link_hash, &params.organization_id))
            }
        }
    }
    pub mod plans {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreatePlanParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
            T4: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub external_id: T2,
            pub description: Option<T3>,
            pub tenant_id: uuid::Uuid,
            pub created_by: uuid::Uuid,
            pub status: super::super::types::public::PlanStatusEnum,
            pub plan_type: super::super::types::public::PlanTypeEnum,
            pub product_family_external_id: T4,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetPlanVersionByIdParams {
            pub plan_version_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct CreatePlanVersionParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::ArraySql<Item = super::super::types::public::BillingPeriodEnum>,
        > {
            pub id: uuid::Uuid,
            pub plan_id: uuid::Uuid,
            pub version: i32,
            pub created_by: uuid::Uuid,
            pub trial_duration_days: Option<i32>,
            pub trial_fallback_plan_id: Option<uuid::Uuid>,
            pub tenant_id: uuid::Uuid,
            pub period_start_day: Option<i16>,
            pub net_terms: Option<i32>,
            pub currency: Option<T1>,
            pub billing_cycles: Option<i32>,
            pub billing_periods: T2,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct CopyVersionToDraftParams {
            pub original_plan_version_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub new_plan_version_id: uuid::Uuid,
            pub created_by: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct PublishPlanVersionParams {
            pub plan_version_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct ActivatePlanParams {
            pub plan_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct FindPlanByExternalIdParams<T1: cornucopia_async::StringSql> {
            pub tenant_id: uuid::Uuid,
            pub external_id: T1,
        }
        #[derive(Debug)]
        pub struct ListPlansParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
        > {
            pub tenant_id: uuid::Uuid,
            pub search: Option<T1>,
            pub product_family_external_id: T2,
            pub order_by: T3,
            pub limit: i64,
            pub offset: i64,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct ListPlansVersionsParams {
            pub tenant_id: uuid::Uuid,
            pub plan_id: uuid::Uuid,
            pub limit: i64,
            pub offset: i64,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct LastPlanVersionParams {
            pub tenant_id: uuid::Uuid,
            pub plan_id: uuid::Uuid,
            pub is_draft: Option<bool>,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct DeleteDraftPlanVersionParams {
            pub plan_version_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct DeleteAllDraftVersionsOfSamePlanParams {
            pub plan_version_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct UpdatePlanVersionOverviewParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::ArraySql<Item = super::super::types::public::BillingPeriodEnum>,
        > {
            pub currency: T1,
            pub net_terms: i32,
            pub billing_periods: T2,
            pub tenant_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct UpdatePlanOverviewParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
        > {
            pub name: T1,
            pub description: Option<T2>,
            pub tenant_id: uuid::Uuid,
            pub plan_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct GetPlanOverviewByExternalIdParams<T1: cornucopia_async::StringSql> {
            pub external_id: T1,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetPlanOverviewByIdParams {
            pub plan_version_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct DeletePlanIfNoVersionsParams {
            pub plan_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct Plan {
            pub id: uuid::Uuid,
            pub name: String,
            pub external_id: String,
            pub description: Option<String>,
            pub status: super::super::types::public::PlanStatusEnum,
            pub plan_type: super::super::types::public::PlanTypeEnum,
        }
        pub struct PlanBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub external_id: &'a str,
            pub description: Option<&'a str>,
            pub status: super::super::types::public::PlanStatusEnum,
            pub plan_type: super::super::types::public::PlanTypeEnum,
        }
        impl<'a> From<PlanBorrowed<'a>> for Plan {
            fn from(
                PlanBorrowed {
                    id,
                    name,
                    external_id,
                    description,
                    status,
                    plan_type,
                }: PlanBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    external_id: external_id.into(),
                    description: description.map(|v| v.into()),
                    status,
                    plan_type,
                }
            }
        }
        pub struct PlanQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> PlanBorrowed,
            mapper: fn(PlanBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> PlanQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(PlanBorrowed) -> R) -> PlanQuery<'a, C, R, N> {
                PlanQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct PlanVersion {
            pub id: uuid::Uuid,
            pub is_draft_version: bool,
            pub plan_id: uuid::Uuid,
            pub version: i32,
            pub created_by: uuid::Uuid,
            pub trial_duration_days: Option<i32>,
            pub trial_fallback_plan_id: Option<uuid::Uuid>,
            pub tenant_id: uuid::Uuid,
            pub period_start_day: Option<i16>,
            pub net_terms: i32,
            pub currency: String,
            pub billing_cycles: Option<i32>,
            pub billing_periods: Vec<super::super::types::public::BillingPeriodEnum>,
        }
        pub struct PlanVersionBorrowed<'a> {
            pub id: uuid::Uuid,
            pub is_draft_version: bool,
            pub plan_id: uuid::Uuid,
            pub version: i32,
            pub created_by: uuid::Uuid,
            pub trial_duration_days: Option<i32>,
            pub trial_fallback_plan_id: Option<uuid::Uuid>,
            pub tenant_id: uuid::Uuid,
            pub period_start_day: Option<i16>,
            pub net_terms: i32,
            pub currency: &'a str,
            pub billing_cycles: Option<i32>,
            pub billing_periods:
                cornucopia_async::ArrayIterator<'a, super::super::types::public::BillingPeriodEnum>,
        }
        impl<'a> From<PlanVersionBorrowed<'a>> for PlanVersion {
            fn from(
                PlanVersionBorrowed {
                    id,
                    is_draft_version,
                    plan_id,
                    version,
                    created_by,
                    trial_duration_days,
                    trial_fallback_plan_id,
                    tenant_id,
                    period_start_day,
                    net_terms,
                    currency,
                    billing_cycles,
                    billing_periods,
                }: PlanVersionBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    is_draft_version,
                    plan_id,
                    version,
                    created_by,
                    trial_duration_days,
                    trial_fallback_plan_id,
                    tenant_id,
                    period_start_day,
                    net_terms,
                    currency: currency.into(),
                    billing_cycles,
                    billing_periods: billing_periods.map(|v| v).collect(),
                }
            }
        }
        pub struct PlanVersionQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> PlanVersionBorrowed,
            mapper: fn(PlanVersionBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> PlanVersionQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(PlanVersionBorrowed) -> R,
            ) -> PlanVersionQuery<'a, C, R, N> {
                PlanVersionQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ListPlan {
            pub id: uuid::Uuid,
            pub name: String,
            pub external_id: String,
            pub description: Option<String>,
            pub status: super::super::types::public::PlanStatusEnum,
            pub plan_type: super::super::types::public::PlanTypeEnum,
            pub total_count: i64,
        }
        pub struct ListPlanBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub external_id: &'a str,
            pub description: Option<&'a str>,
            pub status: super::super::types::public::PlanStatusEnum,
            pub plan_type: super::super::types::public::PlanTypeEnum,
            pub total_count: i64,
        }
        impl<'a> From<ListPlanBorrowed<'a>> for ListPlan {
            fn from(
                ListPlanBorrowed {
                    id,
                    name,
                    external_id,
                    description,
                    status,
                    plan_type,
                    total_count,
                }: ListPlanBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    external_id: external_id.into(),
                    description: description.map(|v| v.into()),
                    status,
                    plan_type,
                    total_count,
                }
            }
        }
        pub struct ListPlanQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ListPlanBorrowed,
            mapper: fn(ListPlanBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ListPlanQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(ListPlanBorrowed) -> R) -> ListPlanQuery<'a, C, R, N> {
                ListPlanQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ListPlanVersion {
            pub id: uuid::Uuid,
            pub is_draft_version: bool,
            pub plan_id: uuid::Uuid,
            pub version: i32,
            pub created_by: uuid::Uuid,
            pub trial_duration_days: Option<i32>,
            pub trial_fallback_plan_id: Option<uuid::Uuid>,
            pub tenant_id: uuid::Uuid,
            pub period_start_day: Option<i16>,
            pub net_terms: i32,
            pub currency: String,
            pub billing_cycles: Option<i32>,
            pub billing_periods: Vec<super::super::types::public::BillingPeriodEnum>,
            pub total_count: i64,
        }
        pub struct ListPlanVersionBorrowed<'a> {
            pub id: uuid::Uuid,
            pub is_draft_version: bool,
            pub plan_id: uuid::Uuid,
            pub version: i32,
            pub created_by: uuid::Uuid,
            pub trial_duration_days: Option<i32>,
            pub trial_fallback_plan_id: Option<uuid::Uuid>,
            pub tenant_id: uuid::Uuid,
            pub period_start_day: Option<i16>,
            pub net_terms: i32,
            pub currency: &'a str,
            pub billing_cycles: Option<i32>,
            pub billing_periods:
                cornucopia_async::ArrayIterator<'a, super::super::types::public::BillingPeriodEnum>,
            pub total_count: i64,
        }
        impl<'a> From<ListPlanVersionBorrowed<'a>> for ListPlanVersion {
            fn from(
                ListPlanVersionBorrowed {
                    id,
                    is_draft_version,
                    plan_id,
                    version,
                    created_by,
                    trial_duration_days,
                    trial_fallback_plan_id,
                    tenant_id,
                    period_start_day,
                    net_terms,
                    currency,
                    billing_cycles,
                    billing_periods,
                    total_count,
                }: ListPlanVersionBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    is_draft_version,
                    plan_id,
                    version,
                    created_by,
                    trial_duration_days,
                    trial_fallback_plan_id,
                    tenant_id,
                    period_start_day,
                    net_terms,
                    currency: currency.into(),
                    billing_cycles,
                    billing_periods: billing_periods.map(|v| v).collect(),
                    total_count,
                }
            }
        }
        pub struct ListPlanVersionQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ListPlanVersionBorrowed,
            mapper: fn(ListPlanVersionBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ListPlanVersionQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(ListPlanVersionBorrowed) -> R,
            ) -> ListPlanVersionQuery<'a, C, R, N> {
                ListPlanVersionQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct PlanOverview {
            pub id: uuid::Uuid,
            pub name: String,
            pub description: Option<String>,
            pub plan_version_id: uuid::Uuid,
            pub is_draft_version: bool,
            pub currency: String,
            pub version: i32,
            pub net_terms: i32,
            pub billing_periods: Vec<super::super::types::public::BillingPeriodEnum>,
        }
        pub struct PlanOverviewBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub description: Option<&'a str>,
            pub plan_version_id: uuid::Uuid,
            pub is_draft_version: bool,
            pub currency: &'a str,
            pub version: i32,
            pub net_terms: i32,
            pub billing_periods:
                cornucopia_async::ArrayIterator<'a, super::super::types::public::BillingPeriodEnum>,
        }
        impl<'a> From<PlanOverviewBorrowed<'a>> for PlanOverview {
            fn from(
                PlanOverviewBorrowed {
                    id,
                    name,
                    description,
                    plan_version_id,
                    is_draft_version,
                    currency,
                    version,
                    net_terms,
                    billing_periods,
                }: PlanOverviewBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    description: description.map(|v| v.into()),
                    plan_version_id,
                    is_draft_version,
                    currency: currency.into(),
                    version,
                    net_terms,
                    billing_periods: billing_periods.map(|v| v).collect(),
                }
            }
        }
        pub struct PlanOverviewQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> PlanOverviewBorrowed,
            mapper: fn(PlanOverviewBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> PlanOverviewQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(PlanOverviewBorrowed) -> R,
            ) -> PlanOverviewQuery<'a, C, R, N> {
                PlanOverviewQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn create_plan() -> CreatePlanStmt {
            CreatePlanStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO
  plan(
    id,
    name,
    external_id,
    description,
    tenant_id,
    created_by,
    status,
    plan_type,
    product_family_id
  )
VALUES
  (
    $1,
    $2,
    $3,
    $4,
    $5,
    $6,
    $7,
    $8,
    (
      SELECT
        id
      FROM
        product_family
      WHERE
        external_id = $9
    )
  ) RETURNING id,
  name,
  external_id,
  description,
  status,
  plan_type",
            ))
        }
        pub struct CreatePlanStmt(cornucopia_async::private::Stmt);
        impl CreatePlanStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                external_id: &'a T2,
                description: &'a Option<T3>,
                tenant_id: &'a uuid::Uuid,
                created_by: &'a uuid::Uuid,
                status: &'a super::super::types::public::PlanStatusEnum,
                plan_type: &'a super::super::types::public::PlanTypeEnum,
                product_family_external_id: &'a T4,
            ) -> PlanQuery<'a, C, Plan, 9> {
                PlanQuery {
                    client,
                    params: [
                        id,
                        name,
                        external_id,
                        description,
                        tenant_id,
                        created_by,
                        status,
                        plan_type,
                        product_family_external_id,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| PlanBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        external_id: row.get(2),
                        description: row.get(3),
                        status: row.get(4),
                        plan_type: row.get(5),
                    },
                    mapper: |it| <Plan>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                CreatePlanParams<T1, T2, T3, T4>,
                PlanQuery<'a, C, Plan, 9>,
                C,
            > for CreatePlanStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreatePlanParams<T1, T2, T3, T4>,
            ) -> PlanQuery<'a, C, Plan, 9> {
                self.bind(
                    client,
                    &params.id,
                    &params.name,
                    &params.external_id,
                    &params.description,
                    &params.tenant_id,
                    &params.created_by,
                    &params.status,
                    &params.plan_type,
                    &params.product_family_external_id,
                )
            }
        }
        pub fn get_plan_version_by_id() -> GetPlanVersionByIdStmt {
            GetPlanVersionByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT
  id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods
FROM
  plan_version
WHERE
  id = $1
  AND tenant_id = $2",
            ))
        }
        pub struct GetPlanVersionByIdStmt(cornucopia_async::private::Stmt);
        impl GetPlanVersionByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                plan_version_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 2> {
                PlanVersionQuery {
                    client,
                    params: [plan_version_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| PlanVersionBorrowed {
                        id: row.get(0),
                        is_draft_version: row.get(1),
                        plan_id: row.get(2),
                        version: row.get(3),
                        created_by: row.get(4),
                        trial_duration_days: row.get(5),
                        trial_fallback_plan_id: row.get(6),
                        tenant_id: row.get(7),
                        period_start_day: row.get(8),
                        net_terms: row.get(9),
                        currency: row.get(10),
                        billing_cycles: row.get(11),
                        billing_periods: row.get(12),
                    },
                    mapper: |it| <PlanVersion>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetPlanVersionByIdParams,
                PlanVersionQuery<'a, C, PlanVersion, 2>,
                C,
            > for GetPlanVersionByIdStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetPlanVersionByIdParams,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 2> {
                self.bind(client, &params.plan_version_id, &params.tenant_id)
            }
        }
        pub fn create_plan_version() -> CreatePlanVersionStmt {
            CreatePlanVersionStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO
  plan_version (
    id,
    is_draft_version,
    plan_id,
    version,
    created_by,
    trial_duration_days,
    trial_fallback_plan_id,
    tenant_id,
    period_start_day,
    net_terms,
    currency,
    billing_cycles,
    billing_periods
  )
VALUES
  (
    $1,
    TRUE,
    $2,
    $3,
    $4,
    $5,
    $6,
    $7,
    $8,
    COALESCE($9, 0),
    COALESCE($10, (SELECT currency FROM tenant WHERE id = $7)),
    $11,
    $12
  ) RETURNING id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods",
            ))
        }
        pub struct CreatePlanVersionStmt(cornucopia_async::private::Stmt);
        impl CreatePlanVersionStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::ArraySql<Item = super::super::types::public::BillingPeriodEnum>,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                plan_id: &'a uuid::Uuid,
                version: &'a i32,
                created_by: &'a uuid::Uuid,
                trial_duration_days: &'a Option<i32>,
                trial_fallback_plan_id: &'a Option<uuid::Uuid>,
                tenant_id: &'a uuid::Uuid,
                period_start_day: &'a Option<i16>,
                net_terms: &'a Option<i32>,
                currency: &'a Option<T1>,
                billing_cycles: &'a Option<i32>,
                billing_periods: &'a T2,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 12> {
                PlanVersionQuery {
                    client,
                    params: [
                        id,
                        plan_id,
                        version,
                        created_by,
                        trial_duration_days,
                        trial_fallback_plan_id,
                        tenant_id,
                        period_start_day,
                        net_terms,
                        currency,
                        billing_cycles,
                        billing_periods,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| PlanVersionBorrowed {
                        id: row.get(0),
                        is_draft_version: row.get(1),
                        plan_id: row.get(2),
                        version: row.get(3),
                        created_by: row.get(4),
                        trial_duration_days: row.get(5),
                        trial_fallback_plan_id: row.get(6),
                        tenant_id: row.get(7),
                        period_start_day: row.get(8),
                        net_terms: row.get(9),
                        currency: row.get(10),
                        billing_cycles: row.get(11),
                        billing_periods: row.get(12),
                    },
                    mapper: |it| <PlanVersion>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::ArraySql<Item = super::super::types::public::BillingPeriodEnum>,
            >
            cornucopia_async::Params<
                'a,
                CreatePlanVersionParams<T1, T2>,
                PlanVersionQuery<'a, C, PlanVersion, 12>,
                C,
            > for CreatePlanVersionStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreatePlanVersionParams<T1, T2>,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 12> {
                self.bind(
                    client,
                    &params.id,
                    &params.plan_id,
                    &params.version,
                    &params.created_by,
                    &params.trial_duration_days,
                    &params.trial_fallback_plan_id,
                    &params.tenant_id,
                    &params.period_start_day,
                    &params.net_terms,
                    &params.currency,
                    &params.billing_cycles,
                    &params.billing_periods,
                )
            }
        }
        pub fn copy_version_to_draft() -> CopyVersionToDraftStmt {
            CopyVersionToDraftStmt(cornucopia_async::private::Stmt::new(
                "WITH original_plan_version AS (
  SELECT
    *
  FROM
    plan_version
  WHERE
    id = $1
    AND tenant_id = $2
),
new_plan_version AS (
  -- Create a new draft version of the plan
  INSERT INTO
    plan_version (
      id,
      is_draft_version,
      plan_id,
      version,
      created_by,
      trial_duration_days,
      trial_fallback_plan_id,
      tenant_id,
      period_start_day,
      net_terms,
      currency,
      billing_cycles,
      billing_periods
    )
  SELECT
    $3,
    TRUE,
    plan_id,
    version + 1,
    $4,
    trial_duration_days,
    trial_fallback_plan_id,
    tenant_id,
    period_start_day,
    net_terms,
    currency,
    billing_cycles,
    billing_periods
  FROM
    original_plan_version RETURNING id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods
),
duplicate_price_component AS (
  -- Duplicate 'price_component' records
  INSERT INTO
    price_component (id, name, fee, plan_version_id, product_item_id)
  SELECT
    gen_random_uuid(),
    name,
    fee,
    new_plan_version.id,
    product_item_id
  FROM
    price_component,
    new_plan_version
  WHERE
    plan_version_id = $1
),
duplicate_schedule AS (
  INSERT INTO
    schedule (
      id,
      billing_period,
      plan_version_id,
      ramps
  )
  SELECT
      gen_random_uuid(),
      billing_period,
      new_plan_version.id,
      ramps
  FROM
      schedule,
      new_plan_version
  WHERE
      plan_version_id = $1
)
SELECT
  *
FROM
  new_plan_version",
            ))
        }
        pub struct CopyVersionToDraftStmt(cornucopia_async::private::Stmt);
        impl CopyVersionToDraftStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                original_plan_version_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
                new_plan_version_id: &'a uuid::Uuid,
                created_by: &'a uuid::Uuid,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 4> {
                PlanVersionQuery {
                    client,
                    params: [
                        original_plan_version_id,
                        tenant_id,
                        new_plan_version_id,
                        created_by,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| PlanVersionBorrowed {
                        id: row.get(0),
                        is_draft_version: row.get(1),
                        plan_id: row.get(2),
                        version: row.get(3),
                        created_by: row.get(4),
                        trial_duration_days: row.get(5),
                        trial_fallback_plan_id: row.get(6),
                        tenant_id: row.get(7),
                        period_start_day: row.get(8),
                        net_terms: row.get(9),
                        currency: row.get(10),
                        billing_cycles: row.get(11),
                        billing_periods: row.get(12),
                    },
                    mapper: |it| <PlanVersion>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                CopyVersionToDraftParams,
                PlanVersionQuery<'a, C, PlanVersion, 4>,
                C,
            > for CopyVersionToDraftStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CopyVersionToDraftParams,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 4> {
                self.bind(
                    client,
                    &params.original_plan_version_id,
                    &params.tenant_id,
                    &params.new_plan_version_id,
                    &params.created_by,
                )
            }
        }
        pub fn publish_plan_version() -> PublishPlanVersionStmt {
            PublishPlanVersionStmt(cornucopia_async::private::Stmt::new(
                "UPDATE
  plan_version
SET
  is_draft_version = FALSE
WHERE
  id = $1
  AND tenant_id = $2 RETURNING id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods",
            ))
        }
        pub struct PublishPlanVersionStmt(cornucopia_async::private::Stmt);
        impl PublishPlanVersionStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                plan_version_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 2> {
                PlanVersionQuery {
                    client,
                    params: [plan_version_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| PlanVersionBorrowed {
                        id: row.get(0),
                        is_draft_version: row.get(1),
                        plan_id: row.get(2),
                        version: row.get(3),
                        created_by: row.get(4),
                        trial_duration_days: row.get(5),
                        trial_fallback_plan_id: row.get(6),
                        tenant_id: row.get(7),
                        period_start_day: row.get(8),
                        net_terms: row.get(9),
                        currency: row.get(10),
                        billing_cycles: row.get(11),
                        billing_periods: row.get(12),
                    },
                    mapper: |it| <PlanVersion>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                PublishPlanVersionParams,
                PlanVersionQuery<'a, C, PlanVersion, 2>,
                C,
            > for PublishPlanVersionStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a PublishPlanVersionParams,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 2> {
                self.bind(client, &params.plan_version_id, &params.tenant_id)
            }
        }
        pub fn activate_plan() -> ActivatePlanStmt {
            ActivatePlanStmt(cornucopia_async::private::Stmt::new(
                "UPDATE
  plan
SET
    status = 'ACTIVE'
WHERE
    id = $1
    AND tenant_id = $2",
            ))
        }
        pub struct ActivatePlanStmt(cornucopia_async::private::Stmt);
        impl ActivatePlanStmt {
            pub async fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                plan_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[plan_id, tenant_id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync>
            cornucopia_async::Params<
                'a,
                ActivatePlanParams,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for ActivatePlanStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ActivatePlanParams,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.plan_id, &params.tenant_id))
            }
        }
        pub fn find_plan_by_external_id() -> FindPlanByExternalIdStmt {
            FindPlanByExternalIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT
  id,
  name,
  external_id,
  description,
  status,
  plan_type
FROM
  plan
WHERE
  tenant_id = $1
  AND external_id = $2",
            ))
        }
        pub struct FindPlanByExternalIdStmt(cornucopia_async::private::Stmt);
        impl FindPlanByExternalIdStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                external_id: &'a T1,
            ) -> PlanQuery<'a, C, Plan, 2> {
                PlanQuery {
                    client,
                    params: [tenant_id, external_id],
                    stmt: &mut self.0,
                    extractor: |row| PlanBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        external_id: row.get(2),
                        description: row.get(3),
                        status: row.get(4),
                        plan_type: row.get(5),
                    },
                    mapper: |it| <Plan>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                FindPlanByExternalIdParams<T1>,
                PlanQuery<'a, C, Plan, 2>,
                C,
            > for FindPlanByExternalIdStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a FindPlanByExternalIdParams<T1>,
            ) -> PlanQuery<'a, C, Plan, 2> {
                self.bind(client, &params.tenant_id, &params.external_id)
            }
        }
        pub fn list_plans() -> ListPlansStmt {
            ListPlansStmt(cornucopia_async::private::Stmt::new(
                "SELECT
  plan.id,
  plan.name,
  plan.external_id,
  plan.description,
  plan.status,
  plan.plan_type,
  COUNT(*) OVER() AS total_count
FROM
  plan
  JOIN product_family ON plan.product_family_id = product_family.id
WHERE
  plan.tenant_id = $1
  AND (
    $2 :: TEXT IS NULL
    OR to_tsvector('english', plan.name || ' ' || plan.external_id) @@ to_tsquery('english', $2)
  )
  AND product_family.external_id = $3
ORDER BY
  CASE
    WHEN $4 = 'DATE_DESC' THEN plan.id
  END DESC,
  CASE
    WHEN $4 = 'DATE_ASC' THEN plan.id
  END ASC,
  CASE
    WHEN $4 = 'NAME_DESC' THEN plan.name
  END DESC,
  CASE
    WHEN $4 = 'NAME_ASC' THEN plan.name
  END ASC
LIMIT
  $5 OFFSET $6",
            ))
        }
        pub struct ListPlansStmt(cornucopia_async::private::Stmt);
        impl ListPlansStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                search: &'a Option<T1>,
                product_family_external_id: &'a T2,
                order_by: &'a T3,
                limit: &'a i64,
                offset: &'a i64,
            ) -> ListPlanQuery<'a, C, ListPlan, 6> {
                ListPlanQuery {
                    client,
                    params: [
                        tenant_id,
                        search,
                        product_family_external_id,
                        order_by,
                        limit,
                        offset,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| ListPlanBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        external_id: row.get(2),
                        description: row.get(3),
                        status: row.get(4),
                        plan_type: row.get(5),
                        total_count: row.get(6),
                    },
                    mapper: |it| <ListPlan>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                ListPlansParams<T1, T2, T3>,
                ListPlanQuery<'a, C, ListPlan, 6>,
                C,
            > for ListPlansStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListPlansParams<T1, T2, T3>,
            ) -> ListPlanQuery<'a, C, ListPlan, 6> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.search,
                    &params.product_family_external_id,
                    &params.order_by,
                    &params.limit,
                    &params.offset,
                )
            }
        }
        pub fn list_plans_versions() -> ListPlansVersionsStmt {
            ListPlansVersionsStmt(cornucopia_async::private::Stmt::new(
                "SELECT
  id,
  is_draft_version,
  plan_id,
  version,
  created_by,
  trial_duration_days,
  trial_fallback_plan_id,
  tenant_id,
  period_start_day,
  net_terms,
  currency,
  billing_cycles,
  billing_periods,
  COUNT(*) OVER() AS total_count
FROM
  plan_version
WHERE
  plan_version.tenant_id = $1
  AND plan_version.plan_id = $2
ORDER BY
  plan_version.version DESC
LIMIT
  $3 OFFSET $4",
            ))
        }
        pub struct ListPlansVersionsStmt(cornucopia_async::private::Stmt);
        impl ListPlansVersionsStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                plan_id: &'a uuid::Uuid,
                limit: &'a i64,
                offset: &'a i64,
            ) -> ListPlanVersionQuery<'a, C, ListPlanVersion, 4> {
                ListPlanVersionQuery {
                    client,
                    params: [tenant_id, plan_id, limit, offset],
                    stmt: &mut self.0,
                    extractor: |row| ListPlanVersionBorrowed {
                        id: row.get(0),
                        is_draft_version: row.get(1),
                        plan_id: row.get(2),
                        version: row.get(3),
                        created_by: row.get(4),
                        trial_duration_days: row.get(5),
                        trial_fallback_plan_id: row.get(6),
                        tenant_id: row.get(7),
                        period_start_day: row.get(8),
                        net_terms: row.get(9),
                        currency: row.get(10),
                        billing_cycles: row.get(11),
                        billing_periods: row.get(12),
                        total_count: row.get(13),
                    },
                    mapper: |it| <ListPlanVersion>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                ListPlansVersionsParams,
                ListPlanVersionQuery<'a, C, ListPlanVersion, 4>,
                C,
            > for ListPlansVersionsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListPlansVersionsParams,
            ) -> ListPlanVersionQuery<'a, C, ListPlanVersion, 4> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.plan_id,
                    &params.limit,
                    &params.offset,
                )
            }
        }
        pub fn last_plan_version() -> LastPlanVersionStmt {
            LastPlanVersionStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    id,
    is_draft_version,
    plan_id,
    version,
    created_by,
    trial_duration_days,
    trial_fallback_plan_id,
    tenant_id,
    period_start_day,
    net_terms,
    currency,
    billing_cycles,
    billing_periods
FROM
    plan_version
WHERE
        plan_version.tenant_id = $1
  AND plan_version.plan_id = $2
  -- only if is_draft is not null, check is_draft_version
    AND (
        -- below does not work, we need to cast to bool
        $3::bool IS NULL
        OR plan_version.is_draft_version = $3
    )
ORDER BY
    plan_version.version DESC
    LIMIT
  1",
            ))
        }
        pub struct LastPlanVersionStmt(cornucopia_async::private::Stmt);
        impl LastPlanVersionStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                plan_id: &'a uuid::Uuid,
                is_draft: &'a Option<bool>,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 3> {
                PlanVersionQuery {
                    client,
                    params: [tenant_id, plan_id, is_draft],
                    stmt: &mut self.0,
                    extractor: |row| PlanVersionBorrowed {
                        id: row.get(0),
                        is_draft_version: row.get(1),
                        plan_id: row.get(2),
                        version: row.get(3),
                        created_by: row.get(4),
                        trial_duration_days: row.get(5),
                        trial_fallback_plan_id: row.get(6),
                        tenant_id: row.get(7),
                        period_start_day: row.get(8),
                        net_terms: row.get(9),
                        currency: row.get(10),
                        billing_cycles: row.get(11),
                        billing_periods: row.get(12),
                    },
                    mapper: |it| <PlanVersion>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                LastPlanVersionParams,
                PlanVersionQuery<'a, C, PlanVersion, 3>,
                C,
            > for LastPlanVersionStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a LastPlanVersionParams,
            ) -> PlanVersionQuery<'a, C, PlanVersion, 3> {
                self.bind(client, &params.tenant_id, &params.plan_id, &params.is_draft)
            }
        }
        pub fn delete_draft_plan_version() -> DeleteDraftPlanVersionStmt {
            DeleteDraftPlanVersionStmt(cornucopia_async::private::Stmt::new(
                "DELETE
FROM
  plan_version
WHERE
  id = $1
  AND tenant_id = $2
  AND is_draft_version = TRUE",
            ))
        }
        pub struct DeleteDraftPlanVersionStmt(cornucopia_async::private::Stmt);
        impl DeleteDraftPlanVersionStmt {
            pub async fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                plan_version_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[plan_version_id, tenant_id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync>
            cornucopia_async::Params<
                'a,
                DeleteDraftPlanVersionParams,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for DeleteDraftPlanVersionStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a DeleteDraftPlanVersionParams,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.plan_version_id, &params.tenant_id))
            }
        }
        pub fn delete_all_draft_versions_of_same_plan() -> DeleteAllDraftVersionsOfSamePlanStmt {
            DeleteAllDraftVersionsOfSamePlanStmt(cornucopia_async::private::Stmt::new(
                "DELETE
FROM
    plan_version pv1
USING
    plan_version pv2
WHERE
    pv1.plan_id = pv2.plan_id
  AND pv1.tenant_id = pv2.tenant_id
  AND pv1.is_draft_version = TRUE
  AND pv2.id = $1
  AND pv2.tenant_id = $2",
            ))
        }
        pub struct DeleteAllDraftVersionsOfSamePlanStmt(cornucopia_async::private::Stmt);
        impl DeleteAllDraftVersionsOfSamePlanStmt {
            pub async fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                plan_version_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[plan_version_id, tenant_id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync>
            cornucopia_async::Params<
                'a,
                DeleteAllDraftVersionsOfSamePlanParams,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for DeleteAllDraftVersionsOfSamePlanStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a DeleteAllDraftVersionsOfSamePlanParams,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.plan_version_id, &params.tenant_id))
            }
        }
        pub fn update_plan_version_overview() -> UpdatePlanVersionOverviewStmt {
            UpdatePlanVersionOverviewStmt(cornucopia_async::private::Stmt::new(
                "UPDATE plan_version
SET
    currency = $1,
    net_terms = $2,
    billing_periods = $3
WHERE
        tenant_id = $4
    AND id = $5
    AND is_draft_version = TRUE",
            ))
        }
        pub struct UpdatePlanVersionOverviewStmt(cornucopia_async::private::Stmt);
        impl UpdatePlanVersionOverviewStmt {
            pub async fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::ArraySql<Item = super::super::types::public::BillingPeriodEnum>,
            >(
                &'a mut self,
                client: &'a C,
                currency: &'a T1,
                net_terms: &'a i32,
                billing_periods: &'a T2,
                tenant_id: &'a uuid::Uuid,
                plan_version_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client
                    .execute(
                        stmt,
                        &[
                            currency,
                            net_terms,
                            billing_periods,
                            tenant_id,
                            plan_version_id,
                        ],
                    )
                    .await
            }
        }
        impl<
                'a,
                C: GenericClient + Send + Sync,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::ArraySql<Item = super::super::types::public::BillingPeriodEnum>,
            >
            cornucopia_async::Params<
                'a,
                UpdatePlanVersionOverviewParams<T1, T2>,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for UpdatePlanVersionOverviewStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpdatePlanVersionOverviewParams<T1, T2>,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(
                    client,
                    &params.currency,
                    &params.net_terms,
                    &params.billing_periods,
                    &params.tenant_id,
                    &params.plan_version_id,
                ))
            }
        }
        pub fn update_plan_overview() -> UpdatePlanOverviewStmt {
            UpdatePlanOverviewStmt(cornucopia_async::private::Stmt::new(
                "UPDATE plan
SET
    name = $1,
    description = $2
WHERE
    tenant_id = $3
  AND id = $4",
            ))
        }
        pub struct UpdatePlanOverviewStmt(cornucopia_async::private::Stmt);
        impl UpdatePlanOverviewStmt {
            pub async fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                name: &'a T1,
                description: &'a Option<T2>,
                tenant_id: &'a uuid::Uuid,
                plan_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client
                    .execute(stmt, &[name, description, tenant_id, plan_id])
                    .await
            }
        }
        impl<
                'a,
                C: GenericClient + Send + Sync,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                UpdatePlanOverviewParams<T1, T2>,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for UpdatePlanOverviewStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpdatePlanOverviewParams<T1, T2>,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(
                    client,
                    &params.name,
                    &params.description,
                    &params.tenant_id,
                    &params.plan_id,
                ))
            }
        }
        pub fn get_plan_overview_by_external_id() -> GetPlanOverviewByExternalIdStmt {
            GetPlanOverviewByExternalIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    p.id,
    p.name,
    p.description,
    pv.id as plan_version_id,
    pv.is_draft_version,
    pv.currency,
    pv.version,
    pv.net_terms,
    pv.billing_periods
FROM
    plan_version pv
JOIN
    plan p ON pv.plan_id = p.id
WHERE
    p.external_id = $1
  AND p.tenant_id = $2
ORDER BY pv.version DESC
LIMIT 1",
            ))
        }
        pub struct GetPlanOverviewByExternalIdStmt(cornucopia_async::private::Stmt);
        impl GetPlanOverviewByExternalIdStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                external_id: &'a T1,
                tenant_id: &'a uuid::Uuid,
            ) -> PlanOverviewQuery<'a, C, PlanOverview, 2> {
                PlanOverviewQuery {
                    client,
                    params: [external_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| PlanOverviewBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        description: row.get(2),
                        plan_version_id: row.get(3),
                        is_draft_version: row.get(4),
                        currency: row.get(5),
                        version: row.get(6),
                        net_terms: row.get(7),
                        billing_periods: row.get(8),
                    },
                    mapper: |it| <PlanOverview>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                GetPlanOverviewByExternalIdParams<T1>,
                PlanOverviewQuery<'a, C, PlanOverview, 2>,
                C,
            > for GetPlanOverviewByExternalIdStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetPlanOverviewByExternalIdParams<T1>,
            ) -> PlanOverviewQuery<'a, C, PlanOverview, 2> {
                self.bind(client, &params.external_id, &params.tenant_id)
            }
        }
        pub fn get_plan_overview_by_id() -> GetPlanOverviewByIdStmt {
            GetPlanOverviewByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    p.id,
    p.name,
    p.description,
    pv.id as plan_version_id,
    pv.is_draft_version,
    pv.version,
    pv.currency,
    pv.net_terms,
    pv.billing_periods
FROM
    plan_version pv
        JOIN
    plan p ON pv.plan_id = p.id
WHERE
        pv.id = $1
  AND p.tenant_id = $2",
            ))
        }
        pub struct GetPlanOverviewByIdStmt(cornucopia_async::private::Stmt);
        impl GetPlanOverviewByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                plan_version_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> PlanOverviewQuery<'a, C, PlanOverview, 2> {
                PlanOverviewQuery {
                    client,
                    params: [plan_version_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| PlanOverviewBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        description: row.get(2),
                        plan_version_id: row.get(3),
                        is_draft_version: row.get(4),
                        currency: row.get(6),
                        version: row.get(5),
                        net_terms: row.get(7),
                        billing_periods: row.get(8),
                    },
                    mapper: |it| <PlanOverview>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetPlanOverviewByIdParams,
                PlanOverviewQuery<'a, C, PlanOverview, 2>,
                C,
            > for GetPlanOverviewByIdStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetPlanOverviewByIdParams,
            ) -> PlanOverviewQuery<'a, C, PlanOverview, 2> {
                self.bind(client, &params.plan_version_id, &params.tenant_id)
            }
        }
        pub fn delete_plan_if_no_versions() -> DeletePlanIfNoVersionsStmt {
            DeletePlanIfNoVersionsStmt(cornucopia_async::private::Stmt::new(
                "DELETE
FROM
    plan
WHERE
    id = $1
  AND tenant_id = $2
  AND NOT EXISTS (
    SELECT
        1
    FROM
        plan_version
    WHERE
            plan_version.plan_id = plan.id
        AND plan_version.tenant_id = plan.tenant_id
  )",
            ))
        }
        pub struct DeletePlanIfNoVersionsStmt(cornucopia_async::private::Stmt);
        impl DeletePlanIfNoVersionsStmt {
            pub async fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                plan_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[plan_id, tenant_id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync>
            cornucopia_async::Params<
                'a,
                DeletePlanIfNoVersionsParams,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for DeletePlanIfNoVersionsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a DeletePlanIfNoVersionsParams,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.plan_id, &params.tenant_id))
            }
        }
    }
    pub mod price_components {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct UpsertPriceComponentParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::JsonSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub fee: T2,
            pub plan_version_id: uuid::Uuid,
            pub product_item_id: Option<uuid::Uuid>,
            pub billable_metric_id: Option<uuid::Uuid>,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct ListPriceComponentsParams {
            pub plan_version_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetPriceComponentParams {
            pub component_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct DeletePriceComponentParams {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct PriceComponent {
            pub id: uuid::Uuid,
            pub name: String,
            pub fee: serde_json::Value,
            pub product_item_id: Option<uuid::Uuid>,
            pub product_item_name: Option<String>,
        }
        pub struct PriceComponentBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub fee: postgres_types::Json<&'a serde_json::value::RawValue>,
            pub product_item_id: Option<uuid::Uuid>,
            pub product_item_name: Option<&'a str>,
        }
        impl<'a> From<PriceComponentBorrowed<'a>> for PriceComponent {
            fn from(
                PriceComponentBorrowed {
                    id,
                    name,
                    fee,
                    product_item_id,
                    product_item_name,
                }: PriceComponentBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    fee: serde_json::from_str(fee.0.get()).unwrap(),
                    product_item_id,
                    product_item_name: product_item_name.map(|v| v.into()),
                }
            }
        }
        pub struct PriceComponentQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> PriceComponentBorrowed,
            mapper: fn(PriceComponentBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> PriceComponentQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(PriceComponentBorrowed) -> R,
            ) -> PriceComponentQuery<'a, C, R, N> {
                PriceComponentQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct PriceComponentWithMetric {
            pub id: uuid::Uuid,
            pub name: String,
            pub fee: serde_json::Value,
            pub product_item_id: Option<uuid::Uuid>,
            pub product_item_name: Option<String>,
            pub billable_metric_id: Option<uuid::Uuid>,
        }
        pub struct PriceComponentWithMetricBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub fee: postgres_types::Json<&'a serde_json::value::RawValue>,
            pub product_item_id: Option<uuid::Uuid>,
            pub product_item_name: Option<&'a str>,
            pub billable_metric_id: Option<uuid::Uuid>,
        }
        impl<'a> From<PriceComponentWithMetricBorrowed<'a>> for PriceComponentWithMetric {
            fn from(
                PriceComponentWithMetricBorrowed {
                    id,
                    name,
                    fee,
                    product_item_id,
                    product_item_name,
                    billable_metric_id,
                }: PriceComponentWithMetricBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    fee: serde_json::from_str(fee.0.get()).unwrap(),
                    product_item_id,
                    product_item_name: product_item_name.map(|v| v.into()),
                    billable_metric_id,
                }
            }
        }
        pub struct PriceComponentWithMetricQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> PriceComponentWithMetricBorrowed,
            mapper: fn(PriceComponentWithMetricBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> PriceComponentWithMetricQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(PriceComponentWithMetricBorrowed) -> R,
            ) -> PriceComponentWithMetricQuery<'a, C, R, N> {
                PriceComponentWithMetricQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn upsert_price_component() -> UpsertPriceComponentStmt {
            UpsertPriceComponentStmt(cornucopia_async :: private :: Stmt :: new("INSERT INTO price_component (id, name, fee, plan_version_id, product_item_id, billable_metric_id)
SELECT $1,
       $2,
       $3,
       $4,
       $5,
       $6
FROM plan_version
WHERE plan_version.id = $4
  AND plan_version.tenant_id = $7
  AND plan_version.is_draft_version = true
ON CONFLICT (id) DO UPDATE SET name               = EXCLUDED.name,
                               fee                = EXCLUDED.fee,
                               product_item_id    = EXCLUDED.product_item_id,
                               billable_metric_id = EXCLUDED.billable_metric_id"))
        }
        pub struct UpsertPriceComponentStmt(cornucopia_async::private::Stmt);
        impl UpsertPriceComponentStmt {
            pub async fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::JsonSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                fee: &'a T2,
                plan_version_id: &'a uuid::Uuid,
                product_item_id: &'a Option<uuid::Uuid>,
                billable_metric_id: &'a Option<uuid::Uuid>,
                tenant_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client
                    .execute(
                        stmt,
                        &[
                            id,
                            name,
                            fee,
                            plan_version_id,
                            product_item_id,
                            billable_metric_id,
                            tenant_id,
                        ],
                    )
                    .await
            }
        }
        impl<
                'a,
                C: GenericClient + Send + Sync,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::JsonSql,
            >
            cornucopia_async::Params<
                'a,
                UpsertPriceComponentParams<T1, T2>,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for UpsertPriceComponentStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpsertPriceComponentParams<T1, T2>,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(
                    client,
                    &params.id,
                    &params.name,
                    &params.fee,
                    &params.plan_version_id,
                    &params.product_item_id,
                    &params.billable_metric_id,
                    &params.tenant_id,
                ))
            }
        }
        pub fn list_price_components() -> ListPriceComponentsStmt {
            ListPriceComponentsStmt(cornucopia_async::private::Stmt::new(
                "SELECT pc.id, pc.name, pc.fee, pc.product_item_id, pi.name as product_item_name
FROM price_component pc
         JOIN plan_version pv ON pc.plan_version_id = pv.id
         LEFT JOIN product pi ON pc.product_item_id = pi.id
WHERE pv.id = $1
  AND pv.tenant_id = $2",
            ))
        }
        pub struct ListPriceComponentsStmt(cornucopia_async::private::Stmt);
        impl ListPriceComponentsStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                plan_version_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> PriceComponentQuery<'a, C, PriceComponent, 2> {
                PriceComponentQuery {
                    client,
                    params: [plan_version_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| PriceComponentBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        fee: row.get(2),
                        product_item_id: row.get(3),
                        product_item_name: row.get(4),
                    },
                    mapper: |it| <PriceComponent>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                ListPriceComponentsParams,
                PriceComponentQuery<'a, C, PriceComponent, 2>,
                C,
            > for ListPriceComponentsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListPriceComponentsParams,
            ) -> PriceComponentQuery<'a, C, PriceComponent, 2> {
                self.bind(client, &params.plan_version_id, &params.tenant_id)
            }
        }
        pub fn get_price_component() -> GetPriceComponentStmt {
            GetPriceComponentStmt(cornucopia_async::private::Stmt::new(
                "SELECT pc.id, pc.name, pc.fee, pc.product_item_id, pi.name as product_item_name
FROM price_component pc
         JOIN plan_version pv ON pc.plan_version_id = pv.id
         LEFT JOIN product pi ON pc.product_item_id = pi.id
WHERE pc.id = $1
  AND pv.tenant_id = $2",
            ))
        }
        pub struct GetPriceComponentStmt(cornucopia_async::private::Stmt);
        impl GetPriceComponentStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                component_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> PriceComponentQuery<'a, C, PriceComponent, 2> {
                PriceComponentQuery {
                    client,
                    params: [component_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| PriceComponentBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        fee: row.get(2),
                        product_item_id: row.get(3),
                        product_item_name: row.get(4),
                    },
                    mapper: |it| <PriceComponent>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetPriceComponentParams,
                PriceComponentQuery<'a, C, PriceComponent, 2>,
                C,
            > for GetPriceComponentStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetPriceComponentParams,
            ) -> PriceComponentQuery<'a, C, PriceComponent, 2> {
                self.bind(client, &params.component_id, &params.tenant_id)
            }
        }
        pub fn list_price_components_by_subscription() -> ListPriceComponentsBySubscriptionStmt {
            ListPriceComponentsBySubscriptionStmt(cornucopia_async::private::Stmt::new(
                "SELECT pc.id,
       pc.name,
       pc.fee,
       pc.product_item_id,
       pi.name as product_item_name,
       bm.id   as billable_metric_id
FROM price_component pc
         JOIN subscription ss ON pc.plan_version_id = ss.plan_version_id
         JOIN plan_version pv ON pc.plan_version_id = pv.id
         LEFT JOIN product pi ON pc.product_item_id = pi.id
         LEFT JOIN billable_metric bm ON pc.billable_metric_id = bm.id
WHERE ss.id = $1",
            ))
        }
        pub struct ListPriceComponentsBySubscriptionStmt(cornucopia_async::private::Stmt);
        impl ListPriceComponentsBySubscriptionStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                subscription_id: &'a uuid::Uuid,
            ) -> PriceComponentWithMetricQuery<'a, C, PriceComponentWithMetric, 1> {
                PriceComponentWithMetricQuery {
                    client,
                    params: [subscription_id],
                    stmt: &mut self.0,
                    extractor: |row| PriceComponentWithMetricBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        fee: row.get(2),
                        product_item_id: row.get(3),
                        product_item_name: row.get(4),
                        billable_metric_id: row.get(5),
                    },
                    mapper: |it| <PriceComponentWithMetric>::from(it),
                }
            }
        }
        pub fn delete_price_component() -> DeletePriceComponentStmt {
            DeletePriceComponentStmt(cornucopia_async::private::Stmt::new(
                "DELETE
FROM price_component pc
    USING plan_version pv
WHERE pc.id = $1
  AND pc.plan_version_id = pv.id
  AND pv.tenant_id = $2
  AND pv.is_draft_version = true",
            ))
        }
        pub struct DeletePriceComponentStmt(cornucopia_async::private::Stmt);
        impl DeletePriceComponentStmt {
            pub async fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[id, tenant_id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync>
            cornucopia_async::Params<
                'a,
                DeletePriceComponentParams,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for DeletePriceComponentStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a DeletePriceComponentParams,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.id, &params.tenant_id))
            }
        }
    }
    pub mod products {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateProductFamilyParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub external_id: T2,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct GetProductFamilyByExternalIdParams<T1: cornucopia_async::StringSql> {
            pub tenant_id: uuid::Uuid,
            pub external_id: T1,
        }
        #[derive(Debug)]
        pub struct UpsertProductParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub description: Option<T2>,
            pub product_family_external_id: T3,
            pub tenant_id: uuid::Uuid,
            pub created_by: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct ListProductsParams<T1: cornucopia_async::StringSql> {
            pub tenant_id: uuid::Uuid,
            pub family_external_id: T1,
            pub limit: i64,
            pub offset: i64,
        }
        #[derive(Debug)]
        pub struct SearchProductsParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
        > {
            pub tenant_id: uuid::Uuid,
            pub family_external_id: T1,
            pub query: Option<T2>,
            pub limit: i64,
            pub offset: i64,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetProductDetailsParams {
            pub product_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ProductFamily {
            pub id: uuid::Uuid,
            pub name: String,
            pub external_id: String,
        }
        pub struct ProductFamilyBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub external_id: &'a str,
        }
        impl<'a> From<ProductFamilyBorrowed<'a>> for ProductFamily {
            fn from(
                ProductFamilyBorrowed {
                    id,
                    name,
                    external_id,
                }: ProductFamilyBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    external_id: external_id.into(),
                }
            }
        }
        pub struct ProductFamilyQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ProductFamilyBorrowed,
            mapper: fn(ProductFamilyBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ProductFamilyQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(ProductFamilyBorrowed) -> R,
            ) -> ProductFamilyQuery<'a, C, R, N> {
                ProductFamilyQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct Product {
            pub id: uuid::Uuid,
            pub name: String,
            pub description: Option<String>,
            pub created_at: time::PrimitiveDateTime,
        }
        pub struct ProductBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub description: Option<&'a str>,
            pub created_at: time::PrimitiveDateTime,
        }
        impl<'a> From<ProductBorrowed<'a>> for Product {
            fn from(
                ProductBorrowed {
                    id,
                    name,
                    description,
                    created_at,
                }: ProductBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    description: description.map(|v| v.into()),
                    created_at,
                }
            }
        }
        pub struct ProductQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ProductBorrowed,
            mapper: fn(ProductBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ProductQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(ProductBorrowed) -> R) -> ProductQuery<'a, C, R, N> {
                ProductQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ListProduct {
            pub id: uuid::Uuid,
            pub name: String,
            pub total_count: i64,
        }
        pub struct ListProductBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub total_count: i64,
        }
        impl<'a> From<ListProductBorrowed<'a>> for ListProduct {
            fn from(
                ListProductBorrowed {
                    id,
                    name,
                    total_count,
                }: ListProductBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    total_count,
                }
            }
        }
        pub struct ListProductQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ListProductBorrowed,
            mapper: fn(ListProductBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ListProductQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(ListProductBorrowed) -> R,
            ) -> ListProductQuery<'a, C, R, N> {
                ListProductQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn create_product_family() -> CreateProductFamilyStmt {
            CreateProductFamilyStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO
    product_family(id, name, external_id, tenant_id)
VALUES
    ($1, $2, $3, $4) RETURNING id,
    name,
    external_id",
            ))
        }
        pub struct CreateProductFamilyStmt(cornucopia_async::private::Stmt);
        impl CreateProductFamilyStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                external_id: &'a T2,
                tenant_id: &'a uuid::Uuid,
            ) -> ProductFamilyQuery<'a, C, ProductFamily, 4> {
                ProductFamilyQuery {
                    client,
                    params: [id, name, external_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| ProductFamilyBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        external_id: row.get(2),
                    },
                    mapper: |it| <ProductFamily>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                CreateProductFamilyParams<T1, T2>,
                ProductFamilyQuery<'a, C, ProductFamily, 4>,
                C,
            > for CreateProductFamilyStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateProductFamilyParams<T1, T2>,
            ) -> ProductFamilyQuery<'a, C, ProductFamily, 4> {
                self.bind(
                    client,
                    &params.id,
                    &params.name,
                    &params.external_id,
                    &params.tenant_id,
                )
            }
        }
        pub fn list_product_families() -> ListProductFamiliesStmt {
            ListProductFamiliesStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    id,
    name,
    external_id
FROM
    product_family
WHERE
    tenant_id = $1",
            ))
        }
        pub struct ListProductFamiliesStmt(cornucopia_async::private::Stmt);
        impl ListProductFamiliesStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> ProductFamilyQuery<'a, C, ProductFamily, 1> {
                ProductFamilyQuery {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| ProductFamilyBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        external_id: row.get(2),
                    },
                    mapper: |it| <ProductFamily>::from(it),
                }
            }
        }
        pub fn get_product_family_by_external_id() -> GetProductFamilyByExternalIdStmt {
            GetProductFamilyByExternalIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    id,
    name,
    external_id
FROM
    product_family
WHERE
    tenant_id = $1
    AND external_id = $2",
            ))
        }
        pub struct GetProductFamilyByExternalIdStmt(cornucopia_async::private::Stmt);
        impl GetProductFamilyByExternalIdStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                external_id: &'a T1,
            ) -> ProductFamilyQuery<'a, C, ProductFamily, 2> {
                ProductFamilyQuery {
                    client,
                    params: [tenant_id, external_id],
                    stmt: &mut self.0,
                    extractor: |row| ProductFamilyBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        external_id: row.get(2),
                    },
                    mapper: |it| <ProductFamily>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                GetProductFamilyByExternalIdParams<T1>,
                ProductFamilyQuery<'a, C, ProductFamily, 2>,
                C,
            > for GetProductFamilyByExternalIdStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetProductFamilyByExternalIdParams<T1>,
            ) -> ProductFamilyQuery<'a, C, ProductFamily, 2> {
                self.bind(client, &params.tenant_id, &params.external_id)
            }
        }
        pub fn upsert_product() -> UpsertProductStmt {
            UpsertProductStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO
    product (
        id,
        name,
        description,
        product_family_id,
        tenant_id,
        created_by
    )
VALUES
    (
        $1,
        $2,
        $3,
        (
            SELECT
                id
            FROM
                product_family
            WHERE
                external_id = $4
                AND tenant_id = $5
        ),
        $5,
        $6
    ) ON CONFLICT (id) DO
UPDATE
SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    created_by = EXCLUDED.created_by,
    product_family_id = EXCLUDED.product_family_id,
    tenant_id = EXCLUDED.tenant_id 
    RETURNING id, name, description,  created_at
    ",
            ))
        }
        pub struct UpsertProductStmt(cornucopia_async::private::Stmt);
        impl UpsertProductStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                description: &'a Option<T2>,
                product_family_external_id: &'a T3,
                tenant_id: &'a uuid::Uuid,
                created_by: &'a uuid::Uuid,
            ) -> ProductQuery<'a, C, Product, 6> {
                ProductQuery {
                    client,
                    params: [
                        id,
                        name,
                        description,
                        product_family_external_id,
                        tenant_id,
                        created_by,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| ProductBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        description: row.get(2),
                        created_at: row.get(3),
                    },
                    mapper: |it| <Product>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                UpsertProductParams<T1, T2, T3>,
                ProductQuery<'a, C, Product, 6>,
                C,
            > for UpsertProductStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpsertProductParams<T1, T2, T3>,
            ) -> ProductQuery<'a, C, Product, 6> {
                self.bind(
                    client,
                    &params.id,
                    &params.name,
                    &params.description,
                    &params.product_family_external_id,
                    &params.tenant_id,
                    &params.created_by,
                )
            }
        }
        pub fn list_products() -> ListProductsStmt {
            ListProductsStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    p.id,
    p.name,
    count(*) OVER() AS total_count
FROM
    product AS p
    JOIN product_family AS pf ON pf.id = p.product_family_id
WHERE
    p.tenant_id = $1
    AND pf.external_id = $2
LIMIT
    $3 OFFSET $4",
            ))
        }
        pub struct ListProductsStmt(cornucopia_async::private::Stmt);
        impl ListProductsStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                family_external_id: &'a T1,
                limit: &'a i64,
                offset: &'a i64,
            ) -> ListProductQuery<'a, C, ListProduct, 4> {
                ListProductQuery {
                    client,
                    params: [tenant_id, family_external_id, limit, offset],
                    stmt: &mut self.0,
                    extractor: |row| ListProductBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        total_count: row.get(2),
                    },
                    mapper: |it| <ListProduct>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                ListProductsParams<T1>,
                ListProductQuery<'a, C, ListProduct, 4>,
                C,
            > for ListProductsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListProductsParams<T1>,
            ) -> ListProductQuery<'a, C, ListProduct, 4> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.family_external_id,
                    &params.limit,
                    &params.offset,
                )
            }
        }
        pub fn search_products() -> SearchProductsStmt {
            SearchProductsStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    p.id,
    p.name,
    count(*) OVER() AS total_count
FROM
    product AS p
    JOIN product_family AS pf ON pf.id = p.product_family_id
WHERE
    p.tenant_id = $1
    AND pf.external_id = $2
    AND p.name ILIKE '%' || $3 || '%'
LIMIT
    $4 OFFSET $5",
            ))
        }
        pub struct SearchProductsStmt(cornucopia_async::private::Stmt);
        impl SearchProductsStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                family_external_id: &'a T1,
                query: &'a Option<T2>,
                limit: &'a i64,
                offset: &'a i64,
            ) -> ListProductQuery<'a, C, ListProduct, 5> {
                ListProductQuery {
                    client,
                    params: [tenant_id, family_external_id, query, limit, offset],
                    stmt: &mut self.0,
                    extractor: |row| ListProductBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        total_count: row.get(2),
                    },
                    mapper: |it| <ListProduct>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                SearchProductsParams<T1, T2>,
                ListProductQuery<'a, C, ListProduct, 5>,
                C,
            > for SearchProductsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a SearchProductsParams<T1, T2>,
            ) -> ListProductQuery<'a, C, ListProduct, 5> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.family_external_id,
                    &params.query,
                    &params.limit,
                    &params.offset,
                )
            }
        }
        pub fn get_product_details() -> GetProductDetailsStmt {
            GetProductDetailsStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    p.id,
    p.name,
    p.description,
    p.created_at
FROM
    product AS p
WHERE
    p.id = $1
    AND p.tenant_id = $2",
            ))
        }
        pub struct GetProductDetailsStmt(cornucopia_async::private::Stmt);
        impl GetProductDetailsStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                product_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> ProductQuery<'a, C, Product, 2> {
                ProductQuery {
                    client,
                    params: [product_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| ProductBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        description: row.get(2),
                        created_at: row.get(3),
                    },
                    mapper: |it| <Product>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetProductDetailsParams,
                ProductQuery<'a, C, Product, 2>,
                C,
            > for GetProductDetailsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetProductDetailsParams,
            ) -> ProductQuery<'a, C, Product, 2> {
                self.bind(client, &params.product_id, &params.tenant_id)
            }
        }
    }
    pub mod provider_configs {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Clone, Copy, Debug)]
        pub struct GetConfigByProviderAndEndpointParams {
            pub tenant_id: uuid::Uuid,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
        }
        #[derive(Debug)]
        pub struct CreateProviderConfigParams<
            T1: cornucopia_async::JsonSql,
            T2: cornucopia_async::JsonSql,
        > {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub enabled: bool,
            pub webhook_security: Option<T1>,
            pub api_security: Option<T2>,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ProviderConfig {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub enabled: bool,
            pub webhook_security: Option<serde_json::Value>,
            pub api_security: Option<serde_json::Value>,
        }
        pub struct ProviderConfigBorrowed<'a> {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
            pub enabled: bool,
            pub webhook_security: Option<postgres_types::Json<&'a serde_json::value::RawValue>>,
            pub api_security: Option<postgres_types::Json<&'a serde_json::value::RawValue>>,
        }
        impl<'a> From<ProviderConfigBorrowed<'a>> for ProviderConfig {
            fn from(
                ProviderConfigBorrowed {
                    id,
                    tenant_id,
                    invoicing_provider,
                    enabled,
                    webhook_security,
                    api_security,
                }: ProviderConfigBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    tenant_id,
                    invoicing_provider,
                    enabled,
                    webhook_security: webhook_security
                        .map(|v| serde_json::from_str(v.0.get()).unwrap()),
                    api_security: api_security.map(|v| serde_json::from_str(v.0.get()).unwrap()),
                }
            }
        }
        pub struct ProviderConfigQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ProviderConfigBorrowed,
            mapper: fn(ProviderConfigBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ProviderConfigQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(ProviderConfigBorrowed) -> R,
            ) -> ProviderConfigQuery<'a, C, R, N> {
                ProviderConfigQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn get_config_by_provider_and_endpoint() -> GetConfigByProviderAndEndpointStmt {
            GetConfigByProviderAndEndpointStmt(cornucopia_async :: private :: Stmt :: new("SELECT id, tenant_id, invoicing_provider, enabled, webhook_security, api_security FROM provider_config WHERE tenant_id = $1 AND invoicing_provider = $2 AND enabled = TRUE"))
        }
        pub struct GetConfigByProviderAndEndpointStmt(cornucopia_async::private::Stmt);
        impl GetConfigByProviderAndEndpointStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                invoicing_provider: &'a super::super::types::public::InvoicingProviderEnum,
            ) -> ProviderConfigQuery<'a, C, ProviderConfig, 2> {
                ProviderConfigQuery {
                    client,
                    params: [tenant_id, invoicing_provider],
                    stmt: &mut self.0,
                    extractor: |row| ProviderConfigBorrowed {
                        id: row.get(0),
                        tenant_id: row.get(1),
                        invoicing_provider: row.get(2),
                        enabled: row.get(3),
                        webhook_security: row.get(4),
                        api_security: row.get(5),
                    },
                    mapper: |it| <ProviderConfig>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetConfigByProviderAndEndpointParams,
                ProviderConfigQuery<'a, C, ProviderConfig, 2>,
                C,
            > for GetConfigByProviderAndEndpointStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetConfigByProviderAndEndpointParams,
            ) -> ProviderConfigQuery<'a, C, ProviderConfig, 2> {
                self.bind(client, &params.tenant_id, &params.invoicing_provider)
            }
        }
        pub fn create_provider_config() -> CreateProviderConfigStmt {
            CreateProviderConfigStmt(cornucopia_async :: private :: Stmt :: new("INSERT INTO provider_config (id, tenant_id, invoicing_provider, enabled, webhook_security, api_security)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (tenant_id, invoicing_provider)
  where enabled = TRUE
  DO UPDATE
  SET
    enabled = EXCLUDED.enabled,
    webhook_security = EXCLUDED.webhook_security,
    api_security = EXCLUDED.api_security
RETURNING id, tenant_id, invoicing_provider, enabled, webhook_security, api_security"))
        }
        pub struct CreateProviderConfigStmt(cornucopia_async::private::Stmt);
        impl CreateProviderConfigStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::JsonSql,
                T2: cornucopia_async::JsonSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
                invoicing_provider: &'a super::super::types::public::InvoicingProviderEnum,
                enabled: &'a bool,
                webhook_security: &'a Option<T1>,
                api_security: &'a Option<T2>,
            ) -> ProviderConfigQuery<'a, C, ProviderConfig, 6> {
                ProviderConfigQuery {
                    client,
                    params: [
                        id,
                        tenant_id,
                        invoicing_provider,
                        enabled,
                        webhook_security,
                        api_security,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| ProviderConfigBorrowed {
                        id: row.get(0),
                        tenant_id: row.get(1),
                        invoicing_provider: row.get(2),
                        enabled: row.get(3),
                        webhook_security: row.get(4),
                        api_security: row.get(5),
                    },
                    mapper: |it| <ProviderConfig>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::JsonSql,
                T2: cornucopia_async::JsonSql,
            >
            cornucopia_async::Params<
                'a,
                CreateProviderConfigParams<T1, T2>,
                ProviderConfigQuery<'a, C, ProviderConfig, 6>,
                C,
            > for CreateProviderConfigStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateProviderConfigParams<T1, T2>,
            ) -> ProviderConfigQuery<'a, C, ProviderConfig, 6> {
                self.bind(
                    client,
                    &params.id,
                    &params.tenant_id,
                    &params.invoicing_provider,
                    &params.enabled,
                    &params.webhook_security,
                    &params.api_security,
                )
            }
        }
    }
    pub mod rates {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct InsertRatesParams<T1: cornucopia_async::JsonSql> {
            pub id: uuid::Uuid,
            pub date: time::Date,
            pub rates: T1,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct InsertRates {
            pub id: uuid::Uuid,
            pub date: time::Date,
            pub rates: serde_json::Value,
        }
        pub struct InsertRatesBorrowed<'a> {
            pub id: uuid::Uuid,
            pub date: time::Date,
            pub rates: postgres_types::Json<&'a serde_json::value::RawValue>,
        }
        impl<'a> From<InsertRatesBorrowed<'a>> for InsertRates {
            fn from(InsertRatesBorrowed { id, date, rates }: InsertRatesBorrowed<'a>) -> Self {
                Self {
                    id,
                    date,
                    rates: serde_json::from_str(rates.0.get()).unwrap(),
                }
            }
        }
        pub struct InsertRatesQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> InsertRatesBorrowed,
            mapper: fn(InsertRatesBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> InsertRatesQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(InsertRatesBorrowed) -> R,
            ) -> InsertRatesQuery<'a, C, R, N> {
                InsertRatesQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub struct SerdejsonValueQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor:
                fn(&tokio_postgres::Row) -> postgres_types::Json<&serde_json::value::RawValue>,
            mapper: fn(postgres_types::Json<&serde_json::value::RawValue>) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> SerdejsonValueQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(postgres_types::Json<&serde_json::value::RawValue>) -> R,
            ) -> SerdejsonValueQuery<'a, C, R, N> {
                SerdejsonValueQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn insert_rates() -> InsertRatesStmt {
            InsertRatesStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO historical_rates_from_usd (id, date, rates)
VALUES ($1, $2, $3)
ON CONFLICT (date) DO UPDATE SET rates = EXCLUDED.rates
RETURNING id, date, rates",
            ))
        }
        pub struct InsertRatesStmt(cornucopia_async::private::Stmt);
        impl InsertRatesStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::JsonSql>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                date: &'a time::Date,
                rates: &'a T1,
            ) -> InsertRatesQuery<'a, C, InsertRates, 3> {
                InsertRatesQuery {
                    client,
                    params: [id, date, rates],
                    stmt: &mut self.0,
                    extractor: |row| InsertRatesBorrowed {
                        id: row.get(0),
                        date: row.get(1),
                        rates: row.get(2),
                    },
                    mapper: |it| <InsertRates>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::JsonSql>
            cornucopia_async::Params<
                'a,
                InsertRatesParams<T1>,
                InsertRatesQuery<'a, C, InsertRates, 3>,
                C,
            > for InsertRatesStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a InsertRatesParams<T1>,
            ) -> InsertRatesQuery<'a, C, InsertRates, 3> {
                self.bind(client, &params.id, &params.date, &params.rates)
            }
        }
        pub fn get_rates() -> GetRatesStmt {
            GetRatesStmt(cornucopia_async::private::Stmt::new(
                "SELECT rates
FROM historical_rates_from_usd
WHERE date = $1",
            ))
        }
        pub struct GetRatesStmt(cornucopia_async::private::Stmt);
        impl GetRatesStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                date: &'a time::Date,
            ) -> SerdejsonValueQuery<'a, C, serde_json::Value, 1> {
                SerdejsonValueQuery {
                    client,
                    params: [date],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| serde_json::from_str(it.0.get()).unwrap(),
                }
            }
        }
    }
    pub mod schedules {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateScheduleParams<T1: cornucopia_async::JsonSql> {
            pub id: uuid::Uuid,
            pub billing_period: super::super::types::public::BillingPeriodEnum,
            pub plan_version_id: uuid::Uuid,
            pub ramps: T1,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct UpdateScheduleParams<T1: cornucopia_async::JsonSql> {
            pub ramps: T1,
            pub tenant_id: uuid::Uuid,
            pub id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct ListSchedulesParams {
            pub tenant_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct DeleteScheduleParams {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct Schedule {
            pub id: uuid::Uuid,
            pub billing_period: super::super::types::public::BillingPeriodEnum,
            pub ramps: serde_json::Value,
        }
        pub struct ScheduleBorrowed<'a> {
            pub id: uuid::Uuid,
            pub billing_period: super::super::types::public::BillingPeriodEnum,
            pub ramps: postgres_types::Json<&'a serde_json::value::RawValue>,
        }
        impl<'a> From<ScheduleBorrowed<'a>> for Schedule {
            fn from(
                ScheduleBorrowed {
                    id,
                    billing_period,
                    ramps,
                }: ScheduleBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    billing_period,
                    ramps: serde_json::from_str(ramps.0.get()).unwrap(),
                }
            }
        }
        pub struct ScheduleQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ScheduleBorrowed,
            mapper: fn(ScheduleBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ScheduleQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(ScheduleBorrowed) -> R) -> ScheduleQuery<'a, C, R, N> {
                ScheduleQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn create_schedule() -> CreateScheduleStmt {
            CreateScheduleStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO schedule (id, billing_period, plan_version_id, ramps)
SELECT $1, $2, $3, $4
FROM plan_version
WHERE plan_version.id = $3
  AND plan_version.tenant_id = $5
RETURNING id, billing_period, ramps",
            ))
        }
        pub struct CreateScheduleStmt(cornucopia_async::private::Stmt);
        impl CreateScheduleStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::JsonSql>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                billing_period: &'a super::super::types::public::BillingPeriodEnum,
                plan_version_id: &'a uuid::Uuid,
                ramps: &'a T1,
                tenant_id: &'a uuid::Uuid,
            ) -> ScheduleQuery<'a, C, Schedule, 5> {
                ScheduleQuery {
                    client,
                    params: [id, billing_period, plan_version_id, ramps, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| ScheduleBorrowed {
                        id: row.get(0),
                        billing_period: row.get(1),
                        ramps: row.get(2),
                    },
                    mapper: |it| <Schedule>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::JsonSql>
            cornucopia_async::Params<
                'a,
                CreateScheduleParams<T1>,
                ScheduleQuery<'a, C, Schedule, 5>,
                C,
            > for CreateScheduleStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateScheduleParams<T1>,
            ) -> ScheduleQuery<'a, C, Schedule, 5> {
                self.bind(
                    client,
                    &params.id,
                    &params.billing_period,
                    &params.plan_version_id,
                    &params.ramps,
                    &params.tenant_id,
                )
            }
        }
        pub fn update_schedule() -> UpdateScheduleStmt {
            UpdateScheduleStmt(cornucopia_async::private::Stmt::new(
                "UPDATE schedule
SET ramps = $1
FROM plan_version
WHERE schedule.plan_version_id = plan_version.id
  AND plan_version.tenant_id = $2
  AND plan_version.is_draft_version = true
  AND schedule.id = $3
RETURNING schedule.id, schedule.billing_period, schedule.ramps",
            ))
        }
        pub struct UpdateScheduleStmt(cornucopia_async::private::Stmt);
        impl UpdateScheduleStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::JsonSql>(
                &'a mut self,
                client: &'a C,
                ramps: &'a T1,
                tenant_id: &'a uuid::Uuid,
                id: &'a uuid::Uuid,
            ) -> ScheduleQuery<'a, C, Schedule, 3> {
                ScheduleQuery {
                    client,
                    params: [ramps, tenant_id, id],
                    stmt: &mut self.0,
                    extractor: |row| ScheduleBorrowed {
                        id: row.get(0),
                        billing_period: row.get(1),
                        ramps: row.get(2),
                    },
                    mapper: |it| <Schedule>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::JsonSql>
            cornucopia_async::Params<
                'a,
                UpdateScheduleParams<T1>,
                ScheduleQuery<'a, C, Schedule, 3>,
                C,
            > for UpdateScheduleStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpdateScheduleParams<T1>,
            ) -> ScheduleQuery<'a, C, Schedule, 3> {
                self.bind(client, &params.ramps, &params.tenant_id, &params.id)
            }
        }
        pub fn list_schedules() -> ListSchedulesStmt {
            ListSchedulesStmt(cornucopia_async::private::Stmt::new(
                "SELECT s.id, s.billing_period, s.ramps
FROM schedule s
         JOIN plan_version pv ON s.plan_version_id = pv.id
WHERE pv.tenant_id = $1
  AND pv.id = $2",
            ))
        }
        pub struct ListSchedulesStmt(cornucopia_async::private::Stmt);
        impl ListSchedulesStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                plan_version_id: &'a uuid::Uuid,
            ) -> ScheduleQuery<'a, C, Schedule, 2> {
                ScheduleQuery {
                    client,
                    params: [tenant_id, plan_version_id],
                    stmt: &mut self.0,
                    extractor: |row| ScheduleBorrowed {
                        id: row.get(0),
                        billing_period: row.get(1),
                        ramps: row.get(2),
                    },
                    mapper: |it| <Schedule>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<'a, ListSchedulesParams, ScheduleQuery<'a, C, Schedule, 2>, C>
            for ListSchedulesStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListSchedulesParams,
            ) -> ScheduleQuery<'a, C, Schedule, 2> {
                self.bind(client, &params.tenant_id, &params.plan_version_id)
            }
        }
        pub fn list_schedules_by_subscription() -> ListSchedulesBySubscriptionStmt {
            ListSchedulesBySubscriptionStmt(cornucopia_async::private::Stmt::new(
                "SELECT s.id, s.billing_period, s.ramps
FROM schedule s
         JOIN subscription ss ON s.plan_version_id = ss.plan_version_id
         JOIN plan_version pv ON s.plan_version_id = pv.id
WHERE ss.id = $1",
            ))
        }
        pub struct ListSchedulesBySubscriptionStmt(cornucopia_async::private::Stmt);
        impl ListSchedulesBySubscriptionStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                subscription_id: &'a uuid::Uuid,
            ) -> ScheduleQuery<'a, C, Schedule, 1> {
                ScheduleQuery {
                    client,
                    params: [subscription_id],
                    stmt: &mut self.0,
                    extractor: |row| ScheduleBorrowed {
                        id: row.get(0),
                        billing_period: row.get(1),
                        ramps: row.get(2),
                    },
                    mapper: |it| <Schedule>::from(it),
                }
            }
        }
        pub fn delete_schedule() -> DeleteScheduleStmt {
            DeleteScheduleStmt(cornucopia_async::private::Stmt::new(
                "DELETE
FROM schedule s
    USING plan_version pv
WHERE s.id = $1
  AND s.plan_version_id = pv.id
  AND pv.tenant_id = $2
  AND pv.is_draft_version = true",
            ))
        }
        pub struct DeleteScheduleStmt(cornucopia_async::private::Stmt);
        impl DeleteScheduleStmt {
            pub async fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[id, tenant_id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync>
            cornucopia_async::Params<
                'a,
                DeleteScheduleParams,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for DeleteScheduleStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a DeleteScheduleParams,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.id, &params.tenant_id))
            }
        }
    }
    pub mod slot_transactions {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Clone, Copy, Debug)]
        pub struct CreateSlotTransactionParams {
            pub id: uuid::Uuid,
            pub price_component_id: uuid::Uuid,
            pub subscription_id: uuid::Uuid,
            pub delta: i32,
            pub prev_active_slots: i32,
            pub effective_at: time::PrimitiveDateTime,
            pub transaction_at: time::PrimitiveDateTime,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetActiveSlotsParams {
            pub subscription_id: uuid::Uuid,
            pub price_component_id: uuid::Uuid,
            pub now: time::PrimitiveDateTime,
        }
        pub struct UuidUuidQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> uuid::Uuid,
            mapper: fn(uuid::Uuid) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> UuidUuidQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(uuid::Uuid) -> R) -> UuidUuidQuery<'a, C, R, N> {
                UuidUuidQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub struct I64Query<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> i64,
            mapper: fn(i64) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> I64Query<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(i64) -> R) -> I64Query<'a, C, R, N> {
                I64Query {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn create_slot_transaction() -> CreateSlotTransactionStmt {
            CreateSlotTransactionStmt(cornucopia_async :: private :: Stmt :: new("insert into slot_transaction(id, price_component_id, subscription_id, delta, prev_active_slots, effective_at, transaction_at)
values ($1,
        $2,
        $3,
        $4,
        $5,
        $6,
        $7)
returning id"))
        }
        pub struct CreateSlotTransactionStmt(cornucopia_async::private::Stmt);
        impl CreateSlotTransactionStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                price_component_id: &'a uuid::Uuid,
                subscription_id: &'a uuid::Uuid,
                delta: &'a i32,
                prev_active_slots: &'a i32,
                effective_at: &'a time::PrimitiveDateTime,
                transaction_at: &'a time::PrimitiveDateTime,
            ) -> UuidUuidQuery<'a, C, uuid::Uuid, 7> {
                UuidUuidQuery {
                    client,
                    params: [
                        id,
                        price_component_id,
                        subscription_id,
                        delta,
                        prev_active_slots,
                        effective_at,
                        transaction_at,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                CreateSlotTransactionParams,
                UuidUuidQuery<'a, C, uuid::Uuid, 7>,
                C,
            > for CreateSlotTransactionStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateSlotTransactionParams,
            ) -> UuidUuidQuery<'a, C, uuid::Uuid, 7> {
                self.bind(
                    client,
                    &params.id,
                    &params.price_component_id,
                    &params.subscription_id,
                    &params.delta,
                    &params.prev_active_slots,
                    &params.effective_at,
                    &params.transaction_at,
                )
            }
        }
        pub fn get_active_slots() -> GetActiveSlotsStmt {
            GetActiveSlotsStmt(cornucopia_async :: private :: Stmt :: new("WITH RankedSlotTransactions AS (
  SELECT
    st.*,
    ROW_NUMBER() OVER (PARTITION BY st.subscription_id, st.price_component_id ORDER BY st.transaction_at DESC) AS row_num
  FROM
    slot_transaction st
  WHERE
    st.subscription_id = $1
    AND st.price_component_id = $2
    AND st.transaction_at <= $3
)
SELECT
  X.prev_active_slots + COALESCE(SUM(Y.delta), 0) AS current_active_slots
FROM
  RankedSlotTransactions X
    LEFT JOIN
  RankedSlotTransactions Y ON Y.effective_at BETWEEN X.transaction_at AND $3
WHERE
  X.row_num = 1
GROUP BY
  X.prev_active_slots"))
        }
        pub struct GetActiveSlotsStmt(cornucopia_async::private::Stmt);
        impl GetActiveSlotsStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                subscription_id: &'a uuid::Uuid,
                price_component_id: &'a uuid::Uuid,
                now: &'a time::PrimitiveDateTime,
            ) -> I64Query<'a, C, i64, 3> {
                I64Query {
                    client,
                    params: [subscription_id, price_component_id, now],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<'a, GetActiveSlotsParams, I64Query<'a, C, i64, 3>, C>
            for GetActiveSlotsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetActiveSlotsParams,
            ) -> I64Query<'a, C, i64, 3> {
                self.bind(
                    client,
                    &params.subscription_id,
                    &params.price_component_id,
                    &params.now,
                )
            }
        }
    }
    pub mod stats {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct TopRevenuePerCustomerParams<T1: cornucopia_async::StringSql> {
            pub tenant_id: uuid::Uuid,
            pub currency: T1,
            pub limit: i64,
        }
        #[derive(Debug)]
        pub struct InsertMrrMovementLogParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub movement_type: super::super::types::public::MrrMovementType,
            pub net_mrr_change: i64,
            pub currency: T1,
            pub applies_to: time::Date,
            pub description: T2,
            pub invoice_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct NewMrrAtDateParams {
            pub date: time::Date,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct TotalMrrAtDateParams {
            pub tenant_id: uuid::Uuid,
            pub date: time::Date,
        }
        #[derive(Debug)]
        pub struct TotalMrrAtDateByPlanParams<T1: cornucopia_async::ArraySql<Item = uuid::Uuid>> {
            pub date: time::Date,
            pub tenant_id: uuid::Uuid,
            pub plan_ids: T1,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct QueryTotalMrrParams {
            pub tenant_id: uuid::Uuid,
            pub start_date: time::Date,
            pub end_date: time::Date,
        }
        #[derive(Debug)]
        pub struct QueryTotalMrrByPlanParams<T1: cornucopia_async::ArraySql<Item = uuid::Uuid>> {
            pub tenant_id: uuid::Uuid,
            pub start_date: time::Date,
            pub plan_ids: T1,
            pub end_date: time::Date,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetMrrBreakdownParams {
            pub tenant_id: uuid::Uuid,
            pub start_date: time::Date,
            pub end_date: time::Date,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct QueryTotalNetRevenueParams {
            pub tenant_id: uuid::Uuid,
            pub start_date: time::Date,
            pub end_date: time::Date,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetLastMrrMovementsParams {
            pub tenant_id: uuid::Uuid,
            pub before: Option<uuid::Uuid>,
            pub after: Option<uuid::Uuid>,
            pub limit: i64,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct QueryRevenueTrendParams {
            pub period_days: i32,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct TopRevenuePerCustomer {
            pub id: uuid::Uuid,
            pub name: String,
            pub total_revenue_cents: i64,
            pub currency: String,
        }
        pub struct TopRevenuePerCustomerBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub total_revenue_cents: i64,
            pub currency: &'a str,
        }
        impl<'a> From<TopRevenuePerCustomerBorrowed<'a>> for TopRevenuePerCustomer {
            fn from(
                TopRevenuePerCustomerBorrowed {
                    id,
                    name,
                    total_revenue_cents,
                    currency,
                }: TopRevenuePerCustomerBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    total_revenue_cents,
                    currency: currency.into(),
                }
            }
        }
        pub struct TopRevenuePerCustomerQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> TopRevenuePerCustomerBorrowed,
            mapper: fn(TopRevenuePerCustomerBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> TopRevenuePerCustomerQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(TopRevenuePerCustomerBorrowed) -> R,
            ) -> TopRevenuePerCustomerQuery<'a, C, R, N> {
                TopRevenuePerCustomerQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub struct I64Query<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> i64,
            mapper: fn(i64) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> I64Query<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(i64) -> R) -> I64Query<'a, C, R, N> {
                I64Query {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct TotalMrrAtDateByPlan {
            pub total_net_mrr_cents: i64,
            pub plan_id: uuid::Uuid,
            pub plan_name: String,
        }
        pub struct TotalMrrAtDateByPlanBorrowed<'a> {
            pub total_net_mrr_cents: i64,
            pub plan_id: uuid::Uuid,
            pub plan_name: &'a str,
        }
        impl<'a> From<TotalMrrAtDateByPlanBorrowed<'a>> for TotalMrrAtDateByPlan {
            fn from(
                TotalMrrAtDateByPlanBorrowed {
                    total_net_mrr_cents,
                    plan_id,
                    plan_name,
                }: TotalMrrAtDateByPlanBorrowed<'a>,
            ) -> Self {
                Self {
                    total_net_mrr_cents,
                    plan_id,
                    plan_name: plan_name.into(),
                }
            }
        }
        pub struct TotalMrrAtDateByPlanQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> TotalMrrAtDateByPlanBorrowed,
            mapper: fn(TotalMrrAtDateByPlanBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> TotalMrrAtDateByPlanQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(TotalMrrAtDateByPlanBorrowed) -> R,
            ) -> TotalMrrAtDateByPlanQuery<'a, C, R, N> {
                TotalMrrAtDateByPlanQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct QueryTotalMrr {
            pub period: time::Date,
            pub total_net_mrr: i64,
            pub net_new_mrr: i64,
            pub new_business_mrr: i64,
            pub new_business_count: i32,
            pub expansion_mrr: i64,
            pub expansion_count: i32,
            pub contraction_mrr: i64,
            pub contraction_count: i32,
            pub churn_mrr: i64,
            pub churn_count: i32,
            pub reactivation_mrr: i64,
            pub reactivation_count: i32,
        }
        pub struct QueryTotalMrrQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> QueryTotalMrr,
            mapper: fn(QueryTotalMrr) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> QueryTotalMrrQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(QueryTotalMrr) -> R) -> QueryTotalMrrQuery<'a, C, R, N> {
                QueryTotalMrrQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct QueryTotalMrrByPlan {
            pub date: time::Date,
            pub plan_id: uuid::Uuid,
            pub plan_name: String,
            pub total_net_mrr: i64,
            pub net_new_mrr: i64,
            pub new_business_mrr: i64,
            pub new_business_count: i32,
            pub expansion_mrr: i64,
            pub expansion_count: i32,
            pub contraction_mrr: i64,
            pub contraction_count: i32,
            pub churn_mrr: i64,
            pub churn_count: i32,
            pub reactivation_mrr: i64,
            pub reactivation_count: i32,
        }
        pub struct QueryTotalMrrByPlanBorrowed<'a> {
            pub date: time::Date,
            pub plan_id: uuid::Uuid,
            pub plan_name: &'a str,
            pub total_net_mrr: i64,
            pub net_new_mrr: i64,
            pub new_business_mrr: i64,
            pub new_business_count: i32,
            pub expansion_mrr: i64,
            pub expansion_count: i32,
            pub contraction_mrr: i64,
            pub contraction_count: i32,
            pub churn_mrr: i64,
            pub churn_count: i32,
            pub reactivation_mrr: i64,
            pub reactivation_count: i32,
        }
        impl<'a> From<QueryTotalMrrByPlanBorrowed<'a>> for QueryTotalMrrByPlan {
            fn from(
                QueryTotalMrrByPlanBorrowed {
                    date,
                    plan_id,
                    plan_name,
                    total_net_mrr,
                    net_new_mrr,
                    new_business_mrr,
                    new_business_count,
                    expansion_mrr,
                    expansion_count,
                    contraction_mrr,
                    contraction_count,
                    churn_mrr,
                    churn_count,
                    reactivation_mrr,
                    reactivation_count,
                }: QueryTotalMrrByPlanBorrowed<'a>,
            ) -> Self {
                Self {
                    date,
                    plan_id,
                    plan_name: plan_name.into(),
                    total_net_mrr,
                    net_new_mrr,
                    new_business_mrr,
                    new_business_count,
                    expansion_mrr,
                    expansion_count,
                    contraction_mrr,
                    contraction_count,
                    churn_mrr,
                    churn_count,
                    reactivation_mrr,
                    reactivation_count,
                }
            }
        }
        pub struct QueryTotalMrrByPlanQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> QueryTotalMrrByPlanBorrowed,
            mapper: fn(QueryTotalMrrByPlanBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> QueryTotalMrrByPlanQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(QueryTotalMrrByPlanBorrowed) -> R,
            ) -> QueryTotalMrrByPlanQuery<'a, C, R, N> {
                QueryTotalMrrByPlanQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct GetMrrBreakdown {
            pub net_new_mrr: i64,
            pub new_business_mrr: i64,
            pub new_business_count: i32,
            pub expansion_mrr: i64,
            pub expansion_count: i32,
            pub contraction_mrr: i64,
            pub contraction_count: i32,
            pub churn_mrr: i64,
            pub churn_count: i32,
            pub reactivation_mrr: i64,
            pub reactivation_count: i32,
        }
        pub struct GetMrrBreakdownQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> GetMrrBreakdown,
            mapper: fn(GetMrrBreakdown) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> GetMrrBreakdownQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(GetMrrBreakdown) -> R,
            ) -> GetMrrBreakdownQuery<'a, C, R, N> {
                GetMrrBreakdownQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct GetLastMrrMovements {
            pub id: uuid::Uuid,
            pub movement_type: super::super::types::public::MrrMovementType,
            pub net_mrr_change: i64,
            pub currency: String,
            pub applies_to: time::Date,
            pub created_at: time::PrimitiveDateTime,
            pub description: String,
            pub invoice_id: uuid::Uuid,
            pub credit_note_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub customer_name: String,
            pub subscription_id: uuid::Uuid,
            pub plan_name: String,
        }
        pub struct GetLastMrrMovementsBorrowed<'a> {
            pub id: uuid::Uuid,
            pub movement_type: super::super::types::public::MrrMovementType,
            pub net_mrr_change: i64,
            pub currency: &'a str,
            pub applies_to: time::Date,
            pub created_at: time::PrimitiveDateTime,
            pub description: &'a str,
            pub invoice_id: uuid::Uuid,
            pub credit_note_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub customer_name: &'a str,
            pub subscription_id: uuid::Uuid,
            pub plan_name: &'a str,
        }
        impl<'a> From<GetLastMrrMovementsBorrowed<'a>> for GetLastMrrMovements {
            fn from(
                GetLastMrrMovementsBorrowed {
                    id,
                    movement_type,
                    net_mrr_change,
                    currency,
                    applies_to,
                    created_at,
                    description,
                    invoice_id,
                    credit_note_id,
                    tenant_id,
                    plan_version_id,
                    customer_id,
                    customer_name,
                    subscription_id,
                    plan_name,
                }: GetLastMrrMovementsBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    movement_type,
                    net_mrr_change,
                    currency: currency.into(),
                    applies_to,
                    created_at,
                    description: description.into(),
                    invoice_id,
                    credit_note_id,
                    tenant_id,
                    plan_version_id,
                    customer_id,
                    customer_name: customer_name.into(),
                    subscription_id,
                    plan_name: plan_name.into(),
                }
            }
        }
        pub struct GetLastMrrMovementsQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> GetLastMrrMovementsBorrowed,
            mapper: fn(GetLastMrrMovementsBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> GetLastMrrMovementsQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(GetLastMrrMovementsBorrowed) -> R,
            ) -> GetLastMrrMovementsQuery<'a, C, R, N> {
                GetLastMrrMovementsQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct QueryRevenueTrend {
            pub total_ytd: i64,
            pub total_current_period: i64,
            pub total_previous_period: i64,
        }
        pub struct QueryRevenueTrendQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> QueryRevenueTrend,
            mapper: fn(QueryRevenueTrend) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> QueryRevenueTrendQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(QueryRevenueTrend) -> R,
            ) -> QueryRevenueTrendQuery<'a, C, R, N> {
                QueryRevenueTrendQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct QueryPendingInvoices {
            pub total: i32,
            pub total_cents: rust_decimal::Decimal,
        }
        pub struct QueryPendingInvoicesQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> QueryPendingInvoices,
            mapper: fn(QueryPendingInvoices) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> QueryPendingInvoicesQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(QueryPendingInvoices) -> R,
            ) -> QueryPendingInvoicesQuery<'a, C, R, N> {
                QueryPendingInvoicesQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct DailyNewSignups30Days {
            pub signup_date: time::Date,
            pub daily_signups: i64,
            pub total_signups_over_30_days: i64,
        }
        pub struct DailyNewSignups30DaysQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> DailyNewSignups30Days,
            mapper: fn(DailyNewSignups30Days) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> DailyNewSignups30DaysQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(DailyNewSignups30Days) -> R,
            ) -> DailyNewSignups30DaysQuery<'a, C, R, N> {
                DailyNewSignups30DaysQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct NewSignupsTrend30Days {
            pub total_last_30_days: i64,
            pub total_previous_30_days: i64,
        }
        pub struct NewSignupsTrend30DaysQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> NewSignupsTrend30Days,
            mapper: fn(NewSignupsTrend30Days) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> NewSignupsTrend30DaysQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(NewSignupsTrend30Days) -> R,
            ) -> NewSignupsTrend30DaysQuery<'a, C, R, N> {
                NewSignupsTrend30DaysQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub struct RustdecimalDecimalQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> rust_decimal::Decimal,
            mapper: fn(rust_decimal::Decimal) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> RustdecimalDecimalQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(rust_decimal::Decimal) -> R,
            ) -> RustdecimalDecimalQuery<'a, C, R, N> {
                RustdecimalDecimalQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct QueryTrialToPaidConversionOverTime {
            pub month: time::OffsetDateTime,
            pub total_trials: i64,
            pub conversions: i64,
            pub conversion_rate_percentage: rust_decimal::Decimal,
            pub conversions_30: i64,
            pub conversion_rate_30_percentage: rust_decimal::Decimal,
            pub conversions_90: i64,
            pub conversion_rate_90_percentage: rust_decimal::Decimal,
        }
        pub struct QueryTrialToPaidConversionOverTimeQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> QueryTrialToPaidConversionOverTime,
            mapper: fn(QueryTrialToPaidConversionOverTime) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> QueryTrialToPaidConversionOverTimeQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(QueryTrialToPaidConversionOverTime) -> R,
            ) -> QueryTrialToPaidConversionOverTimeQuery<'a, C, R, N> {
                QueryTrialToPaidConversionOverTimeQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn top_revenue_per_customer() -> TopRevenuePerCustomerStmt {
            TopRevenuePerCustomerStmt(cornucopia_async::private::Stmt::new(
                "SELECT c.id,
       c.name,
       COALESCE(bi.total_revenue_cents, 0)::bigint AS total_revenue_cents,
       bi.currency
FROM customer c
         LEFT JOIN bi_customer_ytd_summary bi ON bi.customer_id = c.id
WHERE c.tenant_id = $1
  AND (bi.revenue_year IS NULL OR bi.currency = $2)
  AND (bi.revenue_year IS NULL OR bi.revenue_year = DATE_PART('year', CURRENT_DATE))
ORDER BY total_revenue_cents DESC
LIMIT $3",
            ))
        }
        pub struct TopRevenuePerCustomerStmt(cornucopia_async::private::Stmt);
        impl TopRevenuePerCustomerStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                currency: &'a T1,
                limit: &'a i64,
            ) -> TopRevenuePerCustomerQuery<'a, C, TopRevenuePerCustomer, 3> {
                TopRevenuePerCustomerQuery {
                    client,
                    params: [tenant_id, currency, limit],
                    stmt: &mut self.0,
                    extractor: |row| TopRevenuePerCustomerBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        total_revenue_cents: row.get(2),
                        currency: row.get(3),
                    },
                    mapper: |it| <TopRevenuePerCustomer>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                TopRevenuePerCustomerParams<T1>,
                TopRevenuePerCustomerQuery<'a, C, TopRevenuePerCustomer, 3>,
                C,
            > for TopRevenuePerCustomerStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a TopRevenuePerCustomerParams<T1>,
            ) -> TopRevenuePerCustomerQuery<'a, C, TopRevenuePerCustomer, 3> {
                self.bind(client, &params.tenant_id, &params.currency, &params.limit)
            }
        }
        pub fn insert_mrr_movement_log() -> InsertMrrMovementLogStmt {
            InsertMrrMovementLogStmt(cornucopia_async :: private :: Stmt :: new("INSERT INTO bi_mrr_movement_log (id, movement_type, net_mrr_change, currency, applies_to, description, invoice_id,
                                 tenant_id, plan_version_id)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
        $9)"))
        }
        pub struct InsertMrrMovementLogStmt(cornucopia_async::private::Stmt);
        impl InsertMrrMovementLogStmt {
            pub async fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                movement_type: &'a super::super::types::public::MrrMovementType,
                net_mrr_change: &'a i64,
                currency: &'a T1,
                applies_to: &'a time::Date,
                description: &'a T2,
                invoice_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
                plan_version_id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client
                    .execute(
                        stmt,
                        &[
                            id,
                            movement_type,
                            net_mrr_change,
                            currency,
                            applies_to,
                            description,
                            invoice_id,
                            tenant_id,
                            plan_version_id,
                        ],
                    )
                    .await
            }
        }
        impl<
                'a,
                C: GenericClient + Send + Sync,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                InsertMrrMovementLogParams<T1, T2>,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for InsertMrrMovementLogStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a InsertMrrMovementLogParams<T1, T2>,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(
                    client,
                    &params.id,
                    &params.movement_type,
                    &params.net_mrr_change,
                    &params.currency,
                    &params.applies_to,
                    &params.description,
                    &params.invoice_id,
                    &params.tenant_id,
                    &params.plan_version_id,
                ))
            }
        }
        pub fn new_mrr_at_date() -> NewMrrAtDateStmt {
            NewMrrAtDateStmt(cornucopia_async :: private :: Stmt :: new("SELECT
    (bd.net_mrr_cents_usd * (hr.rates->>(SELECT currency FROM tenant WHERE id = bd.tenant_id))::NUMERIC)::bigint AS net_mrr_cents
FROM bi_delta_mrr_daily bd
         JOIN historical_rates_from_usd hr ON bd.historical_rate_id = hr.id
WHERE bd.date = $1
  AND tenant_id = $2"))
        }
        pub struct NewMrrAtDateStmt(cornucopia_async::private::Stmt);
        impl NewMrrAtDateStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                date: &'a time::Date,
                tenant_id: &'a uuid::Uuid,
            ) -> I64Query<'a, C, i64, 2> {
                I64Query {
                    client,
                    params: [date, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<'a, NewMrrAtDateParams, I64Query<'a, C, i64, 2>, C>
            for NewMrrAtDateStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a NewMrrAtDateParams,
            ) -> I64Query<'a, C, i64, 2> {
                self.bind(client, &params.date, &params.tenant_id)
            }
        }
        pub fn total_mrr_at_date() -> TotalMrrAtDateStmt {
            TotalMrrAtDateStmt(cornucopia_async :: private :: Stmt :: new("SELECT
    COALESCE(SUM(bd.net_mrr_cents_usd * (hr.rates->>(SELECT currency FROM tenant WHERE id = bd.tenant_id))::NUMERIC), 0)::bigint AS total_net_mrr_cents
FROM
    bi_delta_mrr_daily bd
        JOIN  historical_rates_from_usd hr ON bd.historical_rate_id = hr.id
WHERE
    bd.tenant_id = $1
  AND bd.date <= $2"))
        }
        pub struct TotalMrrAtDateStmt(cornucopia_async::private::Stmt);
        impl TotalMrrAtDateStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                date: &'a time::Date,
            ) -> I64Query<'a, C, i64, 2> {
                I64Query {
                    client,
                    params: [tenant_id, date],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<'a, TotalMrrAtDateParams, I64Query<'a, C, i64, 2>, C>
            for TotalMrrAtDateStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a TotalMrrAtDateParams,
            ) -> I64Query<'a, C, i64, 2> {
                self.bind(client, &params.tenant_id, &params.date)
            }
        }
        pub fn total_mrr_at_date_by_plan() -> TotalMrrAtDateByPlanStmt {
            TotalMrrAtDateByPlanStmt(cornucopia_async :: private :: Stmt :: new("SELECT
    COALESCE(SUM(bi.net_mrr_cents_usd * (hr.rates->>(SELECT currency FROM tenant WHERE id = bi.tenant_id))::NUMERIC), 0)::bigint AS total_net_mrr_cents,
    p.id AS plan_id,
    p.name AS plan_name
FROM
    bi_delta_mrr_daily bi
        JOIN
    plan_version pv ON bi.plan_version_id = pv.id
        JOIN
    plan p ON pv.plan_id = p.id
        JOIN historical_rates_from_usd hr ON bi.historical_rate_id = hr.id
WHERE
    bi.date <= $1
  AND bi.tenant_id = $2
  AND p.id = ANY ($3)
GROUP BY
    p.id"))
        }
        pub struct TotalMrrAtDateByPlanStmt(cornucopia_async::private::Stmt);
        impl TotalMrrAtDateByPlanStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::ArraySql<Item = uuid::Uuid>>(
                &'a mut self,
                client: &'a C,
                date: &'a time::Date,
                tenant_id: &'a uuid::Uuid,
                plan_ids: &'a T1,
            ) -> TotalMrrAtDateByPlanQuery<'a, C, TotalMrrAtDateByPlan, 3> {
                TotalMrrAtDateByPlanQuery {
                    client,
                    params: [date, tenant_id, plan_ids],
                    stmt: &mut self.0,
                    extractor: |row| TotalMrrAtDateByPlanBorrowed {
                        total_net_mrr_cents: row.get(0),
                        plan_id: row.get(1),
                        plan_name: row.get(2),
                    },
                    mapper: |it| <TotalMrrAtDateByPlan>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::ArraySql<Item = uuid::Uuid>>
            cornucopia_async::Params<
                'a,
                TotalMrrAtDateByPlanParams<T1>,
                TotalMrrAtDateByPlanQuery<'a, C, TotalMrrAtDateByPlan, 3>,
                C,
            > for TotalMrrAtDateByPlanStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a TotalMrrAtDateByPlanParams<T1>,
            ) -> TotalMrrAtDateByPlanQuery<'a, C, TotalMrrAtDateByPlan, 3> {
                self.bind(client, &params.date, &params.tenant_id, &params.plan_ids)
            }
        }
        pub fn query_total_mrr() -> QueryTotalMrrStmt {
            QueryTotalMrrStmt(cornucopia_async :: private :: Stmt :: new("WITH conversion_rates AS (
    SELECT
        id,
        (rates->>(SELECT currency FROM tenant WHERE id = $1))::NUMERIC AS conversion_rate
    FROM
        historical_rates_from_usd
),
     initial_mrr AS (
         SELECT
             COALESCE(SUM(bd.net_mrr_cents_usd * cr.conversion_rate), 0)::BIGINT AS total_net_mrr_cents
         FROM
             bi_delta_mrr_daily bd
                 JOIN
             conversion_rates cr ON bd.historical_rate_id = cr.id
         WHERE
             bd.date < $2
           AND bd.tenant_id = $1
     )
SELECT
    bi.date AS period,
    (im.total_net_mrr_cents + COALESCE(SUM(bi.net_mrr_cents_usd) OVER (ORDER BY bi.date), 0) * cr.conversion_rate)::BIGINT AS total_net_mrr,
    (bi.net_mrr_cents_usd * cr.conversion_rate)::BIGINT AS net_new_mrr,
    (bi.new_business_cents_usd * cr.conversion_rate)::BIGINT AS new_business_mrr,
    bi.new_business_count,
    (bi.expansion_cents_usd * cr.conversion_rate)::BIGINT AS expansion_mrr,
    bi.expansion_count,
    (bi.contraction_cents_usd * cr.conversion_rate)::BIGINT AS contraction_mrr,
    bi.contraction_count,
    (bi.churn_cents_usd * cr.conversion_rate)::BIGINT AS churn_mrr,
    bi.churn_count,
    (bi.reactivation_cents_usd * cr.conversion_rate)::BIGINT AS reactivation_mrr,
    bi.reactivation_count
FROM
    bi_delta_mrr_daily bi
        JOIN
    conversion_rates cr ON bi.historical_rate_id = cr.id
        CROSS JOIN
    initial_mrr im
WHERE
    bi.date BETWEEN $2 AND $3
  AND bi.tenant_id = $1
ORDER BY
    period"))
        }
        pub struct QueryTotalMrrStmt(cornucopia_async::private::Stmt);
        impl QueryTotalMrrStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                start_date: &'a time::Date,
                end_date: &'a time::Date,
            ) -> QueryTotalMrrQuery<'a, C, QueryTotalMrr, 3> {
                QueryTotalMrrQuery {
                    client,
                    params: [tenant_id, start_date, end_date],
                    stmt: &mut self.0,
                    extractor: |row| QueryTotalMrr {
                        period: row.get(0),
                        total_net_mrr: row.get(1),
                        net_new_mrr: row.get(2),
                        new_business_mrr: row.get(3),
                        new_business_count: row.get(4),
                        expansion_mrr: row.get(5),
                        expansion_count: row.get(6),
                        contraction_mrr: row.get(7),
                        contraction_count: row.get(8),
                        churn_mrr: row.get(9),
                        churn_count: row.get(10),
                        reactivation_mrr: row.get(11),
                        reactivation_count: row.get(12),
                    },
                    mapper: |it| <QueryTotalMrr>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                QueryTotalMrrParams,
                QueryTotalMrrQuery<'a, C, QueryTotalMrr, 3>,
                C,
            > for QueryTotalMrrStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a QueryTotalMrrParams,
            ) -> QueryTotalMrrQuery<'a, C, QueryTotalMrr, 3> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.start_date,
                    &params.end_date,
                )
            }
        }
        pub fn query_total_mrr_by_plan() -> QueryTotalMrrByPlanStmt {
            QueryTotalMrrByPlanStmt(cornucopia_async :: private :: Stmt :: new("WITH conversion_rates AS (
    SELECT
        id,
        (rates->>(SELECT currency FROM tenant WHERE id = $1))::NUMERIC AS conversion_rate
    FROM
        historical_rates_from_usd
),
     initial_mrr AS (
         SELECT
             COALESCE(SUM(bi.net_mrr_cents_usd * cr.conversion_rate), 0)::BIGINT AS total_net_mrr_usd,
             pv.plan_id
         FROM
             bi_delta_mrr_daily bi
                 JOIN
             plan_version pv ON bi.plan_version_id = pv.id
                 JOIN
             conversion_rates cr ON bi.historical_rate_id = cr.id
         WHERE
             bi.date < $2
           AND bi.tenant_id = $1
           AND pv.plan_id = ANY ($3)
         GROUP BY
             pv.plan_id
     )
SELECT    bi.date,
          p.id AS plan_id,
          p.name AS plan_name,
          (im.total_net_mrr_usd + COALESCE(SUM(bi.net_mrr_cents_usd) OVER (PARTITION BY p.id ORDER BY bi.date), 0) * cr.conversion_rate)::BIGINT AS total_net_mrr,
          (bi.net_mrr_cents_usd * cr.conversion_rate)::BIGINT AS net_new_mrr,
          (bi.new_business_cents_usd * cr.conversion_rate)::BIGINT AS new_business_mrr,
          bi.new_business_count,
          (bi.expansion_cents_usd * cr.conversion_rate)::BIGINT AS expansion_mrr,
          bi.expansion_count,
          (bi.contraction_cents_usd * cr.conversion_rate)::BIGINT AS contraction_mrr,
          bi.contraction_count,
          (bi.churn_cents_usd * cr.conversion_rate)::BIGINT AS churn_mrr,
          bi.churn_count,
          (bi.reactivation_cents_usd * cr.conversion_rate)::BIGINT AS reactivation_mrr,
          bi.reactivation_count
FROM bi_delta_mrr_daily bi
         JOIN plan_version pv on bi.plan_version_id = pv.id
         JOIN plan p on pv.plan_id = p.id
         JOIN
     conversion_rates cr ON bi.historical_rate_id = cr.id
         JOIN initial_mrr im on pv.plan_id = im.plan_id
WHERE bi.date BETWEEN $2 AND $4
  AND bi.tenant_id = $1
  AND p.id = ANY ($3)
ORDER BY date"))
        }
        pub struct QueryTotalMrrByPlanStmt(cornucopia_async::private::Stmt);
        impl QueryTotalMrrByPlanStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::ArraySql<Item = uuid::Uuid>>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                start_date: &'a time::Date,
                plan_ids: &'a T1,
                end_date: &'a time::Date,
            ) -> QueryTotalMrrByPlanQuery<'a, C, QueryTotalMrrByPlan, 4> {
                QueryTotalMrrByPlanQuery {
                    client,
                    params: [tenant_id, start_date, plan_ids, end_date],
                    stmt: &mut self.0,
                    extractor: |row| QueryTotalMrrByPlanBorrowed {
                        date: row.get(0),
                        plan_id: row.get(1),
                        plan_name: row.get(2),
                        total_net_mrr: row.get(3),
                        net_new_mrr: row.get(4),
                        new_business_mrr: row.get(5),
                        new_business_count: row.get(6),
                        expansion_mrr: row.get(7),
                        expansion_count: row.get(8),
                        contraction_mrr: row.get(9),
                        contraction_count: row.get(10),
                        churn_mrr: row.get(11),
                        churn_count: row.get(12),
                        reactivation_mrr: row.get(13),
                        reactivation_count: row.get(14),
                    },
                    mapper: |it| <QueryTotalMrrByPlan>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::ArraySql<Item = uuid::Uuid>>
            cornucopia_async::Params<
                'a,
                QueryTotalMrrByPlanParams<T1>,
                QueryTotalMrrByPlanQuery<'a, C, QueryTotalMrrByPlan, 4>,
                C,
            > for QueryTotalMrrByPlanStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a QueryTotalMrrByPlanParams<T1>,
            ) -> QueryTotalMrrByPlanQuery<'a, C, QueryTotalMrrByPlan, 4> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.start_date,
                    &params.plan_ids,
                    &params.end_date,
                )
            }
        }
        pub fn get_mrr_breakdown() -> GetMrrBreakdownStmt {
            GetMrrBreakdownStmt(cornucopia_async::private::Stmt::new(
                "WITH conversion_rates AS (
    SELECT
        id,
        (rates->>(SELECT currency FROM tenant WHERE id = $1))::NUMERIC AS rate
    FROM
        historical_rates_from_usd
)
SELECT
    COALESCE(SUM(bi.net_mrr_cents_usd * cr.rate), 0)::BIGINT AS net_new_mrr,
    COALESCE(SUM(bi.new_business_cents_usd * cr.rate), 0)::BIGINT AS new_business_mrr,
    COALESCE(SUM(bi.new_business_count), 0)::INTEGER AS new_business_count,
    COALESCE(SUM(bi.expansion_cents_usd * cr.rate), 0)::BIGINT AS expansion_mrr,
    COALESCE(SUM(bi.expansion_count), 0)::INTEGER AS expansion_count,
    COALESCE(SUM(bi.contraction_cents_usd * cr.rate), 0)::BIGINT AS contraction_mrr,
    COALESCE(SUM(bi.contraction_count), 0)::INTEGER AS contraction_count,
    COALESCE(SUM(bi.churn_cents_usd * cr.rate), 0)::BIGINT AS churn_mrr,
    COALESCE(SUM(bi.churn_count), 0)::INTEGER AS churn_count,
    COALESCE(SUM(bi.reactivation_cents_usd * cr.rate), 0)::BIGINT AS reactivation_mrr,
    COALESCE(SUM(bi.reactivation_count), 0)::INTEGER AS reactivation_count
FROM
    bi_delta_mrr_daily bi
        JOIN conversion_rates cr ON bi.historical_rate_id = cr.id
WHERE
    bi.date BETWEEN $2 AND $3
  AND bi.tenant_id = $1
GROUP BY
    bi.tenant_id",
            ))
        }
        pub struct GetMrrBreakdownStmt(cornucopia_async::private::Stmt);
        impl GetMrrBreakdownStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                start_date: &'a time::Date,
                end_date: &'a time::Date,
            ) -> GetMrrBreakdownQuery<'a, C, GetMrrBreakdown, 3> {
                GetMrrBreakdownQuery {
                    client,
                    params: [tenant_id, start_date, end_date],
                    stmt: &mut self.0,
                    extractor: |row| GetMrrBreakdown {
                        net_new_mrr: row.get(0),
                        new_business_mrr: row.get(1),
                        new_business_count: row.get(2),
                        expansion_mrr: row.get(3),
                        expansion_count: row.get(4),
                        contraction_mrr: row.get(5),
                        contraction_count: row.get(6),
                        churn_mrr: row.get(7),
                        churn_count: row.get(8),
                        reactivation_mrr: row.get(9),
                        reactivation_count: row.get(10),
                    },
                    mapper: |it| <GetMrrBreakdown>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetMrrBreakdownParams,
                GetMrrBreakdownQuery<'a, C, GetMrrBreakdown, 3>,
                C,
            > for GetMrrBreakdownStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetMrrBreakdownParams,
            ) -> GetMrrBreakdownQuery<'a, C, GetMrrBreakdown, 3> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.start_date,
                    &params.end_date,
                )
            }
        }
        pub fn query_total_net_revenue() -> QueryTotalNetRevenueStmt {
            QueryTotalNetRevenueStmt(cornucopia_async::private::Stmt::new(
                "WITH conversion_rates AS (
    SELECT
        id,
        (rates->>(SELECT currency FROM tenant WHERE id = $1))::NUMERIC AS conversion_rate
    FROM
        historical_rates_from_usd
)
SELECT COALESCE(SUM(net_revenue_cents  * cr.conversion_rate), 0)::bigint AS total_net_revenue
FROM bi_revenue_daily
         JOIN conversion_rates cr ON bi_revenue_daily.historical_rate_id = cr.id
WHERE revenue_date BETWEEN $2 AND $3
  AND tenant_id = $1
",
            ))
        }
        pub struct QueryTotalNetRevenueStmt(cornucopia_async::private::Stmt);
        impl QueryTotalNetRevenueStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                start_date: &'a time::Date,
                end_date: &'a time::Date,
            ) -> I64Query<'a, C, i64, 3> {
                I64Query {
                    client,
                    params: [tenant_id, start_date, end_date],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<'a, QueryTotalNetRevenueParams, I64Query<'a, C, i64, 3>, C>
            for QueryTotalNetRevenueStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a QueryTotalNetRevenueParams,
            ) -> I64Query<'a, C, i64, 3> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.start_date,
                    &params.end_date,
                )
            }
        }
        pub fn get_last_mrr_movements() -> GetLastMrrMovementsStmt {
            GetLastMrrMovementsStmt(cornucopia_async::private::Stmt::new(
                "SELECT bi.id,
       bi.movement_type,
       bi.net_mrr_change,
       bi.currency,
       bi.applies_to,
       bi.created_at,
       bi.description,
       bi.invoice_id,
       bi.credit_note_id,
       bi.tenant_id,
       bi.plan_version_id,
       c.id   as customer_id,
       c.name as customer_name,
       s.id   as subscription_id,
       p.name as plan_name
FROM bi_mrr_movement_log bi
         LEFT JOIN invoice i on bi.invoice_id = i.id
         JOIN subscription s on i.subscription_id = s.id
         JOIN plan_version pv on bi.plan_version_id = pv.id
         JOIN plan p on pv.plan_id = p.id
         JOIN customer c on s.customer_id = c.id
WHERE bi.tenant_id = $1
  AND (bi.id < $2 OR $2 IS NULL)
  AND (bi.id > $3 OR $3 IS NULL)
ORDER BY bi.id DESC
LIMIT $4",
            ))
        }
        pub struct GetLastMrrMovementsStmt(cornucopia_async::private::Stmt);
        impl GetLastMrrMovementsStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                before: &'a Option<uuid::Uuid>,
                after: &'a Option<uuid::Uuid>,
                limit: &'a i64,
            ) -> GetLastMrrMovementsQuery<'a, C, GetLastMrrMovements, 4> {
                GetLastMrrMovementsQuery {
                    client,
                    params: [tenant_id, before, after, limit],
                    stmt: &mut self.0,
                    extractor: |row| GetLastMrrMovementsBorrowed {
                        id: row.get(0),
                        movement_type: row.get(1),
                        net_mrr_change: row.get(2),
                        currency: row.get(3),
                        applies_to: row.get(4),
                        created_at: row.get(5),
                        description: row.get(6),
                        invoice_id: row.get(7),
                        credit_note_id: row.get(8),
                        tenant_id: row.get(9),
                        plan_version_id: row.get(10),
                        customer_id: row.get(11),
                        customer_name: row.get(12),
                        subscription_id: row.get(13),
                        plan_name: row.get(14),
                    },
                    mapper: |it| <GetLastMrrMovements>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetLastMrrMovementsParams,
                GetLastMrrMovementsQuery<'a, C, GetLastMrrMovements, 4>,
                C,
            > for GetLastMrrMovementsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetLastMrrMovementsParams,
            ) -> GetLastMrrMovementsQuery<'a, C, GetLastMrrMovements, 4> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.before,
                    &params.after,
                    &params.limit,
                )
            }
        }
        pub fn query_revenue_trend() -> QueryRevenueTrendStmt {
            QueryRevenueTrendStmt(cornucopia_async :: private :: Stmt :: new("WITH period AS (SELECT CURRENT_DATE - INTERVAL '1 day' * $1::integer       AS start_current_period,
                       CURRENT_DATE - INTERVAL '1 day' * ($1::integer * 2) AS start_previous_period),
     conversion_rates AS (
         SELECT
             id,
             (rates->>(SELECT currency FROM tenant WHERE id = $2))::NUMERIC AS conversion_rate
         FROM
             historical_rates_from_usd
     ),
     revenue_ytd AS (SELECT COALESCE(SUM(net_revenue_cents * cr.conversion_rate), 0)::bigint AS total_ytd
                     FROM bi_revenue_daily
                              JOIN conversion_rates cr ON bi_revenue_daily.historical_rate_id = cr.id
                     WHERE revenue_date BETWEEN DATE_TRUNC('year', CURRENT_DATE) AND CURRENT_DATE
                       AND bi_revenue_daily.tenant_id = $2),
     current_period AS (
         SELECT
             COALESCE(SUM(net_revenue_cents_usd * cr.conversion_rate), 0)::bigint AS total
         FROM
             bi_revenue_daily
                 JOIN
             period ON revenue_date BETWEEN period.start_current_period AND CURRENT_DATE
                 JOIN
             conversion_rates cr ON bi_revenue_daily.historical_rate_id = cr.id
         WHERE
             bi_revenue_daily.tenant_id = $2
     ),
     previous_period AS (
         SELECT
             COALESCE(SUM(net_revenue_cents_usd * cr.conversion_rate), 0)::bigint AS total
         FROM
             bi_revenue_daily
                 JOIN
             period ON revenue_date BETWEEN period.start_previous_period AND period.start_current_period
                 JOIN
             conversion_rates cr ON bi_revenue_daily.historical_rate_id = cr.id
         WHERE
             bi_revenue_daily.tenant_id = $2
     )
SELECT COALESCE(revenue_ytd.total_ytd, 0) AS total_ytd,
       COALESCE(current_period.total, 0)  AS total_current_period,
       COALESCE(previous_period.total, 0) AS total_previous_period
FROM revenue_ytd,
     current_period,
     previous_period"))
        }
        pub struct QueryRevenueTrendStmt(cornucopia_async::private::Stmt);
        impl QueryRevenueTrendStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                period_days: &'a i32,
                tenant_id: &'a uuid::Uuid,
            ) -> QueryRevenueTrendQuery<'a, C, QueryRevenueTrend, 2> {
                QueryRevenueTrendQuery {
                    client,
                    params: [period_days, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| QueryRevenueTrend {
                        total_ytd: row.get(0),
                        total_current_period: row.get(1),
                        total_previous_period: row.get(2),
                    },
                    mapper: |it| <QueryRevenueTrend>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                QueryRevenueTrendParams,
                QueryRevenueTrendQuery<'a, C, QueryRevenueTrend, 2>,
                C,
            > for QueryRevenueTrendStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a QueryRevenueTrendParams,
            ) -> QueryRevenueTrendQuery<'a, C, QueryRevenueTrend, 2> {
                self.bind(client, &params.period_days, &params.tenant_id)
            }
        }
        pub fn count_active_subscriptions() -> CountActiveSubscriptionsStmt {
            CountActiveSubscriptionsStmt(cornucopia_async::private::Stmt::new(
                "SELECT COUNT(*) AS total
FROM subscription
WHERE tenant_id = $1
  AND status = 'ACTIVE'",
            ))
        }
        pub struct CountActiveSubscriptionsStmt(cornucopia_async::private::Stmt);
        impl CountActiveSubscriptionsStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> I64Query<'a, C, i64, 1> {
                I64Query {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        pub fn query_pending_invoices() -> QueryPendingInvoicesStmt {
            QueryPendingInvoicesStmt(cornucopia_async :: private :: Stmt :: new("WITH tenant_currency AS (
    SELECT currency FROM tenant WHERE id = $1
),
     latest_rate AS (
         SELECT
             rates
         FROM
             historical_rates_from_usd
         WHERE
             date  <= CURRENT_DATE
         ORDER BY date DESC
         LIMIT 1
     ),
     converted_invoices AS (
         SELECT
             convert_currency(
                     i.amount_cents,
                     (SELECT (rates->>i.currency)::NUMERIC FROM latest_rate),
                     (SELECT (rates->>(SELECT currency FROM tenant_currency))::NUMERIC FROM latest_rate)
             )::BIGINT AS converted_amount_cents
         FROM
             invoice i,
             latest_rate,
             tenant_currency
         WHERE
             i.tenant_id = $1
           AND i.status = 'PENDING'
     )
SELECT
    COUNT(*)::integer AS total,
    COALESCE(SUM(converted_amount_cents), 0) AS total_cents
FROM
    converted_invoices"))
        }
        pub struct QueryPendingInvoicesStmt(cornucopia_async::private::Stmt);
        impl QueryPendingInvoicesStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> QueryPendingInvoicesQuery<'a, C, QueryPendingInvoices, 1> {
                QueryPendingInvoicesQuery {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| QueryPendingInvoices {
                        total: row.get(0),
                        total_cents: row.get(1),
                    },
                    mapper: |it| <QueryPendingInvoices>::from(it),
                }
            }
        }
        pub fn daily_new_signups_30_days() -> DailyNewSignups30DaysStmt {
            DailyNewSignups30DaysStmt(cornucopia_async :: private :: Stmt :: new("WITH date_series AS (SELECT DATE(current_date - INTERVAL '1 day' * generate_series(0, 29)) AS date),
     daily_signups AS (SELECT DATE(created_at) AS signup_date,
                              COUNT(*)         AS daily_signups
                       FROM customer
                       WHERE tenant_id = $1
                         AND created_at >= CURRENT_DATE - INTERVAL '30 days'
                       GROUP BY signup_date)
SELECT ds.date                                                                        as signup_date,
       COALESCE(d.daily_signups, 0)                                                   AS daily_signups,
       COALESCE(SUM(COALESCE(d.daily_signups, 0)) OVER (ORDER BY ds.date), 0)::bigint AS total_signups_over_30_days
FROM date_series ds
         LEFT JOIN daily_signups d ON ds.date = d.signup_date
ORDER BY ds.date"))
        }
        pub struct DailyNewSignups30DaysStmt(cornucopia_async::private::Stmt);
        impl DailyNewSignups30DaysStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> DailyNewSignups30DaysQuery<'a, C, DailyNewSignups30Days, 1> {
                DailyNewSignups30DaysQuery {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| DailyNewSignups30Days {
                        signup_date: row.get(0),
                        daily_signups: row.get(1),
                        total_signups_over_30_days: row.get(2),
                    },
                    mapper: |it| <DailyNewSignups30Days>::from(it),
                }
            }
        }
        pub fn new_signups_trend_30_days() -> NewSignupsTrend30DaysStmt {
            NewSignupsTrend30DaysStmt(cornucopia_async :: private :: Stmt :: new("WITH signup_counts AS (SELECT DATE(created_at) AS signup_date,
                              COUNT(*)         AS daily_signups
                       FROM customer
                       WHERE tenant_id = $1
                         AND created_at >= CURRENT_DATE - INTERVAL '60 days'
                       GROUP BY signup_date)
SELECT COALESCE(SUM(daily_signups) FILTER (WHERE signup_date > CURRENT_DATE - INTERVAL '30 days'),
                0)::bigint                                                                                    AS total_last_30_days,
       COALESCE(SUM(daily_signups) FILTER (WHERE signup_date <= CURRENT_DATE - INTERVAL '30 days' AND
                                                 signup_date > CURRENT_DATE - INTERVAL '60 days'),
                0)::bigint                                                                                    AS total_previous_30_days
FROM signup_counts"))
        }
        pub struct NewSignupsTrend30DaysStmt(cornucopia_async::private::Stmt);
        impl NewSignupsTrend30DaysStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> NewSignupsTrend30DaysQuery<'a, C, NewSignupsTrend30Days, 1> {
                NewSignupsTrend30DaysQuery {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| NewSignupsTrend30Days {
                        total_last_30_days: row.get(0),
                        total_previous_30_days: row.get(1),
                    },
                    mapper: |it| <NewSignupsTrend30Days>::from(it),
                }
            }
        }
        pub fn get_all_time_trial_conversion_rate() -> GetAllTimeTrialConversionRateStmt {
            GetAllTimeTrialConversionRateStmt(cornucopia_async :: private :: Stmt :: new("SELECT CASE
           WHEN COUNT(*) > 0 THEN
               ROUND((COUNT(*) FILTER (WHERE s.activated_at IS NOT NULL)::DECIMAL / COUNT(*)) * 100, 2)
           ELSE
               0
           END AS all_time_conversion_rate_percentage
FROM subscription s
WHERE s.tenant_id = $1
  AND s.trial_start_date IS NOT NULL"))
        }
        pub struct GetAllTimeTrialConversionRateStmt(cornucopia_async::private::Stmt);
        impl GetAllTimeTrialConversionRateStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> RustdecimalDecimalQuery<'a, C, rust_decimal::Decimal, 1> {
                RustdecimalDecimalQuery {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        pub fn query_trial_to_paid_conversion_over_time() -> QueryTrialToPaidConversionOverTimeStmt
        {
            QueryTrialToPaidConversionOverTimeStmt(cornucopia_async :: private :: Stmt :: new("WITH month_series AS (SELECT generate_series(
                                     DATE_TRUNC('month', COALESCE(MIN(trial_start_date), CURRENT_DATE)),
                                     CURRENT_DATE,
                                     '1 month'
                             ) AS month
                      FROM subscription
                      WHERE tenant_id = $1),
     monthly_trials AS (SELECT ms.month,
                               COALESCE(COUNT(s.trial_start_date), 0)                                                AS total_trials,
                               COALESCE(COUNT(s.activated_at)
                                        FILTER (WHERE s.activated_at - s.trial_start_date <= INTERVAL '30 days'),
                                        0)                                                                           AS conversions_30,
                               COALESCE(COUNT(s.activated_at)
                                        FILTER (WHERE s.activated_at - s.trial_start_date <= INTERVAL '90 days'),
                                        0)                                                                           AS conversions_90,
                               COALESCE(COUNT(s.activated_at), 0)                                                    AS conversions
                        FROM month_series ms
                                 LEFT JOIN subscription s ON DATE_TRUNC('month', s.trial_start_date) = ms.month
                            AND s.tenant_id = $1
                        GROUP BY ms.month
                        ORDER BY ms.month)
SELECT month,
       total_trials,
       conversions,
       CASE
           WHEN total_trials > 0 THEN ROUND((conversions::DECIMAL / total_trials) * 100, 2)
           ELSE 0 END                                                                                      AS conversion_rate_percentage,
       conversions_30,
       CASE
           WHEN total_trials > 0 THEN ROUND((conversions_30::DECIMAL / total_trials) * 100, 2)
           ELSE 0 END                                                                                      AS conversion_rate_30_percentage,
       conversions_90,
       CASE
           WHEN total_trials > 0 THEN ROUND((conversions_90::DECIMAL / total_trials) * 100, 2)
           ELSE 0 END                                                                                      AS conversion_rate_90_percentage
FROM monthly_trials"))
        }
        pub struct QueryTrialToPaidConversionOverTimeStmt(cornucopia_async::private::Stmt);
        impl QueryTrialToPaidConversionOverTimeStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant: &'a uuid::Uuid,
            ) -> QueryTrialToPaidConversionOverTimeQuery<'a, C, QueryTrialToPaidConversionOverTime, 1>
            {
                QueryTrialToPaidConversionOverTimeQuery {
                    client,
                    params: [tenant],
                    stmt: &mut self.0,
                    extractor: |row| QueryTrialToPaidConversionOverTime {
                        month: row.get(0),
                        total_trials: row.get(1),
                        conversions: row.get(2),
                        conversion_rate_percentage: row.get(3),
                        conversions_30: row.get(4),
                        conversion_rate_30_percentage: row.get(5),
                        conversions_90: row.get(6),
                        conversion_rate_90_percentage: row.get(7),
                    },
                    mapper: |it| <QueryTrialToPaidConversionOverTime>::from(it),
                }
            }
        }
    }
    pub mod subscriptions {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateSubscriptionParams<T1: cornucopia_async::JsonSql> {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub created_by: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub status: super::super::types::public::SubscriptionStatusEnum,
            pub billing_start: time::Date,
            pub billing_end: Option<time::Date>,
            pub billing_day: i16,
            pub effective_billing_period: super::super::types::public::BillingPeriodEnum,
            pub parameters: Option<T1>,
            pub net_terms: i32,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct ListSubscriptionsParams {
            pub tenant_id: uuid::Uuid,
            pub plan_id: Option<uuid::Uuid>,
            pub customer_id: Option<uuid::Uuid>,
            pub limit: i64,
            pub offset: i64,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct SubscriptionByIdParams {
            pub subscription_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct UpdateSubscriptionStatusParams {
            pub status: super::super::types::public::SubscriptionStatusEnum,
            pub id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct SubscriptionToInvoice {
            pub subscription_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub plan_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub billing_start_date: time::Date,
            pub billing_end_date: Option<time::Date>,
            pub billing_day: i16,
            pub effective_billing_period: super::super::types::public::BillingPeriodEnum,
            pub input_parameters: serde_json::Value,
            pub currency: String,
            pub net_terms: i32,
            pub version: i32,
        }
        pub struct SubscriptionToInvoiceBorrowed<'a> {
            pub subscription_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub plan_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub billing_start_date: time::Date,
            pub billing_end_date: Option<time::Date>,
            pub billing_day: i16,
            pub effective_billing_period: super::super::types::public::BillingPeriodEnum,
            pub input_parameters: postgres_types::Json<&'a serde_json::value::RawValue>,
            pub currency: &'a str,
            pub net_terms: i32,
            pub version: i32,
        }
        impl<'a> From<SubscriptionToInvoiceBorrowed<'a>> for SubscriptionToInvoice {
            fn from(
                SubscriptionToInvoiceBorrowed {
                    subscription_id,
                    tenant_id,
                    customer_id,
                    plan_id,
                    plan_version_id,
                    billing_start_date,
                    billing_end_date,
                    billing_day,
                    effective_billing_period,
                    input_parameters,
                    currency,
                    net_terms,
                    version,
                }: SubscriptionToInvoiceBorrowed<'a>,
            ) -> Self {
                Self {
                    subscription_id,
                    tenant_id,
                    customer_id,
                    plan_id,
                    plan_version_id,
                    billing_start_date,
                    billing_end_date,
                    billing_day,
                    effective_billing_period,
                    input_parameters: serde_json::from_str(input_parameters.0.get()).unwrap(),
                    currency: currency.into(),
                    net_terms,
                    version,
                }
            }
        }
        pub struct SubscriptionToInvoiceQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> SubscriptionToInvoiceBorrowed,
            mapper: fn(SubscriptionToInvoiceBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> SubscriptionToInvoiceQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(SubscriptionToInvoiceBorrowed) -> R,
            ) -> SubscriptionToInvoiceQuery<'a, C, R, N> {
                SubscriptionToInvoiceQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct GetSubscriptionCurrentPeriod {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub billing_start_date: time::Date,
            pub billing_end_date: Option<time::Date>,
            pub billing_day: i16,
            pub effective_billing_period: super::super::types::public::BillingPeriodEnum,
            pub input_parameters: serde_json::Value,
            pub customer_id: uuid::Uuid,
            pub customer_external_id: Option<String>,
            pub currency: String,
            pub net_terms: i32,
        }
        pub struct GetSubscriptionCurrentPeriodBorrowed<'a> {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub billing_start_date: time::Date,
            pub billing_end_date: Option<time::Date>,
            pub billing_day: i16,
            pub effective_billing_period: super::super::types::public::BillingPeriodEnum,
            pub input_parameters: postgres_types::Json<&'a serde_json::value::RawValue>,
            pub customer_id: uuid::Uuid,
            pub customer_external_id: Option<&'a str>,
            pub currency: &'a str,
            pub net_terms: i32,
        }
        impl<'a> From<GetSubscriptionCurrentPeriodBorrowed<'a>> for GetSubscriptionCurrentPeriod {
            fn from(
                GetSubscriptionCurrentPeriodBorrowed {
                    id,
                    tenant_id,
                    plan_version_id,
                    billing_start_date,
                    billing_end_date,
                    billing_day,
                    effective_billing_period,
                    input_parameters,
                    customer_id,
                    customer_external_id,
                    currency,
                    net_terms,
                }: GetSubscriptionCurrentPeriodBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    tenant_id,
                    plan_version_id,
                    billing_start_date,
                    billing_end_date,
                    billing_day,
                    effective_billing_period,
                    input_parameters: serde_json::from_str(input_parameters.0.get()).unwrap(),
                    customer_id,
                    customer_external_id: customer_external_id.map(|v| v.into()),
                    currency: currency.into(),
                    net_terms,
                }
            }
        }
        pub struct GetSubscriptionCurrentPeriodQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> GetSubscriptionCurrentPeriodBorrowed,
            mapper: fn(GetSubscriptionCurrentPeriodBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> GetSubscriptionCurrentPeriodQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(GetSubscriptionCurrentPeriodBorrowed) -> R,
            ) -> GetSubscriptionCurrentPeriodQuery<'a, C, R, N> {
                GetSubscriptionCurrentPeriodQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub struct UuidUuidQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> uuid::Uuid,
            mapper: fn(uuid::Uuid) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> UuidUuidQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(uuid::Uuid) -> R) -> UuidUuidQuery<'a, C, R, N> {
                UuidUuidQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct SubscriptionList {
            pub subscription_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub billing_start_date: time::Date,
            pub billing_end_date: Option<time::Date>,
            pub billing_day: i16,
            pub effective_billing_period: super::super::types::public::BillingPeriodEnum,
            pub input_parameters: serde_json::Value,
            pub net_terms: i32,
            pub currency: String,
            pub version: i32,
            pub customer_name: String,
            pub plan_id: uuid::Uuid,
            pub plan_name: String,
            pub total_count: i64,
        }
        pub struct SubscriptionListBorrowed<'a> {
            pub subscription_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub billing_start_date: time::Date,
            pub billing_end_date: Option<time::Date>,
            pub billing_day: i16,
            pub effective_billing_period: super::super::types::public::BillingPeriodEnum,
            pub input_parameters: postgres_types::Json<&'a serde_json::value::RawValue>,
            pub net_terms: i32,
            pub currency: &'a str,
            pub version: i32,
            pub customer_name: &'a str,
            pub plan_id: uuid::Uuid,
            pub plan_name: &'a str,
            pub total_count: i64,
        }
        impl<'a> From<SubscriptionListBorrowed<'a>> for SubscriptionList {
            fn from(
                SubscriptionListBorrowed {
                    subscription_id,
                    tenant_id,
                    customer_id,
                    plan_version_id,
                    billing_start_date,
                    billing_end_date,
                    billing_day,
                    effective_billing_period,
                    input_parameters,
                    net_terms,
                    currency,
                    version,
                    customer_name,
                    plan_id,
                    plan_name,
                    total_count,
                }: SubscriptionListBorrowed<'a>,
            ) -> Self {
                Self {
                    subscription_id,
                    tenant_id,
                    customer_id,
                    plan_version_id,
                    billing_start_date,
                    billing_end_date,
                    billing_day,
                    effective_billing_period,
                    input_parameters: serde_json::from_str(input_parameters.0.get()).unwrap(),
                    net_terms,
                    currency: currency.into(),
                    version,
                    customer_name: customer_name.into(),
                    plan_id,
                    plan_name: plan_name.into(),
                    total_count,
                }
            }
        }
        pub struct SubscriptionListQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> SubscriptionListBorrowed,
            mapper: fn(SubscriptionListBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> SubscriptionListQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(SubscriptionListBorrowed) -> R,
            ) -> SubscriptionListQuery<'a, C, R, N> {
                SubscriptionListQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct Subscription {
            pub subscription_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub billing_start_date: time::Date,
            pub billing_end_date: Option<time::Date>,
            pub billing_day: i16,
            pub effective_billing_period: super::super::types::public::BillingPeriodEnum,
            pub input_parameters: serde_json::Value,
            pub net_terms: i32,
            pub currency: String,
            pub version: i32,
            pub customer_name: String,
            pub plan_id: uuid::Uuid,
            pub plan_name: String,
        }
        pub struct SubscriptionBorrowed<'a> {
            pub subscription_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub customer_id: uuid::Uuid,
            pub plan_version_id: uuid::Uuid,
            pub billing_start_date: time::Date,
            pub billing_end_date: Option<time::Date>,
            pub billing_day: i16,
            pub effective_billing_period: super::super::types::public::BillingPeriodEnum,
            pub input_parameters: postgres_types::Json<&'a serde_json::value::RawValue>,
            pub net_terms: i32,
            pub currency: &'a str,
            pub version: i32,
            pub customer_name: &'a str,
            pub plan_id: uuid::Uuid,
            pub plan_name: &'a str,
        }
        impl<'a> From<SubscriptionBorrowed<'a>> for Subscription {
            fn from(
                SubscriptionBorrowed {
                    subscription_id,
                    tenant_id,
                    customer_id,
                    plan_version_id,
                    billing_start_date,
                    billing_end_date,
                    billing_day,
                    effective_billing_period,
                    input_parameters,
                    net_terms,
                    currency,
                    version,
                    customer_name,
                    plan_id,
                    plan_name,
                }: SubscriptionBorrowed<'a>,
            ) -> Self {
                Self {
                    subscription_id,
                    tenant_id,
                    customer_id,
                    plan_version_id,
                    billing_start_date,
                    billing_end_date,
                    billing_day,
                    effective_billing_period,
                    input_parameters: serde_json::from_str(input_parameters.0.get()).unwrap(),
                    net_terms,
                    currency: currency.into(),
                    version,
                    customer_name: customer_name.into(),
                    plan_id,
                    plan_name: plan_name.into(),
                }
            }
        }
        pub struct SubscriptionQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> SubscriptionBorrowed,
            mapper: fn(SubscriptionBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> SubscriptionQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(SubscriptionBorrowed) -> R,
            ) -> SubscriptionQuery<'a, C, R, N> {
                SubscriptionQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn subscription_to_invoice_candidates() -> SubscriptionToInvoiceCandidatesStmt {
            SubscriptionToInvoiceCandidatesStmt(cornucopia_async::private::Stmt::new(
                "SELECT s.id AS subscription_id,
       s.tenant_id,
       s.customer_id,
       pp.plan_id,
       s.plan_version_id,
       s.billing_start_date,
       s.billing_end_date,
       s.billing_day,
       s.effective_billing_period,
       s.input_parameters,
       pp.currency,
       pp.net_terms,
       pp.version
FROM subscription s
         JOIN plan_version pp ON s.plan_version_id = pp.id
         LEFT JOIN invoice i ON s.id = i.subscription_id AND i.invoice_date > $1
where (s.billing_end_date is null OR s.billing_end_date < $1)
  AND i.id IS NULL",
            ))
        }
        pub struct SubscriptionToInvoiceCandidatesStmt(cornucopia_async::private::Stmt);
        impl SubscriptionToInvoiceCandidatesStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                input_date: &'a time::Date,
            ) -> SubscriptionToInvoiceQuery<'a, C, SubscriptionToInvoice, 1> {
                SubscriptionToInvoiceQuery {
                    client,
                    params: [input_date],
                    stmt: &mut self.0,
                    extractor: |row| SubscriptionToInvoiceBorrowed {
                        subscription_id: row.get(0),
                        tenant_id: row.get(1),
                        customer_id: row.get(2),
                        plan_id: row.get(3),
                        plan_version_id: row.get(4),
                        billing_start_date: row.get(5),
                        billing_end_date: row.get(6),
                        billing_day: row.get(7),
                        effective_billing_period: row.get(8),
                        input_parameters: row.get(9),
                        currency: row.get(10),
                        net_terms: row.get(11),
                        version: row.get(12),
                    },
                    mapper: |it| <SubscriptionToInvoice>::from(it),
                }
            }
        }
        pub fn get_subscription_current_period() -> GetSubscriptionCurrentPeriodStmt {
            GetSubscriptionCurrentPeriodStmt(cornucopia_async::private::Stmt::new(
                "SELECT s.id,
       s.tenant_id,
       s.plan_version_id,
       s.billing_start_date,
       s.billing_end_date,
       s.billing_day,
       s.effective_billing_period,
       s.input_parameters,
       s.customer_id,
       c.alias as customer_external_id,
--        cbp.current_period_start_date,
--        cbp.current_period_end_date,
--        cbp.current_period_idx::integer,
       pp.currency,
       s.net_terms
FROM subscription s
         JOIN plan_version pp ON s.plan_version_id = pp.id
         JOIN customer c ON s.customer_id = c.id
         JOIN current_billing_period cbp ON s.id = cbp.subscription_id
WHERE s.id = $1",
            ))
        }
        pub struct GetSubscriptionCurrentPeriodStmt(cornucopia_async::private::Stmt);
        impl GetSubscriptionCurrentPeriodStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                subscription_id: &'a uuid::Uuid,
            ) -> GetSubscriptionCurrentPeriodQuery<'a, C, GetSubscriptionCurrentPeriod, 1>
            {
                GetSubscriptionCurrentPeriodQuery {
                    client,
                    params: [subscription_id],
                    stmt: &mut self.0,
                    extractor: |row| GetSubscriptionCurrentPeriodBorrowed {
                        id: row.get(0),
                        tenant_id: row.get(1),
                        plan_version_id: row.get(2),
                        billing_start_date: row.get(3),
                        billing_end_date: row.get(4),
                        billing_day: row.get(5),
                        effective_billing_period: row.get(6),
                        input_parameters: row.get(7),
                        customer_id: row.get(8),
                        customer_external_id: row.get(9),
                        currency: row.get(10),
                        net_terms: row.get(11),
                    },
                    mapper: |it| <GetSubscriptionCurrentPeriod>::from(it),
                }
            }
        }
        pub fn create_subscription() -> CreateSubscriptionStmt {
            CreateSubscriptionStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO subscription (id,
                          tenant_id,
                          customer_id,
                          created_by,
                          plan_version_id,
                          status,
                          billing_start_date,
                          billing_end_date,
                          billing_day,
                          effective_billing_period,
                          input_parameters,
                          net_terms)
VALUES ($1,
        $2,
        $3,
        $4,
        $5,
        $6,
        $7,
        $8,
        $9,
        $10,
        $11,
        $12)
RETURNING id
",
            ))
        }
        pub struct CreateSubscriptionStmt(cornucopia_async::private::Stmt);
        impl CreateSubscriptionStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::JsonSql>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
                customer_id: &'a uuid::Uuid,
                created_by: &'a uuid::Uuid,
                plan_version_id: &'a uuid::Uuid,
                status: &'a super::super::types::public::SubscriptionStatusEnum,
                billing_start: &'a time::Date,
                billing_end: &'a Option<time::Date>,
                billing_day: &'a i16,
                effective_billing_period: &'a super::super::types::public::BillingPeriodEnum,
                parameters: &'a Option<T1>,
                net_terms: &'a i32,
            ) -> UuidUuidQuery<'a, C, uuid::Uuid, 12> {
                UuidUuidQuery {
                    client,
                    params: [
                        id,
                        tenant_id,
                        customer_id,
                        created_by,
                        plan_version_id,
                        status,
                        billing_start,
                        billing_end,
                        billing_day,
                        effective_billing_period,
                        parameters,
                        net_terms,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::JsonSql>
            cornucopia_async::Params<
                'a,
                CreateSubscriptionParams<T1>,
                UuidUuidQuery<'a, C, uuid::Uuid, 12>,
                C,
            > for CreateSubscriptionStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateSubscriptionParams<T1>,
            ) -> UuidUuidQuery<'a, C, uuid::Uuid, 12> {
                self.bind(
                    client,
                    &params.id,
                    &params.tenant_id,
                    &params.customer_id,
                    &params.created_by,
                    &params.plan_version_id,
                    &params.status,
                    &params.billing_start,
                    &params.billing_end,
                    &params.billing_day,
                    &params.effective_billing_period,
                    &params.parameters,
                    &params.net_terms,
                )
            }
        }
        pub fn list_subscriptions() -> ListSubscriptionsStmt {
            ListSubscriptionsStmt(cornucopia_async::private::Stmt::new(
                "SELECT s.id             AS subscription_id,
       s.tenant_id,
       s.customer_id,
       s.plan_version_id,
       s.billing_start_date,
       s.billing_end_date,
       s.billing_day,
       s.effective_billing_period,
       s.input_parameters,
       s.net_terms,
       pp.currency,
       pp.version,
       c.name           AS customer_name,
       p.id             AS plan_id,
       p.name           AS plan_name,
       count(*) OVER () AS total_count
FROM subscription s
         JOIN plan_version pp   ON s.plan_version_id = pp.id
         JOIN plan p            ON pp.plan_id = p.id
         JOIN customer c        ON s.customer_id = c.id
WHERE s.tenant_id = $1
  AND ($2 :: UUID         IS NULL OR pp.plan_id = $2)
  AND ($3 :: UUID     IS NULL OR s.customer_id = $3)
ORDER BY s.id DESC
    LIMIT $4 OFFSET $5",
            ))
        }
        pub struct ListSubscriptionsStmt(cornucopia_async::private::Stmt);
        impl ListSubscriptionsStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
                plan_id: &'a Option<uuid::Uuid>,
                customer_id: &'a Option<uuid::Uuid>,
                limit: &'a i64,
                offset: &'a i64,
            ) -> SubscriptionListQuery<'a, C, SubscriptionList, 5> {
                SubscriptionListQuery {
                    client,
                    params: [tenant_id, plan_id, customer_id, limit, offset],
                    stmt: &mut self.0,
                    extractor: |row| SubscriptionListBorrowed {
                        subscription_id: row.get(0),
                        tenant_id: row.get(1),
                        customer_id: row.get(2),
                        plan_version_id: row.get(3),
                        billing_start_date: row.get(4),
                        billing_end_date: row.get(5),
                        billing_day: row.get(6),
                        effective_billing_period: row.get(7),
                        input_parameters: row.get(8),
                        net_terms: row.get(9),
                        currency: row.get(10),
                        version: row.get(11),
                        customer_name: row.get(12),
                        plan_id: row.get(13),
                        plan_name: row.get(14),
                        total_count: row.get(15),
                    },
                    mapper: |it| <SubscriptionList>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                ListSubscriptionsParams,
                SubscriptionListQuery<'a, C, SubscriptionList, 5>,
                C,
            > for ListSubscriptionsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListSubscriptionsParams,
            ) -> SubscriptionListQuery<'a, C, SubscriptionList, 5> {
                self.bind(
                    client,
                    &params.tenant_id,
                    &params.plan_id,
                    &params.customer_id,
                    &params.limit,
                    &params.offset,
                )
            }
        }
        pub fn subscription_by_id() -> SubscriptionByIdStmt {
            SubscriptionByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT s.id   AS subscription_id,
       s.tenant_id,
       s.customer_id,
       s.plan_version_id,
       s.billing_start_date,
       s.billing_end_date,
       s.billing_day,
       s.effective_billing_period,
       s.input_parameters,
       s.net_terms,
       pp.currency,
       pp.version,
       c.name           AS customer_name,
       p.id             AS plan_id,
       p.name           AS plan_name
FROM subscription s
         JOIN plan_version pp   ON s.plan_version_id = pp.id
         JOIN plan p            ON pp.plan_id = p.id
         JOIN customer c        ON s.customer_id = c.id
WHERE s.id = $1
  AND s.tenant_id = $2",
            ))
        }
        pub struct SubscriptionByIdStmt(cornucopia_async::private::Stmt);
        impl SubscriptionByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                subscription_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> SubscriptionQuery<'a, C, Subscription, 2> {
                SubscriptionQuery {
                    client,
                    params: [subscription_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| SubscriptionBorrowed {
                        subscription_id: row.get(0),
                        tenant_id: row.get(1),
                        customer_id: row.get(2),
                        plan_version_id: row.get(3),
                        billing_start_date: row.get(4),
                        billing_end_date: row.get(5),
                        billing_day: row.get(6),
                        effective_billing_period: row.get(7),
                        input_parameters: row.get(8),
                        net_terms: row.get(9),
                        currency: row.get(10),
                        version: row.get(11),
                        customer_name: row.get(12),
                        plan_id: row.get(13),
                        plan_name: row.get(14),
                    },
                    mapper: |it| <Subscription>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                SubscriptionByIdParams,
                SubscriptionQuery<'a, C, Subscription, 2>,
                C,
            > for SubscriptionByIdStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a SubscriptionByIdParams,
            ) -> SubscriptionQuery<'a, C, Subscription, 2> {
                self.bind(client, &params.subscription_id, &params.tenant_id)
            }
        }
        pub fn update_subscription_status() -> UpdateSubscriptionStatusStmt {
            UpdateSubscriptionStatusStmt(cornucopia_async::private::Stmt::new(
                "UPDATE subscription
SET status = $1
WHERE id = $2",
            ))
        }
        pub struct UpdateSubscriptionStatusStmt(cornucopia_async::private::Stmt);
        impl UpdateSubscriptionStatusStmt {
            pub async fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                status: &'a super::super::types::public::SubscriptionStatusEnum,
                id: &'a uuid::Uuid,
            ) -> Result<u64, tokio_postgres::Error> {
                let stmt = self.0.prepare(client).await?;
                client.execute(stmt, &[status, id]).await
            }
        }
        impl<'a, C: GenericClient + Send + Sync>
            cornucopia_async::Params<
                'a,
                UpdateSubscriptionStatusParams,
                std::pin::Pin<
                    Box<
                        dyn futures::Future<Output = Result<u64, tokio_postgres::Error>>
                            + Send
                            + 'a,
                    >,
                >,
                C,
            > for UpdateSubscriptionStatusStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpdateSubscriptionStatusParams,
            ) -> std::pin::Pin<
                Box<dyn futures::Future<Output = Result<u64, tokio_postgres::Error>> + Send + 'a>,
            > {
                Box::pin(self.bind(client, &params.status, &params.id))
            }
        }
    }
    pub mod tenants {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateTenantForUserParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub slug: T2,
            pub user_id: uuid::Uuid,
            pub currency: T3,
        }
        #[derive(Debug)]
        pub struct CreateTenantForOrgParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub name: T1,
            pub slug: T2,
            pub organization_id: uuid::Uuid,
            pub currency: T3,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct Tenant {
            pub id: uuid::Uuid,
            pub name: String,
            pub slug: String,
            pub currency: String,
        }
        pub struct TenantBorrowed<'a> {
            pub id: uuid::Uuid,
            pub name: &'a str,
            pub slug: &'a str,
            pub currency: &'a str,
        }
        impl<'a> From<TenantBorrowed<'a>> for Tenant {
            fn from(
                TenantBorrowed {
                    id,
                    name,
                    slug,
                    currency,
                }: TenantBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    name: name.into(),
                    slug: slug.into(),
                    currency: currency.into(),
                }
            }
        }
        pub struct TenantQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> TenantBorrowed,
            mapper: fn(TenantBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> TenantQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(TenantBorrowed) -> R) -> TenantQuery<'a, C, R, N> {
                TenantQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn tenants_per_user() -> TenantsPerUserStmt {
            TenantsPerUserStmt(cornucopia_async::private::Stmt::new(
                "SELECT t.id, t.name, t.slug, t.currency
FROM tenant t
JOIN organization o ON t.organization_id = o.id
JOIN organization_member om ON om.organization_id = o.id
JOIN \"user\" u ON u.id = om.user_id
WHERE u.id = $1",
            ))
        }
        pub struct TenantsPerUserStmt(cornucopia_async::private::Stmt);
        impl TenantsPerUserStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                user_id: &'a uuid::Uuid,
            ) -> TenantQuery<'a, C, Tenant, 1> {
                TenantQuery {
                    client,
                    params: [user_id],
                    stmt: &mut self.0,
                    extractor: |row| TenantBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                        currency: row.get(3),
                    },
                    mapper: |it| <Tenant>::from(it),
                }
            }
        }
        pub fn get_tenant_by_slug() -> GetTenantBySlugStmt {
            GetTenantBySlugStmt(cornucopia_async::private::Stmt::new(
                "SELECT t.id, t.name, t.slug, t.currency
FROM tenant AS t
WHERE t.slug = $1",
            ))
        }
        pub struct GetTenantBySlugStmt(cornucopia_async::private::Stmt);
        impl GetTenantBySlugStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                tenant_slug: &'a T1,
            ) -> TenantQuery<'a, C, Tenant, 1> {
                TenantQuery {
                    client,
                    params: [tenant_slug],
                    stmt: &mut self.0,
                    extractor: |row| TenantBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                        currency: row.get(3),
                    },
                    mapper: |it| <Tenant>::from(it),
                }
            }
        }
        pub fn get_tenant_by_id() -> GetTenantByIdStmt {
            GetTenantByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT t.id, t.name, t.slug, t.currency
FROM tenant AS t
WHERE t.id = $1",
            ))
        }
        pub struct GetTenantByIdStmt(cornucopia_async::private::Stmt);
        impl GetTenantByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> TenantQuery<'a, C, Tenant, 1> {
                TenantQuery {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| TenantBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                        currency: row.get(3),
                    },
                    mapper: |it| <Tenant>::from(it),
                }
            }
        }
        pub fn create_tenant_for_user() -> CreateTenantForUserStmt {
            CreateTenantForUserStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO tenant(id, name, slug, organization_id, currency)
VALUES ($1, $2, $3,
        (SELECT o.id
         FROM organization o
                  JOIN organization_member om ON om.organization_id = o.id
                  JOIN \"user\" u ON u.id = om.user_id
         WHERE u.id = $4 LIMIT 1),
        $5)
RETURNING id, name, slug, currency",
            ))
        }
        pub struct CreateTenantForUserStmt(cornucopia_async::private::Stmt);
        impl CreateTenantForUserStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                slug: &'a T2,
                user_id: &'a uuid::Uuid,
                currency: &'a T3,
            ) -> TenantQuery<'a, C, Tenant, 5> {
                TenantQuery {
                    client,
                    params: [id, name, slug, user_id, currency],
                    stmt: &mut self.0,
                    extractor: |row| TenantBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                        currency: row.get(3),
                    },
                    mapper: |it| <Tenant>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                CreateTenantForUserParams<T1, T2, T3>,
                TenantQuery<'a, C, Tenant, 5>,
                C,
            > for CreateTenantForUserStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateTenantForUserParams<T1, T2, T3>,
            ) -> TenantQuery<'a, C, Tenant, 5> {
                self.bind(
                    client,
                    &params.id,
                    &params.name,
                    &params.slug,
                    &params.user_id,
                    &params.currency,
                )
            }
        }
        pub fn create_tenant_for_org() -> CreateTenantForOrgStmt {
            CreateTenantForOrgStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO tenant(id, name, slug, organization_id, currency)
VALUES ($1, $2, $3, $4, $5)
RETURNING id, name, slug, currency",
            ))
        }
        pub struct CreateTenantForOrgStmt(cornucopia_async::private::Stmt);
        impl CreateTenantForOrgStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                name: &'a T1,
                slug: &'a T2,
                organization_id: &'a uuid::Uuid,
                currency: &'a T3,
            ) -> TenantQuery<'a, C, Tenant, 5> {
                TenantQuery {
                    client,
                    params: [id, name, slug, organization_id, currency],
                    stmt: &mut self.0,
                    extractor: |row| TenantBorrowed {
                        id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                        currency: row.get(3),
                    },
                    mapper: |it| <Tenant>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                CreateTenantForOrgParams<T1, T2, T3>,
                TenantQuery<'a, C, Tenant, 5>,
                C,
            > for CreateTenantForOrgStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateTenantForOrgParams<T1, T2, T3>,
            ) -> TenantQuery<'a, C, Tenant, 5> {
                self.bind(
                    client,
                    &params.id,
                    &params.name,
                    &params.slug,
                    &params.organization_id,
                    &params.currency,
                )
            }
        }
    }
    pub mod users {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct UpsertUserParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub email: T1,
            pub password_hash: Option<T2>,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct CanAccessTenantParams {
            pub user_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetUserRoleParams {
            pub user_id: uuid::Uuid,
            pub organization_id: uuid::Uuid,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetUserRoleByTenantParams {
            pub user_id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug)]
        pub struct GetUserRoleByTenantSlugParams<T1: cornucopia_async::StringSql> {
            pub user_id: uuid::Uuid,
            pub tenant_slug: T1,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct UpsertUser {
            pub id: uuid::Uuid,
            pub email: String,
        }
        pub struct UpsertUserBorrowed<'a> {
            pub id: uuid::Uuid,
            pub email: &'a str,
        }
        impl<'a> From<UpsertUserBorrowed<'a>> for UpsertUser {
            fn from(UpsertUserBorrowed { id, email }: UpsertUserBorrowed<'a>) -> Self {
                Self {
                    id,
                    email: email.into(),
                }
            }
        }
        pub struct UpsertUserQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> UpsertUserBorrowed,
            mapper: fn(UpsertUserBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> UpsertUserQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(UpsertUserBorrowed) -> R,
            ) -> UpsertUserQuery<'a, C, R, N> {
                UpsertUserQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct User {
            pub id: uuid::Uuid,
            pub email: String,
            pub role: super::super::types::public::OrganizationUserRole,
        }
        pub struct UserBorrowed<'a> {
            pub id: uuid::Uuid,
            pub email: &'a str,
            pub role: super::super::types::public::OrganizationUserRole,
        }
        impl<'a> From<UserBorrowed<'a>> for User {
            fn from(UserBorrowed { id, email, role }: UserBorrowed<'a>) -> Self {
                Self {
                    id,
                    email: email.into(),
                    role,
                }
            }
        }
        pub struct UserQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> UserBorrowed,
            mapper: fn(UserBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> UserQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(UserBorrowed) -> R) -> UserQuery<'a, C, R, N> {
                UserQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct UserWithHash {
            pub id: uuid::Uuid,
            pub email: String,
            pub password_hash: Option<String>,
            pub role: super::super::types::public::OrganizationUserRole,
        }
        pub struct UserWithHashBorrowed<'a> {
            pub id: uuid::Uuid,
            pub email: &'a str,
            pub password_hash: Option<&'a str>,
            pub role: super::super::types::public::OrganizationUserRole,
        }
        impl<'a> From<UserWithHashBorrowed<'a>> for UserWithHash {
            fn from(
                UserWithHashBorrowed {
                    id,
                    email,
                    password_hash,
                    role,
                }: UserWithHashBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    email: email.into(),
                    password_hash: password_hash.map(|v| v.into()),
                    role,
                }
            }
        }
        pub struct UserWithHashQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> UserWithHashBorrowed,
            mapper: fn(UserWithHashBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> UserWithHashQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(UserWithHashBorrowed) -> R,
            ) -> UserWithHashQuery<'a, C, R, N> {
                UserWithHashQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub struct BoolQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> bool,
            mapper: fn(bool) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> BoolQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(bool) -> R) -> BoolQuery<'a, C, R, N> {
                BoolQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub struct SuperSuperTypesPublicOrganizationUserRoleQuery<
            'a,
            C: GenericClient,
            T,
            const N: usize,
        > {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor:
                fn(&tokio_postgres::Row) -> super::super::types::public::OrganizationUserRole,
            mapper: fn(super::super::types::public::OrganizationUserRole) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> SuperSuperTypesPublicOrganizationUserRoleQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(super::super::types::public::OrganizationUserRole) -> R,
            ) -> SuperSuperTypesPublicOrganizationUserRoleQuery<'a, C, R, N> {
                SuperSuperTypesPublicOrganizationUserRoleQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq, Copy)]
        pub struct GetUserRoleByTenantSlug {
            pub role: super::super::types::public::OrganizationUserRole,
            pub tenant_id: uuid::Uuid,
        }
        pub struct GetUserRoleByTenantSlugQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> GetUserRoleByTenantSlug,
            mapper: fn(GetUserRoleByTenantSlug) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> GetUserRoleByTenantSlugQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(GetUserRoleByTenantSlug) -> R,
            ) -> GetUserRoleByTenantSlugQuery<'a, C, R, N> {
                GetUserRoleByTenantSlugQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ListUsers {
            pub id: uuid::Uuid,
            pub email: String,
            pub role: super::super::types::public::OrganizationUserRole,
        }
        pub struct ListUsersBorrowed<'a> {
            pub id: uuid::Uuid,
            pub email: &'a str,
            pub role: super::super::types::public::OrganizationUserRole,
        }
        impl<'a> From<ListUsersBorrowed<'a>> for ListUsers {
            fn from(ListUsersBorrowed { id, email, role }: ListUsersBorrowed<'a>) -> Self {
                Self {
                    id,
                    email: email.into(),
                    role,
                }
            }
        }
        pub struct ListUsersQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ListUsersBorrowed,
            mapper: fn(ListUsersBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ListUsersQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(self, mapper: fn(ListUsersBorrowed) -> R) -> ListUsersQuery<'a, C, R, N> {
                ListUsersQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn upsert_user() -> UpsertUserStmt {
            UpsertUserStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO \"user\" (id, email, password_hash)
VALUES ($1, $2, $3)
ON CONFLICT (id) DO UPDATE
    SET email = EXCLUDED.email
RETURNING id,
    email",
            ))
        }
        pub struct UpsertUserStmt(cornucopia_async::private::Stmt);
        impl UpsertUserStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                email: &'a T1,
                password_hash: &'a Option<T2>,
            ) -> UpsertUserQuery<'a, C, UpsertUser, 3> {
                UpsertUserQuery {
                    client,
                    params: [id, email, password_hash],
                    stmt: &mut self.0,
                    extractor: |row| UpsertUserBorrowed {
                        id: row.get(0),
                        email: row.get(1),
                    },
                    mapper: |it| <UpsertUser>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                UpsertUserParams<T1, T2>,
                UpsertUserQuery<'a, C, UpsertUser, 3>,
                C,
            > for UpsertUserStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a UpsertUserParams<T1, T2>,
            ) -> UpsertUserQuery<'a, C, UpsertUser, 3> {
                self.bind(client, &params.id, &params.email, &params.password_hash)
            }
        }
        pub fn get_user_by_id() -> GetUserByIdStmt {
            GetUserByIdStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    id,
    email,
    om.role
FROM
    \"user\"
JOIN organization_member om on \"user\".id = om.user_id
WHERE
    id = $1",
            ))
        }
        pub struct GetUserByIdStmt(cornucopia_async::private::Stmt);
        impl GetUserByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
            ) -> UserQuery<'a, C, User, 1> {
                UserQuery {
                    client,
                    params: [id],
                    stmt: &mut self.0,
                    extractor: |row| UserBorrowed {
                        id: row.get(0),
                        email: row.get(1),
                        role: row.get(2),
                    },
                    mapper: |it| <User>::from(it),
                }
            }
        }
        pub fn get_user_by_email() -> GetUserByEmailStmt {
            GetUserByEmailStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    id,
    email,
    om.role
FROM
    \"user\"
        JOIN organization_member om on \"user\".id = om.user_id
WHERE
    email = $1",
            ))
        }
        pub struct GetUserByEmailStmt(cornucopia_async::private::Stmt);
        impl GetUserByEmailStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                email: &'a T1,
            ) -> UserQuery<'a, C, User, 1> {
                UserQuery {
                    client,
                    params: [email],
                    stmt: &mut self.0,
                    extractor: |row| UserBorrowed {
                        id: row.get(0),
                        email: row.get(1),
                        role: row.get(2),
                    },
                    mapper: |it| <User>::from(it),
                }
            }
        }
        pub fn get_user_hash_by_email() -> GetUserHashByEmailStmt {
            GetUserHashByEmailStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    id,
    email,
    password_hash,
    om.role
FROM
    \"user\"
        JOIN organization_member om on \"user\".id = om.user_id
WHERE
        email = $1",
            ))
        }
        pub struct GetUserHashByEmailStmt(cornucopia_async::private::Stmt);
        impl GetUserHashByEmailStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                email: &'a T1,
            ) -> UserWithHashQuery<'a, C, UserWithHash, 1> {
                UserWithHashQuery {
                    client,
                    params: [email],
                    stmt: &mut self.0,
                    extractor: |row| UserWithHashBorrowed {
                        id: row.get(0),
                        email: row.get(1),
                        password_hash: row.get(2),
                        role: row.get(3),
                    },
                    mapper: |it| <UserWithHash>::from(it),
                }
            }
        }
        pub fn can_access_tenant() -> CanAccessTenantStmt {
            CanAccessTenantStmt(cornucopia_async::private::Stmt::new(
                "SELECT EXISTS (SELECT 1
               FROM organization_member om
                        JOIN tenant t ON om.organization_id = t.organization_id
               WHERE om.user_id = $1
                 AND t.id = $2) AS user_has_access",
            ))
        }
        pub struct CanAccessTenantStmt(cornucopia_async::private::Stmt);
        impl CanAccessTenantStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                user_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> BoolQuery<'a, C, bool, 2> {
                BoolQuery {
                    client,
                    params: [user_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<'a, CanAccessTenantParams, BoolQuery<'a, C, bool, 2>, C>
            for CanAccessTenantStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CanAccessTenantParams,
            ) -> BoolQuery<'a, C, bool, 2> {
                self.bind(client, &params.user_id, &params.tenant_id)
            }
        }
        pub fn get_user_role() -> GetUserRoleStmt {
            GetUserRoleStmt(cornucopia_async::private::Stmt::new(
                "SELECT role
FROM organization_member
WHERE user_id = $1
  AND organization_id = $2",
            ))
        }
        pub struct GetUserRoleStmt(cornucopia_async::private::Stmt);
        impl GetUserRoleStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                user_id: &'a uuid::Uuid,
                organization_id: &'a uuid::Uuid,
            ) -> SuperSuperTypesPublicOrganizationUserRoleQuery<
                'a,
                C,
                super::super::types::public::OrganizationUserRole,
                2,
            > {
                SuperSuperTypesPublicOrganizationUserRoleQuery {
                    client,
                    params: [user_id, organization_id],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetUserRoleParams,
                SuperSuperTypesPublicOrganizationUserRoleQuery<
                    'a,
                    C,
                    super::super::types::public::OrganizationUserRole,
                    2,
                >,
                C,
            > for GetUserRoleStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetUserRoleParams,
            ) -> SuperSuperTypesPublicOrganizationUserRoleQuery<
                'a,
                C,
                super::super::types::public::OrganizationUserRole,
                2,
            > {
                self.bind(client, &params.user_id, &params.organization_id)
            }
        }
        pub fn get_user_role_by_tenant() -> GetUserRoleByTenantStmt {
            GetUserRoleByTenantStmt(cornucopia_async::private::Stmt::new(
                "SELECT role
FROM organization_member om
         JOIN tenant t ON om.organization_id = t.organization_id
WHERE user_id = $1
  AND t.id = $2",
            ))
        }
        pub struct GetUserRoleByTenantStmt(cornucopia_async::private::Stmt);
        impl GetUserRoleByTenantStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                user_id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> SuperSuperTypesPublicOrganizationUserRoleQuery<
                'a,
                C,
                super::super::types::public::OrganizationUserRole,
                2,
            > {
                SuperSuperTypesPublicOrganizationUserRoleQuery {
                    client,
                    params: [user_id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetUserRoleByTenantParams,
                SuperSuperTypesPublicOrganizationUserRoleQuery<
                    'a,
                    C,
                    super::super::types::public::OrganizationUserRole,
                    2,
                >,
                C,
            > for GetUserRoleByTenantStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetUserRoleByTenantParams,
            ) -> SuperSuperTypesPublicOrganizationUserRoleQuery<
                'a,
                C,
                super::super::types::public::OrganizationUserRole,
                2,
            > {
                self.bind(client, &params.user_id, &params.tenant_id)
            }
        }
        pub fn get_user_role_by_tenant_slug() -> GetUserRoleByTenantSlugStmt {
            GetUserRoleByTenantSlugStmt(cornucopia_async::private::Stmt::new(
                "SELECT role, t.id as tenant_id
FROM organization_member om
         JOIN tenant t ON om.organization_id = t.organization_id
WHERE user_id = $1
  AND t.slug = $2",
            ))
        }
        pub struct GetUserRoleByTenantSlugStmt(cornucopia_async::private::Stmt);
        impl GetUserRoleByTenantSlugStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                user_id: &'a uuid::Uuid,
                tenant_slug: &'a T1,
            ) -> GetUserRoleByTenantSlugQuery<'a, C, GetUserRoleByTenantSlug, 2> {
                GetUserRoleByTenantSlugQuery {
                    client,
                    params: [user_id, tenant_slug],
                    stmt: &mut self.0,
                    extractor: |row| GetUserRoleByTenantSlug {
                        role: row.get(0),
                        tenant_id: row.get(1),
                    },
                    mapper: |it| <GetUserRoleByTenantSlug>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                GetUserRoleByTenantSlugParams<T1>,
                GetUserRoleByTenantSlugQuery<'a, C, GetUserRoleByTenantSlug, 2>,
                C,
            > for GetUserRoleByTenantSlugStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetUserRoleByTenantSlugParams<T1>,
            ) -> GetUserRoleByTenantSlugQuery<'a, C, GetUserRoleByTenantSlug, 2> {
                self.bind(client, &params.user_id, &params.tenant_slug)
            }
        }
        pub fn get_user_role_oss() -> GetUserRoleOssStmt {
            GetUserRoleOssStmt(cornucopia_async::private::Stmt::new(
                "SELECT role
FROM organization_member
WHERE user_id = $1
LIMIT 1",
            ))
        }
        pub struct GetUserRoleOssStmt(cornucopia_async::private::Stmt);
        impl GetUserRoleOssStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                user_id: &'a uuid::Uuid,
            ) -> SuperSuperTypesPublicOrganizationUserRoleQuery<
                'a,
                C,
                super::super::types::public::OrganizationUserRole,
                1,
            > {
                SuperSuperTypesPublicOrganizationUserRoleQuery {
                    client,
                    params: [user_id],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
        pub fn list_users() -> ListUsersStmt {
            ListUsersStmt(cornucopia_async::private::Stmt::new(
                "SELECT
    id,
    email,
    om.role
FROM
    \"user\"
        JOIN organization_member om on \"user\".id = om.user_id",
            ))
        }
        pub struct ListUsersStmt(cornucopia_async::private::Stmt);
        impl ListUsersStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
            ) -> ListUsersQuery<'a, C, ListUsers, 0> {
                ListUsersQuery {
                    client,
                    params: [],
                    stmt: &mut self.0,
                    extractor: |row| ListUsersBorrowed {
                        id: row.get(0),
                        email: row.get(1),
                        role: row.get(2),
                    },
                    mapper: |it| <ListUsers>::from(it),
                }
            }
        }
        pub fn exist_users() -> ExistUsersStmt {
            ExistUsersStmt(cornucopia_async::private::Stmt::new(
                "SELECT EXISTS (SELECT 1 FROM \"user\") AS user_exists",
            ))
        }
        pub struct ExistUsersStmt(cornucopia_async::private::Stmt);
        impl ExistUsersStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
            ) -> BoolQuery<'a, C, bool, 0> {
                BoolQuery {
                    client,
                    params: [],
                    stmt: &mut self.0,
                    extractor: |row| row.get(0),
                    mapper: |it| it,
                }
            }
        }
    }
    pub mod webhook_in_events {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateWebhookInEventParams<T1: cornucopia_async::StringSql> {
            pub id: uuid::Uuid,
            pub received_at: time::OffsetDateTime,
            pub key: T1,
            pub provider_config_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct WebhookInEvent {
            pub id: uuid::Uuid,
            pub received_at: time::OffsetDateTime,
            pub action: Option<String>,
            pub key: String,
            pub processed: bool,
            pub attempts: i32,
            pub error: Option<String>,
            pub provider_config_id: uuid::Uuid,
        }
        pub struct WebhookInEventBorrowed<'a> {
            pub id: uuid::Uuid,
            pub received_at: time::OffsetDateTime,
            pub action: Option<&'a str>,
            pub key: &'a str,
            pub processed: bool,
            pub attempts: i32,
            pub error: Option<&'a str>,
            pub provider_config_id: uuid::Uuid,
        }
        impl<'a> From<WebhookInEventBorrowed<'a>> for WebhookInEvent {
            fn from(
                WebhookInEventBorrowed {
                    id,
                    received_at,
                    action,
                    key,
                    processed,
                    attempts,
                    error,
                    provider_config_id,
                }: WebhookInEventBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    received_at,
                    action: action.map(|v| v.into()),
                    key: key.into(),
                    processed,
                    attempts,
                    error: error.map(|v| v.into()),
                    provider_config_id,
                }
            }
        }
        pub struct WebhookInEventQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> WebhookInEventBorrowed,
            mapper: fn(WebhookInEventBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> WebhookInEventQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(WebhookInEventBorrowed) -> R,
            ) -> WebhookInEventQuery<'a, C, R, N> {
                WebhookInEventQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct FindWebhookInEventsByTenantId {
            pub id: uuid::Uuid,
            pub received_at: time::OffsetDateTime,
            pub action: String,
            pub key: String,
            pub processed: bool,
            pub attempts: i32,
            pub error: String,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
        }
        pub struct FindWebhookInEventsByTenantIdBorrowed<'a> {
            pub id: uuid::Uuid,
            pub received_at: time::OffsetDateTime,
            pub action: &'a str,
            pub key: &'a str,
            pub processed: bool,
            pub attempts: i32,
            pub error: &'a str,
            pub invoicing_provider: super::super::types::public::InvoicingProviderEnum,
        }
        impl<'a> From<FindWebhookInEventsByTenantIdBorrowed<'a>> for FindWebhookInEventsByTenantId {
            fn from(
                FindWebhookInEventsByTenantIdBorrowed {
                    id,
                    received_at,
                    action,
                    key,
                    processed,
                    attempts,
                    error,
                    invoicing_provider,
                }: FindWebhookInEventsByTenantIdBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    received_at,
                    action: action.into(),
                    key: key.into(),
                    processed,
                    attempts,
                    error: error.into(),
                    invoicing_provider,
                }
            }
        }
        pub struct FindWebhookInEventsByTenantIdQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> FindWebhookInEventsByTenantIdBorrowed,
            mapper: fn(FindWebhookInEventsByTenantIdBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> FindWebhookInEventsByTenantIdQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(FindWebhookInEventsByTenantIdBorrowed) -> R,
            ) -> FindWebhookInEventsByTenantIdQuery<'a, C, R, N> {
                FindWebhookInEventsByTenantIdQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn get_webhook_in_event_by_id() -> GetWebhookInEventByIdStmt {
            GetWebhookInEventByIdStmt(cornucopia_async :: private :: Stmt :: new("SELECT id, received_at, action, key, processed, attempts, error, provider_config_id FROM webhook_in_event WHERE id = $1"))
        }
        pub struct GetWebhookInEventByIdStmt(cornucopia_async::private::Stmt);
        impl GetWebhookInEventByIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
            ) -> WebhookInEventQuery<'a, C, WebhookInEvent, 1> {
                WebhookInEventQuery {
                    client,
                    params: [id],
                    stmt: &mut self.0,
                    extractor: |row| WebhookInEventBorrowed {
                        id: row.get(0),
                        received_at: row.get(1),
                        action: row.get(2),
                        key: row.get(3),
                        processed: row.get(4),
                        attempts: row.get(5),
                        error: row.get(6),
                        provider_config_id: row.get(7),
                    },
                    mapper: |it| <WebhookInEvent>::from(it),
                }
            }
        }
        pub fn create_webhook_in_event() -> CreateWebhookInEventStmt {
            CreateWebhookInEventStmt(cornucopia_async::private::Stmt::new(
                "INSERT INTO webhook_in_event (id, received_at, key, provider_config_id)
VALUES ($1, $2, $3, $4)
RETURNING id, received_at, action, key, processed, attempts, error, provider_config_id",
            ))
        }
        pub struct CreateWebhookInEventStmt(cornucopia_async::private::Stmt);
        impl CreateWebhookInEventStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                received_at: &'a time::OffsetDateTime,
                key: &'a T1,
                provider_config_id: &'a uuid::Uuid,
            ) -> WebhookInEventQuery<'a, C, WebhookInEvent, 4> {
                WebhookInEventQuery {
                    client,
                    params: [id, received_at, key, provider_config_id],
                    stmt: &mut self.0,
                    extractor: |row| WebhookInEventBorrowed {
                        id: row.get(0),
                        received_at: row.get(1),
                        action: row.get(2),
                        key: row.get(3),
                        processed: row.get(4),
                        attempts: row.get(5),
                        error: row.get(6),
                        provider_config_id: row.get(7),
                    },
                    mapper: |it| <WebhookInEvent>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                CreateWebhookInEventParams<T1>,
                WebhookInEventQuery<'a, C, WebhookInEvent, 4>,
                C,
            > for CreateWebhookInEventStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateWebhookInEventParams<T1>,
            ) -> WebhookInEventQuery<'a, C, WebhookInEvent, 4> {
                self.bind(
                    client,
                    &params.id,
                    &params.received_at,
                    &params.key,
                    &params.provider_config_id,
                )
            }
        }
        pub fn find_webhook_in_events_by_tenant_id() -> FindWebhookInEventsByTenantIdStmt {
            FindWebhookInEventsByTenantIdStmt(cornucopia_async :: private :: Stmt :: new("SELECT webhook_in_event.id, webhook_in_event.received_at, webhook_in_event.action, webhook_in_event.key, webhook_in_event.processed, webhook_in_event.attempts, webhook_in_event.error, provider_config.invoicing_provider
FROM webhook_in_event
JOIN provider_config ON provider_config.id = webhook_in_event.provider_config_id
WHERE provider_config.tenant_id = $1"))
        }
        pub struct FindWebhookInEventsByTenantIdStmt(cornucopia_async::private::Stmt);
        impl FindWebhookInEventsByTenantIdStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> FindWebhookInEventsByTenantIdQuery<'a, C, FindWebhookInEventsByTenantId, 1>
            {
                FindWebhookInEventsByTenantIdQuery {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| FindWebhookInEventsByTenantIdBorrowed {
                        id: row.get(0),
                        received_at: row.get(1),
                        action: row.get(2),
                        key: row.get(3),
                        processed: row.get(4),
                        attempts: row.get(5),
                        error: row.get(6),
                        invoicing_provider: row.get(7),
                    },
                    mapper: |it| <FindWebhookInEventsByTenantId>::from(it),
                }
            }
        }
    }
    pub mod webhook_out_endpoints {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateEndpointParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
            T4: cornucopia_async::ArraySql<Item = super::super::types::public::WebhookOutEventTypeEnum>,
        > {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub url: T1,
            pub description: Option<T2>,
            pub secret: T3,
            pub events_to_listen: T4,
            pub enabled: bool,
        }
        #[derive(Clone, Copy, Debug)]
        pub struct GetByIdAndTenantParams {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct WebhookOutEndpoint {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub url: String,
            pub description: Option<String>,
            pub secret: String,
            pub created_at: time::PrimitiveDateTime,
            pub events_to_listen: Vec<super::super::types::public::WebhookOutEventTypeEnum>,
            pub enabled: bool,
        }
        pub struct WebhookOutEndpointBorrowed<'a> {
            pub id: uuid::Uuid,
            pub tenant_id: uuid::Uuid,
            pub url: &'a str,
            pub description: Option<&'a str>,
            pub secret: &'a str,
            pub created_at: time::PrimitiveDateTime,
            pub events_to_listen: cornucopia_async::ArrayIterator<
                'a,
                super::super::types::public::WebhookOutEventTypeEnum,
            >,
            pub enabled: bool,
        }
        impl<'a> From<WebhookOutEndpointBorrowed<'a>> for WebhookOutEndpoint {
            fn from(
                WebhookOutEndpointBorrowed {
                    id,
                    tenant_id,
                    url,
                    description,
                    secret,
                    created_at,
                    events_to_listen,
                    enabled,
                }: WebhookOutEndpointBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    tenant_id,
                    url: url.into(),
                    description: description.map(|v| v.into()),
                    secret: secret.into(),
                    created_at,
                    events_to_listen: events_to_listen.map(|v| v).collect(),
                    enabled,
                }
            }
        }
        pub struct WebhookOutEndpointQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> WebhookOutEndpointBorrowed,
            mapper: fn(WebhookOutEndpointBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> WebhookOutEndpointQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(WebhookOutEndpointBorrowed) -> R,
            ) -> WebhookOutEndpointQuery<'a, C, R, N> {
                WebhookOutEndpointQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn create_endpoint() -> CreateEndpointStmt {
            CreateEndpointStmt(cornucopia_async :: private :: Stmt :: new("insert into webhook_out_endpoint (id, tenant_id, url, description, secret, events_to_listen, enabled)
values ($1, $2, $3, $4, $5, $6, $7)
returning id, tenant_id, url, description, secret, created_at, events_to_listen, enabled"))
        }
        pub struct CreateEndpointStmt(cornucopia_async::private::Stmt);
        impl CreateEndpointStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::ArraySql<
                    Item = super::super::types::public::WebhookOutEventTypeEnum,
                >,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
                url: &'a T1,
                description: &'a Option<T2>,
                secret: &'a T3,
                events_to_listen: &'a T4,
                enabled: &'a bool,
            ) -> WebhookOutEndpointQuery<'a, C, WebhookOutEndpoint, 7> {
                WebhookOutEndpointQuery {
                    client,
                    params: [
                        id,
                        tenant_id,
                        url,
                        description,
                        secret,
                        events_to_listen,
                        enabled,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| WebhookOutEndpointBorrowed {
                        id: row.get(0),
                        tenant_id: row.get(1),
                        url: row.get(2),
                        description: row.get(3),
                        secret: row.get(4),
                        created_at: row.get(5),
                        events_to_listen: row.get(6),
                        enabled: row.get(7),
                    },
                    mapper: |it| <WebhookOutEndpoint>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
                T4: cornucopia_async::ArraySql<
                    Item = super::super::types::public::WebhookOutEventTypeEnum,
                >,
            >
            cornucopia_async::Params<
                'a,
                CreateEndpointParams<T1, T2, T3, T4>,
                WebhookOutEndpointQuery<'a, C, WebhookOutEndpoint, 7>,
                C,
            > for CreateEndpointStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateEndpointParams<T1, T2, T3, T4>,
            ) -> WebhookOutEndpointQuery<'a, C, WebhookOutEndpoint, 7> {
                self.bind(
                    client,
                    &params.id,
                    &params.tenant_id,
                    &params.url,
                    &params.description,
                    &params.secret,
                    &params.events_to_listen,
                    &params.enabled,
                )
            }
        }
        pub fn list_endpoints() -> ListEndpointsStmt {
            ListEndpointsStmt(cornucopia_async :: private :: Stmt :: new("select id, tenant_id, url, description, secret, created_at, events_to_listen, enabled
from webhook_out_endpoint
where tenant_id = $1"))
        }
        pub struct ListEndpointsStmt(cornucopia_async::private::Stmt);
        impl ListEndpointsStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                tenant_id: &'a uuid::Uuid,
            ) -> WebhookOutEndpointQuery<'a, C, WebhookOutEndpoint, 1> {
                WebhookOutEndpointQuery {
                    client,
                    params: [tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| WebhookOutEndpointBorrowed {
                        id: row.get(0),
                        tenant_id: row.get(1),
                        url: row.get(2),
                        description: row.get(3),
                        secret: row.get(4),
                        created_at: row.get(5),
                        events_to_listen: row.get(6),
                        enabled: row.get(7),
                    },
                    mapper: |it| <WebhookOutEndpoint>::from(it),
                }
            }
        }
        pub fn get_by_id_and_tenant() -> GetByIdAndTenantStmt {
            GetByIdAndTenantStmt(cornucopia_async :: private :: Stmt :: new("select id, tenant_id, url, description, secret, created_at, events_to_listen, enabled
from webhook_out_endpoint
where id = $1 and tenant_id = $2
limit 1"))
        }
        pub struct GetByIdAndTenantStmt(cornucopia_async::private::Stmt);
        impl GetByIdAndTenantStmt {
            pub fn bind<'a, C: GenericClient>(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                tenant_id: &'a uuid::Uuid,
            ) -> WebhookOutEndpointQuery<'a, C, WebhookOutEndpoint, 2> {
                WebhookOutEndpointQuery {
                    client,
                    params: [id, tenant_id],
                    stmt: &mut self.0,
                    extractor: |row| WebhookOutEndpointBorrowed {
                        id: row.get(0),
                        tenant_id: row.get(1),
                        url: row.get(2),
                        description: row.get(3),
                        secret: row.get(4),
                        created_at: row.get(5),
                        events_to_listen: row.get(6),
                        enabled: row.get(7),
                    },
                    mapper: |it| <WebhookOutEndpoint>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient>
            cornucopia_async::Params<
                'a,
                GetByIdAndTenantParams,
                WebhookOutEndpointQuery<'a, C, WebhookOutEndpoint, 2>,
                C,
            > for GetByIdAndTenantStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a GetByIdAndTenantParams,
            ) -> WebhookOutEndpointQuery<'a, C, WebhookOutEndpoint, 2> {
                self.bind(client, &params.id, &params.tenant_id)
            }
        }
    }
    pub mod webhook_out_events {
        use cornucopia_async::GenericClient;
        use futures;
        use futures::{StreamExt, TryStreamExt};
        #[derive(Debug)]
        pub struct CreateEventParams<
            T1: cornucopia_async::StringSql,
            T2: cornucopia_async::StringSql,
            T3: cornucopia_async::StringSql,
        > {
            pub id: uuid::Uuid,
            pub endpoint_id: uuid::Uuid,
            pub event_type: super::super::types::public::WebhookOutEventTypeEnum,
            pub request_body: T1,
            pub response_body: Option<T2>,
            pub http_status_code: Option<i16>,
            pub error_message: Option<T3>,
        }
        #[derive(Debug)]
        pub struct ListEventsParams<T1: cornucopia_async::StringSql> {
            pub endpoint_id: uuid::Uuid,
            pub order_by: T1,
            pub limit: i64,
            pub offset: i64,
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct WebhookOutEvent {
            pub id: uuid::Uuid,
            pub endpoint_id: uuid::Uuid,
            pub event_type: super::super::types::public::WebhookOutEventTypeEnum,
            pub request_body: String,
            pub response_body: Option<String>,
            pub http_status_code: Option<i16>,
            pub created_at: time::PrimitiveDateTime,
            pub error_message: Option<String>,
        }
        pub struct WebhookOutEventBorrowed<'a> {
            pub id: uuid::Uuid,
            pub endpoint_id: uuid::Uuid,
            pub event_type: super::super::types::public::WebhookOutEventTypeEnum,
            pub request_body: &'a str,
            pub response_body: Option<&'a str>,
            pub http_status_code: Option<i16>,
            pub created_at: time::PrimitiveDateTime,
            pub error_message: Option<&'a str>,
        }
        impl<'a> From<WebhookOutEventBorrowed<'a>> for WebhookOutEvent {
            fn from(
                WebhookOutEventBorrowed {
                    id,
                    endpoint_id,
                    event_type,
                    request_body,
                    response_body,
                    http_status_code,
                    created_at,
                    error_message,
                }: WebhookOutEventBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    endpoint_id,
                    event_type,
                    request_body: request_body.into(),
                    response_body: response_body.map(|v| v.into()),
                    http_status_code,
                    created_at,
                    error_message: error_message.map(|v| v.into()),
                }
            }
        }
        pub struct WebhookOutEventQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> WebhookOutEventBorrowed,
            mapper: fn(WebhookOutEventBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> WebhookOutEventQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(WebhookOutEventBorrowed) -> R,
            ) -> WebhookOutEventQuery<'a, C, R, N> {
                WebhookOutEventQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        #[derive(Debug, Clone, PartialEq)]
        pub struct ListWebhookOutEvent {
            pub id: uuid::Uuid,
            pub endpoint_id: uuid::Uuid,
            pub event_type: super::super::types::public::WebhookOutEventTypeEnum,
            pub request_body: String,
            pub response_body: Option<String>,
            pub http_status_code: Option<i16>,
            pub created_at: time::PrimitiveDateTime,
            pub error_message: Option<String>,
            pub total_count: i64,
        }
        pub struct ListWebhookOutEventBorrowed<'a> {
            pub id: uuid::Uuid,
            pub endpoint_id: uuid::Uuid,
            pub event_type: super::super::types::public::WebhookOutEventTypeEnum,
            pub request_body: &'a str,
            pub response_body: Option<&'a str>,
            pub http_status_code: Option<i16>,
            pub created_at: time::PrimitiveDateTime,
            pub error_message: Option<&'a str>,
            pub total_count: i64,
        }
        impl<'a> From<ListWebhookOutEventBorrowed<'a>> for ListWebhookOutEvent {
            fn from(
                ListWebhookOutEventBorrowed {
                    id,
                    endpoint_id,
                    event_type,
                    request_body,
                    response_body,
                    http_status_code,
                    created_at,
                    error_message,
                    total_count,
                }: ListWebhookOutEventBorrowed<'a>,
            ) -> Self {
                Self {
                    id,
                    endpoint_id,
                    event_type,
                    request_body: request_body.into(),
                    response_body: response_body.map(|v| v.into()),
                    http_status_code,
                    created_at,
                    error_message: error_message.map(|v| v.into()),
                    total_count,
                }
            }
        }
        pub struct ListWebhookOutEventQuery<'a, C: GenericClient, T, const N: usize> {
            client: &'a C,
            params: [&'a (dyn postgres_types::ToSql + Sync); N],
            stmt: &'a mut cornucopia_async::private::Stmt,
            extractor: fn(&tokio_postgres::Row) -> ListWebhookOutEventBorrowed,
            mapper: fn(ListWebhookOutEventBorrowed) -> T,
        }
        impl<'a, C, T: 'a, const N: usize> ListWebhookOutEventQuery<'a, C, T, N>
        where
            C: GenericClient,
        {
            pub fn map<R>(
                self,
                mapper: fn(ListWebhookOutEventBorrowed) -> R,
            ) -> ListWebhookOutEventQuery<'a, C, R, N> {
                ListWebhookOutEventQuery {
                    client: self.client,
                    params: self.params,
                    stmt: self.stmt,
                    extractor: self.extractor,
                    mapper,
                }
            }
            pub async fn one(self) -> Result<T, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                let row = self.client.query_one(stmt, &self.params).await?;
                Ok((self.mapper)((self.extractor)(&row)))
            }
            pub async fn all(self) -> Result<Vec<T>, tokio_postgres::Error> {
                self.iter().await?.try_collect().await
            }
            pub async fn opt(self) -> Result<Option<T>, tokio_postgres::Error> {
                let stmt = self.stmt.prepare(self.client).await?;
                Ok(self
                    .client
                    .query_opt(stmt, &self.params)
                    .await?
                    .map(|row| (self.mapper)((self.extractor)(&row))))
            }
            pub async fn iter(
                self,
            ) -> Result<
                impl futures::Stream<Item = Result<T, tokio_postgres::Error>> + 'a,
                tokio_postgres::Error,
            > {
                let stmt = self.stmt.prepare(self.client).await?;
                let it = self
                    .client
                    .query_raw(stmt, cornucopia_async::private::slice_iter(&self.params))
                    .await?
                    .map(move |res| res.map(|row| (self.mapper)((self.extractor)(&row))))
                    .into_stream();
                Ok(it)
            }
        }
        pub fn create_event() -> CreateEventStmt {
            CreateEventStmt(cornucopia_async :: private :: Stmt :: new("insert into webhook_out_event(id, endpoint_id, event_type, request_body, response_body, http_status_code, error_message)
values ($1, $2, $3, $4, $5, $6, $7)
returning id, endpoint_id, event_type, request_body, response_body, http_status_code, created_at, error_message"))
        }
        pub struct CreateEventStmt(cornucopia_async::private::Stmt);
        impl CreateEventStmt {
            pub fn bind<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >(
                &'a mut self,
                client: &'a C,
                id: &'a uuid::Uuid,
                endpoint_id: &'a uuid::Uuid,
                event_type: &'a super::super::types::public::WebhookOutEventTypeEnum,
                request_body: &'a T1,
                response_body: &'a Option<T2>,
                http_status_code: &'a Option<i16>,
                error_message: &'a Option<T3>,
            ) -> WebhookOutEventQuery<'a, C, WebhookOutEvent, 7> {
                WebhookOutEventQuery {
                    client,
                    params: [
                        id,
                        endpoint_id,
                        event_type,
                        request_body,
                        response_body,
                        http_status_code,
                        error_message,
                    ],
                    stmt: &mut self.0,
                    extractor: |row| WebhookOutEventBorrowed {
                        id: row.get(0),
                        endpoint_id: row.get(1),
                        event_type: row.get(2),
                        request_body: row.get(3),
                        response_body: row.get(4),
                        http_status_code: row.get(5),
                        created_at: row.get(6),
                        error_message: row.get(7),
                    },
                    mapper: |it| <WebhookOutEvent>::from(it),
                }
            }
        }
        impl<
                'a,
                C: GenericClient,
                T1: cornucopia_async::StringSql,
                T2: cornucopia_async::StringSql,
                T3: cornucopia_async::StringSql,
            >
            cornucopia_async::Params<
                'a,
                CreateEventParams<T1, T2, T3>,
                WebhookOutEventQuery<'a, C, WebhookOutEvent, 7>,
                C,
            > for CreateEventStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a CreateEventParams<T1, T2, T3>,
            ) -> WebhookOutEventQuery<'a, C, WebhookOutEvent, 7> {
                self.bind(
                    client,
                    &params.id,
                    &params.endpoint_id,
                    &params.event_type,
                    &params.request_body,
                    &params.response_body,
                    &params.http_status_code,
                    &params.error_message,
                )
            }
        }
        pub fn list_events() -> ListEventsStmt {
            ListEventsStmt(cornucopia_async::private::Stmt::new(
                "SELECT id,
       endpoint_id,
       event_type,
       request_body,
       response_body,
       http_status_code,
       created_at,
       error_message,
       COUNT(*) OVER () AS total_count
FROM webhook_out_event
WHERE endpoint_id = $1
ORDER BY CASE
           WHEN $2 = 'DATE_DESC' THEN created_at
           END DESC,
         CASE
           WHEN $2 = 'DATE_ASC' THEN created_at
           END ASC,
         CASE
           WHEN $2 = 'ID_DESC' THEN id
           END DESC,
         CASE
           WHEN $2 = 'ID_ASC' THEN id
           END ASC
LIMIT $3 OFFSET $4",
            ))
        }
        pub struct ListEventsStmt(cornucopia_async::private::Stmt);
        impl ListEventsStmt {
            pub fn bind<'a, C: GenericClient, T1: cornucopia_async::StringSql>(
                &'a mut self,
                client: &'a C,
                endpoint_id: &'a uuid::Uuid,
                order_by: &'a T1,
                limit: &'a i64,
                offset: &'a i64,
            ) -> ListWebhookOutEventQuery<'a, C, ListWebhookOutEvent, 4> {
                ListWebhookOutEventQuery {
                    client,
                    params: [endpoint_id, order_by, limit, offset],
                    stmt: &mut self.0,
                    extractor: |row| ListWebhookOutEventBorrowed {
                        id: row.get(0),
                        endpoint_id: row.get(1),
                        event_type: row.get(2),
                        request_body: row.get(3),
                        response_body: row.get(4),
                        http_status_code: row.get(5),
                        created_at: row.get(6),
                        error_message: row.get(7),
                        total_count: row.get(8),
                    },
                    mapper: |it| <ListWebhookOutEvent>::from(it),
                }
            }
        }
        impl<'a, C: GenericClient, T1: cornucopia_async::StringSql>
            cornucopia_async::Params<
                'a,
                ListEventsParams<T1>,
                ListWebhookOutEventQuery<'a, C, ListWebhookOutEvent, 4>,
                C,
            > for ListEventsStmt
        {
            fn params(
                &'a mut self,
                client: &'a C,
                params: &'a ListEventsParams<T1>,
            ) -> ListWebhookOutEventQuery<'a, C, ListWebhookOutEvent, 4> {
                self.bind(
                    client,
                    &params.endpoint_id,
                    &params.order_by,
                    &params.limit,
                    &params.offset,
                )
            }
        }
    }
}
