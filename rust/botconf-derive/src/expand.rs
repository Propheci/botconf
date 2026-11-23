//! The generated `Describe` impl.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DataEnum, DeriveInput, Fields, FieldsNamed, Path, Type, parse_quote};

use crate::attrs::{Botconf, DefaultFrom, Serde};
use crate::case::RenameRule;
use crate::doc::doc_of;

pub fn describe(input: &DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "Describe cannot be derived for a generic type; implement it by hand",
        ));
    }
    let serde = Serde::parse(&input.attrs)?;
    let botconf = Botconf::parse(&input.attrs)?;
    let krate: Path = botconf
        .krate
        .clone()
        .unwrap_or_else(|| parse_quote!(::botconf));
    let cx = Cx { krate: &krate };
    let node = match cx.value_override(&botconf, None) {
        Some(node) => node,
        None => match &input.data {
            Data::Struct(data) => cx.struct_node(&data.fields, &serde)?,
            Data::Enum(data) => cx.enum_node(input, data, &serde)?,
            Data::Union(_) => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "Describe cannot be derived for a union",
                ));
            }
        },
    };
    let name = &input.ident;
    let doc = doc_of(&input.attrs);
    Ok(quote! {
        #[automatically_derived]
        impl #krate::describe::Describe for #name {
            fn node() -> #krate::describe::Node {
                #node
            }

            fn doc() -> ::std::string::String {
                ::std::string::String::from(#doc)
            }
        }
    })
}

struct Cx<'a> {
    krate: &'a Path,
}

