//! The Factur-X XMP metadata, stamped onto a PDF that already carries the payload.
//!
//! Typst attaches the XML and wires it in the way PDF/A-3 requires — associated
//! file, embedded-files name tree, relationship. What it cannot know is that the
//! attachment *is* an invoice: that claim lives in the document's XMP, under the
//! Factur-X extension schema, and it is how a receiver discovers the invoice
//! **before parsing anything**.
//!
//! # The trap this module exists to avoid
//!
//! XMP (ISO 16684-1) allows each property at most once per packet, and
//! `pdfaExtension:schemas` *is* a property. Typst already writes one — it
//! declares the XMP Media Management and Adobe PDF schemas — so appending the
//! Factur-X block as a second `rdf:Description` with its own
//! `pdfaExtension:schemas` produces a packet every XML parser accepts and
//! Adobe-lineage parsers and veraPDF reject outright, taking the file's PDF/A
//! identification down with it. The `rdf:li` is therefore merged into the bag
//! that is already there.
//!
//! # What is verified, and what is not
//!
//! `tests/einvoicing.rs` asserts the payload still extracts and that the XMP now
//! agrees with it about the profile and the filename — both checked by
//! `en16931_formats::zugferd`. PDF/A conformance itself is veraPDF's answer, not
//! a Rust test's, and stays outside what this module claims.

use lopdf::{Document, Object};

use super::EInvoiceFormat;
use crate::errors::{InvoicingError, InvoicingResult};

/// The namespace URI, whose mixed case and trailing `#` are both load-bearing.
/// The sample PDFs in the Factur-X 1.0 info package use an all-lowercase
/// spelling that is not correct.
const FX_NS: &str = "urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#";
const FX_VERSION: &str = "1.0";

/// Where the merge goes: the opening of the bag Typst already writes.
const SCHEMAS_BAG_OPEN: &str = "<pdfaExtension:schemas><rdf:Bag>";
const RDF_CLOSE: &str = "</rdf:RDF>";

fn property(name: &str, description: &str) -> String {
    format!(
        "<rdf:li rdf:parseType=\"Resource\">\
<pdfaProperty:name>{name}</pdfaProperty:name>\
<pdfaProperty:valueType>Text</pdfaProperty:valueType>\
<pdfaProperty:category>external</pdfaProperty:category>\
<pdfaProperty:description>{description}</pdfaProperty:description>\
</rdf:li>"
    )
}

/// The `rdf:li` describing the Factur-X schema, to merge into the existing bag.
fn extension_schema_entry() -> String {
    let properties = [
        property("DocumentFileName", "name of the embedded XML invoice file"),
        property("DocumentType", "INVOICE"),
        property("Version", "the actual version of the standard"),
        property(
            "ConformanceLevel",
            "the conformance level of the embedded XML",
        ),
    ]
    .concat();

    format!(
        "<rdf:li rdf:parseType=\"Resource\">\
<pdfaSchema:schema>Factur-X PDFA Extension Schema</pdfaSchema:schema>\
<pdfaSchema:namespaceURI>{FX_NS}</pdfaSchema:namespaceURI>\
<pdfaSchema:prefix>fx</pdfaSchema:prefix>\
<pdfaSchema:property><rdf:Seq>{properties}</rdf:Seq></pdfaSchema:property>\
</rdf:li>"
    )
}

/// The properties themselves. A separate `rdf:Description` is fine — the
/// one-per-packet rule is about properties, and these appear nowhere else.
fn document_description(filename: &str, conformance_level: &str) -> String {
    format!(
        "<rdf:Description xmlns:fx=\"{FX_NS}\" rdf:about=\"\">\
<fx:DocumentType>INVOICE</fx:DocumentType>\
<fx:DocumentFileName>{filename}</fx:DocumentFileName>\
<fx:Version>{FX_VERSION}</fx:Version>\
<fx:ConformanceLevel>{conformance_level}</fx:ConformanceLevel>\
</rdf:Description>"
    )
}

