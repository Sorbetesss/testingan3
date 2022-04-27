// Copyright 2019-2022 Parity Technologies (UK) Ltd.
// This file is part of subxt.
//
// subxt is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// subxt is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with subxt.  If not, see <http://www.gnu.org/licenses/>.

extern crate proc_macro;

use darling::FromMeta;
use proc_macro::TokenStream;
use proc_macro_error::proc_macro_error;
use subxt_codegen::{DerivesRegistry, Derives};
use syn::{
    parse_macro_input,
    punctuated::Punctuated,
};

#[derive(Debug, FromMeta)]
struct RuntimeMetadataArgs {
    runtime_metadata_path: String,
    #[darling(multiple)]
    generated_type_derives: Vec<GeneratedTypeDerives>,
}

#[derive(Debug, FromMeta)]
struct GeneratedTypeDerives {
    #[darling(rename = "type")]
    ty: String,
    derive: Punctuated<syn::Path, syn::Token![,]>,
}

#[proc_macro_attribute]
#[proc_macro_error]
pub fn subxt(args: TokenStream, input: TokenStream) -> TokenStream {
    let attr_args = parse_macro_input!(args as syn::AttributeArgs);
    let item_mod = parse_macro_input!(input as syn::ItemMod);
    let args = match RuntimeMetadataArgs::from_list(&attr_args) {
        Ok(v) => v,
        Err(e) => return TokenStream::from(e.write_errors()),
    };

    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let root_path = std::path::Path::new(&root);
    let path = root_path.join(args.runtime_metadata_path);


    subxt_codegen::generate_runtime_api(item_mod, &path, &args.generated_type_derives)
        .into()
}
