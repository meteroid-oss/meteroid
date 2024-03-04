extern crate proc_macro;

use proc_macro::TokenStream;

use quote::quote;
use syn::{parse_macro_input, Attribute, Data, DeriveInput, Meta, NestedMeta};

#[proc_macro_derive(ErrorAsTonic, attributes(code))]
pub fn error_as_tonic_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    error_as_tonic_impl(&ast)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn error_as_tonic_impl(ast: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &ast.ident;

    let variants = match &ast.data {
        Data::Enum(data) => &data.variants,
        _ => {
            return Err(syn::Error::new_spanned(
                &ast,
                "ErrorAsTonic can only be used on enums",
            ))
        }
    };

    let mut arms = proc_macro2::TokenStream::new();

    for variant in variants {
        let ident = &variant.ident;
        let code_attr = variant
            .attrs
            .iter()
            .find(|attr| attr.path.is_ident("code"))
            .ok_or_else(|| syn::Error::new_spanned(variant, "Missing `code` attribute"))?;

        let code = parse_code_attr(code_attr)?;

        let arm = quote! {
            #name::#ident { .. } => #code,
        };

        arms.extend(arm);
    }

    let gen = quote! {
        impl From<#name> for ::tonic::Status {
            fn from(error: #name) -> ::tonic::Status {
                let code = match &error {
                    #arms
                };

                let metadata = ::common_grpc_error_as_tonic_macros::error_to_metadata(&error);

                ::tonic::Status::with_metadata(code, error.to_string(), metadata)
            }
        }
    };

    Ok(gen)
}

fn parse_code_attr(attr: &Attribute) -> syn::Result<proc_macro2::TokenStream> {
    let meta = attr.parse_meta()?;
    let nested = match meta {
        Meta::List(meta) => {
            if meta.nested.len() != 1 {
                return Err(syn::Error::new_spanned(attr, "Expected exactly one `code`"));
            }
            meta.nested.first().unwrap().clone()
        }
        _ => {
            return Err(syn::Error::new_spanned(
                attr,
                "Expected `code` attribute to be a list",
            ))
        }
    };

    match nested {
        NestedMeta::Meta(Meta::Path(path)) => {
            // Assuming the code is one of tonic::Code variants
            // No need to parse a string; it's directly the variant name
            if path.segments.len() == 1 {
                let segment = &path.segments[0];
                let ident = &segment.ident;
                Ok(quote! { ::tonic::Code::#ident })
            } else {
                Err(syn::Error::new_spanned(
                    path,
                    "Expected tonic::Code variant",
                ))
            }
        }
        _ => Err(syn::Error::new_spanned(
            attr,
            "Expected tonic::Code variant for `code` attribute",
        )),
    }
}
