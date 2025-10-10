extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemTrait, PatType, TraitItem, parse_macro_input, parse_quote};

/// Define a helper attribute to mark methods for transformation
#[proc_macro_attribute]
pub fn delegated(_attr: TokenStream, _item: TokenStream) -> TokenStream {
    // This is just a marker attribute, return the item unchanged
    _item
}

/// A proc macro applied to a trait that transforms methods with #[delegated] to their `with_conn` versions
/// and auto-generates a separate trait that implements the original versions.
#[proc_macro_attribute]
pub fn with_conn_delegate(attr: TokenStream, input: TokenStream) -> TokenStream {
    // Parse the attribute to check if "all" is specified
    let generate_all = !attr.is_empty();

    // Parse the input as a trait definition
    let input_trait = parse_macro_input!(input as ItemTrait);
    let trait_name = &input_trait.ident;
    let auto_trait_name = format_ident!("{}Auto", trait_name);

    // Will hold modified trait methods and auto-trait methods
    let mut modified_trait_methods = Vec::new();
    let mut auto_trait_methods = Vec::new();
    let mut auto_impl_methods = Vec::new();

    // Process all methods in the trait
    for item in &input_trait.items {
        if let TraitItem::Fn(method) = item {
            let original_method_name = &method.sig.ident;

            // Determine if this method should be transformed
            let should_transform = generate_all
                || method
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("delegated"));

            if should_transform {
                // Create with_conn version of the method
                let with_conn_method_name = format_ident!("{}_with_conn", original_method_name);
                let return_type = &method.sig.output;

                // Filter out the generate attribute
                let filtered_attrs: Vec<_> = method
                    .attrs
                    .iter()
                    .filter(|attr| !attr.path().is_ident("delegated"))
                    .collect();

                // Extract original method parameters (excluding &self)
                let mut params = Vec::new();
                let mut param_names = Vec::new();

                for input in method.sig.inputs.iter().skip(1) {
                    // Skip &self
                    if let FnArg::Typed(pat_type) = input {
                        params.push(pat_type.clone());

                        // Extract name for later use
                        if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                            param_names.push(pat_ident.ident.clone());
                        }
                    }
                }

                // Create the with_conn version method for the modified trait
                let conn_param: PatType = parse_quote! {
                    conn: &mut PgConn
                };
                let mut with_conn_params = vec![conn_param];
                with_conn_params.extend(params.clone());

                let with_conn_method: TraitItem = parse_quote! {
                    #(#filtered_attrs)*
                    async fn #with_conn_method_name(
                        &self,
                        #(#with_conn_params),*
                    ) #return_type;
                };
                modified_trait_methods.push(with_conn_method);

                // Create auto trait method (original method signature)
                let auto_trait_method: TraitItem = parse_quote! {
                    async fn #original_method_name(
                        &self,
                        #(#params),*
                    ) #return_type;
                };
                auto_trait_methods.push(auto_trait_method);

                // Create auto implementation that delegates to with_conn
                let auto_impl = quote! {
                    async fn #original_method_name(&self, #(#params),*) #return_type {
                        let mut conn = self.get_conn().await?;
                        self.#with_conn_method_name(&mut conn, #(#param_names),*).await
                    }
                };
                auto_impl_methods.push(auto_impl);
            } else {
                // Keep non-transformed methods as-is
                modified_trait_methods.push(item.clone());
            }
        } else {
            // Keep non-function items as-is
            modified_trait_methods.push(item.clone());
        }
    }

    // Create the modified trait (original trait name but with with_conn methods)
    let vis = &input_trait.vis;
    let modified_trait = quote! {
        #[async_trait::async_trait]
        #vis trait #trait_name {
            #(#modified_trait_methods)*
        }
    };

    // Create the auto trait with original method signatures
    let auto_trait = quote! {
        #[async_trait::async_trait]
        #vis trait #auto_trait_name {
            #(#auto_trait_methods)*
        }
    };

    // Create the auto-implementation
    let auto_impl = quote! {
        // Auto-implementation for methods that need with_conn
        #[async_trait::async_trait]
        impl #auto_trait_name for Store {
            #(#auto_impl_methods)*
        }
    };

    // Return all the generated code
    TokenStream::from(quote! {
        // Modified trait (original name but with _with_conn methods)
        #modified_trait

        // Auto trait with original method signatures
        #auto_trait

        // Auto implementation that delegates to with_conn methods
        #auto_impl
    })
}
