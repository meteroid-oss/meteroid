//! Fluent assertion helpers for test validation.

use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum};
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_store::domain::Invoice;
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum};

// =============================================================================
// SUBSCRIPTION ASSERTIONS
// =============================================================================

/// Fluent assertion builder for subscription state.
pub struct SubscriptionAssert<'a> {
    sub: &'a SubscriptionRow,
    context: String,
}

impl<'a> SubscriptionAssert<'a> {
    /// Create a new subscription assertion.
    pub fn new(sub: &'a SubscriptionRow) -> Self {
        Self {
            sub,
            context: String::new(),
        }
    }

    /// Add context to error messages.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = context.into();
        self
    }

    fn format_msg(&self, base: &str) -> String {
        if self.context.is_empty() {
            base.to_string()
        } else {
            format!("[{}] {}", self.context, base)
        }
    }

    /// Assert the subscription has a specific status.
    pub fn has_status(self, expected: SubscriptionStatusEnum) -> Self {
        assert_eq!(
            self.sub.status,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected status {:?}, got {:?}",
                expected, self.sub.status
            ))
        );
        self
    }

    /// Assert the subscription has a specific next_cycle_action.
    pub fn has_next_action(self, expected: Option<CycleActionEnum>) -> Self {
        assert_eq!(
            self.sub.next_cycle_action,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected next_cycle_action {:?}, got {:?}",
                expected, self.sub.next_cycle_action
            ))
        );
        self
    }

    /// Assert pending_checkout flag value.
    pub fn has_pending_checkout(self, expected: bool) -> Self {
        assert_eq!(
            self.sub.pending_checkout,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected pending_checkout={}, got {}",
                expected, self.sub.pending_checkout
            ))
        );
        self
    }

    /// Assert whether payment method is present.
    pub fn has_payment_method(self, expected: bool) -> Self {
        let has_pm = self.sub.payment_method.is_some();
        assert_eq!(
            has_pm,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected has_payment_method={}, got {}",
                expected, has_pm
            ))
        );
        self
    }

    /// Assert trial duration days.
    pub fn has_trial_duration(self, expected: Option<i32>) -> Self {
        assert_eq!(
            self.sub.trial_duration,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected trial_duration={:?}, got {:?}",
                expected, self.sub.trial_duration
            ))
        );
        self
    }

    /// Assert cycle index.
    pub fn has_cycle_index(self, expected: i32) -> Self {
        assert_eq!(
            self.sub.cycle_index,
            Some(expected),
            "{}",
            self.format_msg(&format!(
                "Expected cycle_index={}, got {:?}",
                expected, self.sub.cycle_index
            ))
        );
        self
    }

    /// Shorthand: Assert subscription is Active with RenewSubscription action.
    #[allow(clippy::wrong_self_convention)]
    pub fn is_active(self) -> Self {
        self.has_status(SubscriptionStatusEnum::Active)
            .has_next_action(Some(CycleActionEnum::RenewSubscription))
    }

    /// Shorthand: Assert subscription is TrialActive.
    #[allow(clippy::wrong_self_convention)]
    pub fn is_trial_active(self) -> Self {
        self.has_status(SubscriptionStatusEnum::TrialActive)
    }

    /// Shorthand: Assert subscription is PendingActivation.
    #[allow(clippy::wrong_self_convention)]
    pub fn is_pending_activation(self) -> Self {
        self.has_status(SubscriptionStatusEnum::PendingActivation)
            .has_next_action(None)
    }

    /// Shorthand: Assert subscription is TrialExpired.
    #[allow(clippy::wrong_self_convention)]
    pub fn is_trial_expired(self) -> Self {
        self.has_status(SubscriptionStatusEnum::TrialExpired)
            .has_next_action(None)
    }
}

/// Extension trait for SubscriptionRow to enable fluent assertions.
pub trait SubscriptionAssertExt {
    fn assert(&self) -> SubscriptionAssert<'_>;
}

