//! `#[derive(Describe)]`, re-exported by `botconf` (its `derive` feature):
//! a config type's [description](https://docs.rs/botconf/latest/botconf/describe/)
//! from its doc comments and serde's own attributes, plus `#[botconf(…)]`
//! for what serde does not say. See `botconf::describe` for the attributes.

use proc_macro::TokenStream;

mod attrs;
mod case;
mod doc;
mod expand;

#[proc_macro_derive(Describe, attributes(botconf))]
pub fn derive_describe(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    expand::describe(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
