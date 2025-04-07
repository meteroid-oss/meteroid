use crate::errors::InvoicingResult;
use crate::model::*;

// Function to attach Factur-X XML to a PDF
#[allow(unused)]
pub fn generate_facturx_pdf(_pdf_data: &[u8], _invoice: &Invoice) -> InvoicingResult<Vec<u8>> {
    todo!()
}

#[allow(unused)]
fn generate_facturx_xml(_invoice: &Invoice) -> InvoicingResult<String> {
    todo!()
}