fn stamp_packet(xmp: &str, filename: &str, conformance_level: &str) -> InvoicingResult<String> {
    if xmp.contains(FX_NS) {
        return Ok(xmp.to_owned());
    }

    let bag = xmp.find(SCHEMAS_BAG_OPEN).ok_or_else(|| {
        InvoicingError::PdfGenerationError(
            "the PDF's XMP carries no pdfaExtension:schemas bag to merge into".to_string(),
        )
    })? + SCHEMAS_BAG_OPEN.len();

    let mut out = String::with_capacity(xmp.len() + 2048);
    out.push_str(&xmp[..bag]);
    out.push_str(&extension_schema_entry());
    out.push_str(&xmp[bag..]);

    let rdf_close = out.rfind(RDF_CLOSE).ok_or_else(|| {
        InvoicingError::PdfGenerationError("the PDF's XMP has no rdf:RDF to extend".to_string())
    })?;
    out.insert_str(
        rdf_close,
        &document_description(filename, conformance_level),
    );

    Ok(out)
}

/// Declare the embedded payload in the PDF's XMP, so a receiver knows the file
/// is an invoice before it opens it.
pub fn stamp(pdf: &[u8], format: EInvoiceFormat, filename: &str) -> InvoicingResult<Vec<u8>> {
    let conformance_level = format.xmp_conformance_level().ok_or_else(|| {
        InvoicingError::PdfGenerationError(format!(
            "{} is not carried inside a PDF",
            format.profile_name()
        ))
    })?;

    let mut doc = Document::load_mem(pdf).map_err(|e| {
        InvoicingError::PdfGenerationError(format!("the generated PDF is unreadable: {e}"))
    })?;

    let metadata_id = match doc.catalog().and_then(|c| c.get(b"Metadata")) {
        Ok(Object::Reference(id)) => *id,
        _ => {
            return Err(InvoicingError::PdfGenerationError(
                "the generated PDF carries no XMP metadata stream".to_string(),
            ));
        }
    };

    let stream = doc
        .get_object_mut(metadata_id)
        .and_then(Object::as_stream_mut)
        .map_err(|e| {
            InvoicingError::PdfGenerationError(format!("the XMP metadata is not a stream: {e}"))
        })?;

    let xmp = std::str::from_utf8(&stream.content).map_err(|e| {
        InvoicingError::PdfGenerationError(format!("the XMP metadata is not UTF-8: {e}"))
    })?;
    let stamped = stamp_packet(xmp, filename, conformance_level)?;

    // PDF/A requires the document metadata stream to carry no filter, which is
    // what `set_plain_content` preserves — `set_content` would be free to encode
    // it and the file would stop being PDF/A with nothing to show for it.
    stream.set_plain_content(stamped.into_bytes());

    let mut out = Vec::with_capacity(pdf.len() + 4096);
    doc.save_to(&mut out).map_err(|e| {
        InvoicingError::PdfGenerationError(format!("could not rewrite the PDF metadata: {e}"))
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKET: &str = "<rdf:RDF><rdf:Description><pdfaExtension:schemas><rdf:Bag>\
<rdf:li>existing</rdf:li></rdf:Bag></pdfaExtension:schemas></rdf:Description></rdf:RDF>";

    #[test]
    fn the_schema_is_merged_into_the_one_bag_that_is_already_there() {
        let out = stamp_packet(PACKET, "factur-x.xml", "EN 16931").unwrap();
        assert_eq!(
            out.matches("<pdfaExtension:schemas>").count(),
            1,
            "a second schemas property invalidates the whole packet"
        );
        assert!(out.contains("<rdf:li>existing</rdf:li>"));
        assert!(out.contains("Factur-X PDFA Extension Schema"));
    }

    #[test]
    fn the_properties_state_the_payload_the_pdf_actually_carries() {
        let out = stamp_packet(PACKET, "factur-x.xml", "EN 16931").unwrap();
        assert!(out.contains("<fx:DocumentFileName>factur-x.xml</fx:DocumentFileName>"));
        assert!(out.contains("<fx:ConformanceLevel>EN 16931</fx:ConformanceLevel>"));
        assert!(out.ends_with("</rdf:RDF>"));
    }

    #[test]
    fn stamping_twice_changes_nothing() {
        let once = stamp_packet(PACKET, "factur-x.xml", "EN 16931").unwrap();
        let twice = stamp_packet(&once, "factur-x.xml", "EN 16931").unwrap();
        assert_eq!(once, twice);
    }
}