impl SubscriptionAssertExt for SubscriptionRow {
    fn assert(&self) -> SubscriptionAssert<'_> {
        SubscriptionAssert::new(self)
    }
}

// =============================================================================
// INVOICE ASSERTIONS
// =============================================================================

/// Fluent assertion builder for invoice state.
pub struct InvoiceAssert<'a> {
    invoice: &'a Invoice,
    context: String,
}

impl<'a> InvoiceAssert<'a> {
    /// Create a new invoice assertion.
    pub fn new(invoice: &'a Invoice) -> Self {
        Self {
            invoice,
            context: String::new(),
        }
    }

    /// Add context to error messages.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = context.into();
        self
    }

    fn format_msg(&self, base: &str) -> String {
        if self.context.is_empty() {
            base.to_string()
        } else {
            format!("[{}] {}", self.context, base)
        }
    }

    /// Assert the invoice has a specific status.
    pub fn has_status(self, expected: InvoiceStatusEnum) -> Self {
        assert_eq!(
            self.invoice.status,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected invoice status {:?}, got {:?}",
                expected, self.invoice.status
            ))
        );
        self
    }

    /// Assert the invoice has a specific payment status.
    pub fn has_payment_status(self, expected: InvoicePaymentStatus) -> Self {
        assert_eq!(
            self.invoice.payment_status,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected payment_status {:?}, got {:?}",
                expected, self.invoice.payment_status
            ))
        );
        self
    }

    /// Assert the invoice total.
    pub fn has_total(self, expected: i64) -> Self {
        assert_eq!(
            self.invoice.total,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected total={}, got {}",
                expected, self.invoice.total
            ))
        );
        self
    }

    /// Assert whether the invoice is prorated.
    pub fn check_prorated(self, expected: bool) -> Self {
        let is_prorated = self.invoice.line_items.iter().any(|li| li.is_prorated);
        assert_eq!(
            is_prorated,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected is_prorated={}, got {}",
                expected, is_prorated
            ))
        );
        self
    }

    /// Assert the invoice date.
    pub fn has_invoice_date(self, expected: chrono::NaiveDate) -> Self {
        assert_eq!(
            self.invoice.invoice_date,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected invoice_date={}, got {}",
                expected, self.invoice.invoice_date
            ))
        );
        self
    }

    /// Assert the billing period dates via line items.
    /// All line items should have matching period dates.
    pub fn has_period(
        self,
        expected_start: chrono::NaiveDate,
        expected_end: chrono::NaiveDate,
    ) -> Self {
        for (i, li) in self.invoice.line_items.iter().enumerate() {
            assert_eq!(
                li.start_date,
                expected_start,
                "{}",
                self.format_msg(&format!(
                    "line_item[{}]: Expected start_date={}, got {}",
                    i, expected_start, li.start_date
                ))
            );
            assert_eq!(
                li.end_date,
                expected_end,
                "{}",
                self.format_msg(&format!(
                    "line_item[{}]: Expected end_date={}, got {}",
                    i, expected_end, li.end_date
                ))
            );
        }
        self
    }

    /// Shorthand: Assert invoice is finalized and unpaid.
    #[allow(clippy::wrong_self_convention)]
    pub fn is_finalized_unpaid(self) -> Self {
        self.has_status(InvoiceStatusEnum::Finalized)
            .has_payment_status(InvoicePaymentStatus::Unpaid)
    }

    /// Shorthand: Assert invoice is finalized and paid.
    #[allow(dead_code, clippy::wrong_self_convention)]
    pub fn is_finalized_paid(self) -> Self {
        self.has_status(InvoiceStatusEnum::Finalized)
            .has_payment_status(InvoicePaymentStatus::Paid)
    }

    /// Assert the invoice subtotal (before discounts).
    #[allow(dead_code)]
    pub fn has_subtotal(self, expected: i64) -> Self {
        assert_eq!(
            self.invoice.subtotal,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected subtotal={}, got {}",
                expected, self.invoice.subtotal
            ))
        );
        self
    }

    /// Assert the invoice has applied coupons.
    #[allow(dead_code)]
    pub fn has_coupons_count(self, expected: usize) -> Self {
        let actual = self.invoice.coupons.len();
        assert_eq!(
            actual,
            expected,
            "{}",
            self.format_msg(&format!("Expected {} coupons, got {}", expected, actual))
        );
        self
    }

    /// Assert the invoice discount total.
    #[allow(dead_code)]
    pub fn has_discount(self, expected: i64) -> Self {
        assert_eq!(
            self.invoice.discount,
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected discount={}, got {}",
                expected, self.invoice.discount
            ))
        );
        self
    }
}

