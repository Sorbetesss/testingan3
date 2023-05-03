// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use crate::{types::TypeGenerator, CodegenError, CratePath};
use frame_metadata::v15::{RuntimeApiMetadata, RuntimeMetadataV15};
use heck::ToSnakeCase as _;
use heck::ToUpperCamelCase as _;

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use scale_info::form::PortableForm;

/// Generates runtime functions for the given API metadata.
fn generate_runtime_api(
    metadata: &RuntimeMetadataV15,
    api: &RuntimeApiMetadata<PortableForm>,
    type_gen: &TypeGenerator,
    types_mod_ident: &syn::Ident,
    crate_path: &CratePath,
    should_gen_docs: bool,
) -> Result<(TokenStream2, TokenStream2), CodegenError> {
    // Trait name must remain as is (upper case) to identity the runtime call.
    let trait_name = &api.name;
    // The snake case for the trait name.
    let trait_name_snake = format_ident!("{}", api.name.to_snake_case());
    let docs = &api.docs;
    let docs: TokenStream2 = should_gen_docs
        .then_some(quote! { #( #[doc = #docs ] )* })
        .unwrap_or_default();

    let structs_and_methods: Vec<_> = api.methods.iter().map(|method| {
        let method_name = format_ident!("{}", method.name);

        // Runtime function name is `TraitName_MethodName`.
        let runtime_fn_name = format!("{}_{}", trait_name, method_name);
        let docs = &method.docs;
        let docs: TokenStream2 = should_gen_docs
            .then_some(quote! { #( #[doc = #docs ] )* })
            .unwrap_or_default();

        let inputs: Vec<_> = method.inputs.iter().map(|input| {
            let name = format_ident!("{}", &input.name);
            let ty = type_gen.resolve_type_path(input.ty.id);

            let param = quote!(#name: #ty);
            (param, name)
        }).collect();

        let params = inputs.iter().map(|(param, _)| param);
        let param_names = inputs.iter().map(|(_, name)| name);

        // From the method metadata generate a structure that holds
        // all parameter types. This structure is used with metadata
        // to encode parameters to the call via `encode_as_fields_to`.
        let derives = type_gen.default_derives();
        let struct_name = format_ident!("{}", method.name.to_upper_camel_case());
        let struct_params = params.clone();
        let struct_input = quote!(
            #derives
            pub struct #struct_name {
                #( pub #struct_params, )*
            }
        );

        let output = type_gen.resolve_type_path(method.output.id);

        let Ok(call_hash) =
            subxt_metadata::get_runtime_api_hash(metadata, trait_name, &method.name) else {
                return Err(CodegenError::MissingRuntimeApiMetadata(
                    trait_name.into(),
                    method.name.clone(),
                ))
            };

        let method = quote!(
            #docs
            pub fn #method_name(&self, #( #params, )* ) -> #crate_path::runtime_api::Payload<types::#struct_name, #output> {
                #crate_path::runtime_api::Payload::new_static(
                    #runtime_fn_name,
                    types::#struct_name { #( #param_names, )* },
                    [#(#call_hash,)*],
                )
            }
        );

        Ok((struct_input, method))
    }).collect::<Result<_, _>>()?;

    let trait_name = format_ident!("{}", trait_name);

    let structs = structs_and_methods.iter().map(|(struct_, _)| struct_);
    let methods = structs_and_methods.iter().map(|(_, method)| method);

    let runtime_api = quote!(
        pub mod #trait_name_snake {
            use super::root_mod;
            use super::#types_mod_ident;

            #docs
            pub struct #trait_name;

            impl #trait_name {
                #( #methods )*
            }

            pub mod types {
                use super::#types_mod_ident;

                #( #structs )*
            }
        }
    );

    // A getter for the `RuntimeApi` to get the trait structure.
    let trait_getter = quote!(
        pub fn #trait_name_snake(&self) -> #trait_name_snake::#trait_name {
            #trait_name_snake::#trait_name
        }
    );

    Ok((runtime_api, trait_getter))
}

/// Generate the runtime APIs.
pub fn generate_runtime_apis(
    metadata: &RuntimeMetadataV15,
    type_gen: &TypeGenerator,
    types_mod_ident: &syn::Ident,
    crate_path: &CratePath,
    should_gen_docs: bool,
) -> Result<TokenStream2, CodegenError> {
    let apis = &metadata.apis;

    let runtime_fns: Vec<_> = apis
        .iter()
        .map(|api| {
            generate_runtime_api(
                metadata,
                api,
                type_gen,
                types_mod_ident,
                crate_path,
                should_gen_docs,
            )
        })
        .collect::<Result<_, _>>()?;

    let runtime_apis_def = runtime_fns.iter().map(|(apis, _)| apis);
    let runtime_apis_getters = runtime_fns.iter().map(|(_, getters)| getters);

    Ok(quote! {
        pub mod runtime_apis {
            use super::root_mod;
            use super::#types_mod_ident;

            use #crate_path::ext::codec::Encode;

            pub struct RuntimeApi;

            impl RuntimeApi {
                #( #runtime_apis_getters )*
            }

            #( #runtime_apis_def )*
        }
    })
}
