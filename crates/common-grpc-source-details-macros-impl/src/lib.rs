extern crate proc_macro;

use proc_macro::TokenStream;

use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(SourceDetailsError)]
pub fn source_details_error_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = input.ident;

    let output = quote! {
        impl SourceDetailsError for #name {
            #[track_caller]
            fn as_metadata_map(&self) -> tonic::metadata::MetadataMap {
                let mut metadata = tonic::metadata::MetadataMap::new();

                // store 'source' and `location' in metadata
                //   workaround of logging detailed error only on server side
                if let Some(source) = self.source() {
                    let source_details = common_grpc_source_details_macros::SourceDetails {
                        source: source.to_string(),
                        location: std::panic::Location::caller().to_string(),
                    };

                    let json = serde_json::to_string(&source_details);

                    metadata.insert_bin(
                        common_grpc::middleware::common::error_logger::HEADER_SOURCE_DETAILS,
                        tonic::metadata::MetadataValue::from_bytes(json.unwrap().as_bytes()),
                    );
                }

                metadata
            }

            #[track_caller]
            fn as_status(&self, code: tonic::Code) -> tonic::Status {
                tonic::Status::with_metadata(code, self.to_string(), self.as_metadata_map())
            }
        }
    };

    // Return the generated code as a token stream
    output.into()
}