impl Cx<'_> {
    /// A Rust expression written in an attribute, with botconf's names in
    /// scope.
    fn user(&self, tokens: &TokenStream) -> TokenStream {
        let krate = self.krate;
        quote! {{
            #[allow(unused_imports)]
            use #krate::{*, describe::*};
            #tokens
        }}
    }

    /// The node `kind`, `secret` or `env` set, ignoring the type's.
    fn value_override(&self, botconf: &Botconf, bounded: Option<&Type>) -> Option<TokenStream> {
        let krate = self.krate;
        let kind = if let Some(kind) = &botconf.kind {
            self.user(kind)
        } else if botconf.secret {
            quote!(#krate::describe::ValueKind::Secret)
        } else if let Some(env) = &botconf.env {
            quote!(#krate::describe::ValueKind::EnvVar(#krate::describe::EnvValue::#env))
        } else if let (Some(ty), true) = (bounded, botconf.min.is_some() || botconf.max.is_some()) {
            let bound = |bound: &Option<TokenStream>| match bound {
                Some(tokens) => {
                    let tokens = self.user(tokens);
                    quote!(::core::option::Option::Some(#tokens))
                }
                None => quote!(::core::option::Option::None),
            };
            let (min, max) = (bound(&botconf.min), bound(&botconf.max));
            return Some(quote! {
                #krate::describe::__private::bounded(
                    <#ty as #krate::describe::Describe>::node(), #min, #max,
                )
            });
        } else {
            return None;
        };
        Some(quote!(#krate::describe::Node::Value(#kind)))
    }

    fn struct_node(&self, fields: &Fields, serde: &Serde) -> syn::Result<TokenStream> {
        let krate = self.krate;
        match fields {
            Fields::Named(named) => {
                self.table_node(named, serde.rename_all, serde.default.is_some())
            }
            // A newtype reads as what it wraps.
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let ty = &unnamed.unnamed[0].ty;
                Ok(quote!(<#ty as #krate::describe::Describe>::node()))
            }
            _ => Err(syn::Error::new_spanned(
                fields,
                "Describe needs named fields or one wrapped value; set #[botconf(kind = …)] or \
                 implement it by hand",
            )),
        }
    }

    /// A table of named fields. `container_default`: the type's `Default`
    /// gives every field's default (`#[serde(default)]` on the type).
    fn table_node(
        &self,
        fields: &FieldsNamed,
        rename_all: Option<RenameRule>,
        container_default: bool,
    ) -> syn::Result<TokenStream> {
        let krate = self.krate;
        let mut steps = Vec::new();
        for field in &fields.named {
            let serde = Serde::parse(&field.attrs)?;
            let botconf = Botconf::parse(&field.attrs)?;
            if serde.skip || botconf.skip {
                continue;
            }
            let ty = &field.ty;
            if serde.flatten {
                steps.push(quote! {
                    table.flatten(<#ty as #krate::describe::Describe>::node());
                });
                continue;
            }
            let ident = field.ident.as_ref().expect("named fields have names");
            let name = ident.to_string();
            let name = name.strip_prefix("r#").unwrap_or(&name);
            let key = match (&serde.rename, rename_all) {
                (Some(rename), _) => rename.clone(),
                (None, Some(rule)) => rule.apply_to_field(name),
                (None, None) => name.to_owned(),
            };
            let default = match (&serde.default, container_default) {
                (Some(DefaultFrom::Trait), _) => {
                    Some(quote!(&<#ty as ::core::default::Default>::default()))
                }
                (Some(DefaultFrom::Function(path)), _) => Some(quote!(&#path())),
                (None, true) => Some(quote!(&defaults.#ident)),
                (None, false) => None,
            };
            if let (Some(span), None) = (serde.custom, &botconf.kind) {
                return Err(syn::Error::new(
                    span,
                    "the file's shape is not the field type's: set #[botconf(kind = …)] (or \
                     #[botconf(skip)])",
                ));
            }
            let required = default.is_none() && !is_option(ty);
            // Read through `with`, the file does not write the default as
            // the type serializes it.
            let default = default.filter(|_| serde.custom.is_none());
            steps.push(self.entry(&key, ty, &field.attrs, &botconf, default, required));
        }
        let defaults = container_default
            .then(|| quote!(let defaults = <Self as ::core::default::Default>::default();));
        Ok(quote! {{
            #defaults
            let mut table = #krate::describe::__private::TableBuilder::default();
            #(#steps)*
            table.build()
        }})
    }

    /// `table.entry(…)` for one field.
    fn entry(
        &self,
        key: &str,
        ty: &Type,
        attrs: &[syn::Attribute],
        botconf: &Botconf,
        default: Option<TokenStream>,
        required: bool,
    ) -> TokenStream {
        let krate = self.krate;
        let private = quote!(#krate::describe::__private);
        let node = self
            .value_override(botconf, Some(ty))
            .unwrap_or_else(|| quote!(<#ty as #krate::describe::Describe>::node()));
        let doc = doc_of(attrs);
        let description = if doc.is_empty() {
            quote!(<#ty as #krate::describe::Describe>::doc())
        } else {
            quote!(::std::string::String::from(#doc))
        };
        let mut sets = Vec::new();
        if let Some(title) = &botconf.title {
            sets.push(quote!(entry.title = ::std::string::String::from(#title);));
        }
        if required {
            sets.push(quote!(entry.required = true;));
        }
        if is_option(ty) {
            sets.push(quote!(entry.nullable = true;));
        }
        if let Some(value) = default {
            // Resolved here, on the concrete type: a default that is not
            // `Serialize` is left out rather than refused.
            sets.push(quote! {
                entry.default = {
                    #[allow(unused_imports)]
                    use #private::{BySerialize as _, Otherwise as _};
                    (&#private::Json(#value)).get()
                };
            });
        }
        for (field, tokens) in [("example", &botconf.example), ("hint", &botconf.hint)] {
            if let Some(tokens) = tokens {
                let field = format_ident!("{field}");
                sets.push(quote! {
                    entry.#field = ::core::option::Option::Some(#private::json!(#tokens));
                });
            }
        }
        if botconf.runtime {
            sets.push(quote!(entry.runtime = true;));
        }
        if let Some(panel) = &botconf.panel {
            let panel = self.user(panel);
            sets.push(quote!(entry.panel_kind = ::core::option::Option::Some(#panel);));
        }
        quote! {
            table.entry({
                let mut entry = #krate::describe::Entry::new(#key, #node);
                entry.description = #description;
                #(#sets)*
                entry
            });
        }
    }

    fn enum_node(
        &self,
        input: &DeriveInput,
        data: &DataEnum,
        serde: &Serde,
    ) -> syn::Result<TokenStream> {
        let krate = self.krate;
        if serde.untagged || serde.content {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "Describe cannot be derived for an untagged or adjacently tagged enum; set \
                 #[botconf(kind = …)] or implement it by hand",
            ));
        }
        let mut variants = Vec::new();
        for variant in &data.variants {
            let variant_serde = Serde::parse(&variant.attrs)?;
            let variant_botconf = Botconf::parse(&variant.attrs)?;
            if variant_serde.skip || variant_serde.other || variant_botconf.skip {
                continue;
            }
            let name = variant.ident.to_string();
            let value = match (&variant_serde.rename, serde.rename_all) {
                (Some(rename), _) => rename.clone(),
                (None, Some(rule)) => rule.apply_to_variant(&name),
                (None, None) => name,
            };
            variants.push((variant, variant_serde, variant_botconf, value));
        }
        let Some(tag) = &serde.tag else {
            // Externally tagged: only unit variants are plain values.
            if let Some((variant, ..)) = variants
                .iter()
                .find(|(variant, ..)| !matches!(variant.fields, Fields::Unit))
            {
                return Err(syn::Error::new_spanned(
                    &variant.ident,
                    "Describe reads an enum with data only when it is internally tagged \
                     (#[serde(tag = \"…\")]); set #[botconf(kind = …)] or implement it by hand",
                ));
            }
            let values = variants.iter().map(|(.., value)| value);
            return Ok(quote! {
                #krate::describe::Node::Value(#krate::describe::ValueKind::OneOf(
                    ::std::vec![#(::std::string::String::from(#values)),*],
                ))
            });
        };
        let mut built = Vec::new();
        for (variant, variant_serde, variant_botconf, value) in &variants {
            let table = match &variant.fields {
                Fields::Unit => quote!(#krate::describe::Table::default()),
                Fields::Named(named) => {
                    let node = self.table_node(named, variant_serde.rename_all, false)?;
                    quote!(#krate::describe::__private::table_of(#node))
                }
                Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                    let ty = &unnamed.unnamed[0].ty;
                    quote! {
                        #krate::describe::__private::table_of(
                            <#ty as #krate::describe::Describe>::node(),
                        )
                    }
                }
                Fields::Unnamed(_) => {
                    return Err(syn::Error::new_spanned(
                        &variant.ident,
                        "a tagged variant holds named fields or one table",
                    ));
                }
            };
            let title = match &variant_botconf.title {
                Some(title) => quote!(::std::string::String::from(#title)),
                None => quote!(#krate::describe::title_of(#value)),
            };
            let doc = doc_of(&variant.attrs);
            built.push(quote! {
                #krate::describe::Variant {
                    value: ::std::string::String::from(#value),
                    title: #title,
                    description: ::std::string::String::from(#doc),
                    table: #table,
                }
            });
        }
        Ok(quote! {
            #krate::describe::Node::Tagged {
                tag: ::std::string::String::from(#tag),
                common: #krate::describe::Table::default(),
                variants: ::std::vec![#(#built),*],
            }
        })
    }
}

/// `Option<…>`: serde reads an absent key as `None`, default or not.
fn is_option(ty: &Type) -> bool {
    match ty {
        Type::Path(path) if path.qself.is_none() => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Option"),
        _ => false,
    }
}