/// Extension trait for Invoice to enable fluent assertions.
#[allow(dead_code)]
pub trait InvoiceAssertExt {
    fn assert(&self) -> InvoiceAssert<'_>;
}

impl InvoiceAssertExt for Invoice {
    fn assert(&self) -> InvoiceAssert<'_> {
        InvoiceAssert::new(self)
    }
}

// =============================================================================
// INVOICE LIST ASSERTIONS
// =============================================================================

/// Fluent assertion builder for a list of invoices.
pub struct InvoicesAssert<'a> {
    invoices: &'a [Invoice],
    context: String,
}

impl<'a> InvoicesAssert<'a> {
    /// Create a new invoices assertion.
    pub fn new(invoices: &'a [Invoice]) -> Self {
        Self {
            invoices,
            context: String::new(),
        }
    }

    /// Add context to error messages.
    #[allow(dead_code)]
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = context.into();
        self
    }

    fn format_msg(&self, base: &str) -> String {
        if self.context.is_empty() {
            base.to_string()
        } else {
            format!("[{}] {}", self.context, base)
        }
    }

    /// Assert the number of invoices.
    pub fn has_count(self, expected: usize) -> Self {
        assert_eq!(
            self.invoices.len(),
            expected,
            "{}",
            self.format_msg(&format!(
                "Expected {} invoices, got {}",
                expected,
                self.invoices.len()
            ))
        );
        self
    }

    /// Assert there are no invoices.
    pub fn assert_empty(self) -> Self {
        assert!(
            self.invoices.is_empty(),
            "{}",
            self.format_msg(&format!(
                "Expected no invoices, got {}",
                self.invoices.len()
            ))
        );
        self
    }

    /// Assert a specific invoice by index and return assertion for chaining.
    pub fn invoice_at(&self, index: usize) -> InvoiceAssert<'_> {
        assert!(
            index < self.invoices.len(),
            "{}",
            self.format_msg(&format!(
                "Invoice index {} out of bounds (len={})",
                index,
                self.invoices.len()
            ))
        );
        InvoiceAssert::new(&self.invoices[index]).with_context(format!("invoice[{}]", index))
    }

    /// Assert the latest (last) invoice.
    #[allow(dead_code)]
    pub fn latest(&self) -> InvoiceAssert<'_> {
        assert!(
            !self.invoices.is_empty(),
            "{}",
            self.format_msg("Expected at least one invoice")
        );
        InvoiceAssert::new(self.invoices.last().unwrap()).with_context("latest invoice")
    }
}

/// Extension trait for Vec<Invoice> to enable fluent assertions.
pub trait InvoicesAssertExt {
    fn assert(&self) -> InvoicesAssert<'_>;
}

impl InvoicesAssertExt for Vec<Invoice> {
    fn assert(&self) -> InvoicesAssert<'_> {
        InvoicesAssert::new(self)
    }
}

impl InvoicesAssertExt for [Invoice] {
    fn assert(&self) -> InvoicesAssert<'_> {
        InvoicesAssert::new(self)
    }
}
