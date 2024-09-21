use crate::StoreResult;
use std::sync::Arc;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait InvoiceRenderingService: Send + Sync {
    async fn preview_invoice_html(&self, invoice_id: Uuid, tenant_id: Uuid) -> StoreResult<String>;
    // async fn generate_pdfs(&self, invoice_ids: Vec<Uuid>) -> StoreResult<Vec<GenerateResult>>;
}

struct NoopInvoiceRenderingService;

// pub enum GenerateResult {
//     Success { invoice_id: Uuid, pdf_url: String },
//     Failure { invoice_id: Uuid, error: String },
// }

#[async_trait::async_trait]
impl InvoiceRenderingService for NoopInvoiceRenderingService {
    async fn preview_invoice_html(
        &self,
        _invoice_id: Uuid,
        _tenant_id: Uuid,
    ) -> StoreResult<String> {
        Ok("noop".to_string())
    }

    // async fn generate_pdfs(&self, _invoice_ids: Vec<Uuid>) -> StoreResult<Vec<GenerateResult>> {
    //     Ok(Vec::new())
    // }
}

pub fn noop_invoice_rendering_service() -> Arc<dyn InvoiceRenderingService> {
    Arc::new(NoopInvoiceRenderingService)
}
