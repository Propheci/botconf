//! What serde's attributes and `#[botconf(…)]` say about an item.

use proc_macro2::{Span, TokenStream, TokenTree};
use syn::{
    Attribute, Ident, Lit, LitStr, Path, Token,
    ext::IdentExt as _,
    meta::ParseNestedMeta,
    parse::{ParseStream, Parser as _},
    spanned::Spanned as _,
};

use crate::case::RenameRule;

/// Where a field's default comes from.
pub enum DefaultFrom {
    /// `#[serde(default)]`: the type's `Default`.
    Trait,
    /// `#[serde(default = "path")]`.
    Function(Path),
}

/// The serde attributes the description depends on; the others are read
/// past.
#[derive(Default)]
pub struct Serde {
    pub rename: Option<String>,
    pub rename_all: Option<RenameRule>,
    pub tag: Option<String>,
    /// `content = "…"` (adjacently tagged).
    pub content: bool,
    pub untagged: bool,
    pub default: Option<DefaultFrom>,
    /// `skip` or `skip_deserializing`: never read from the file.
    pub skip: bool,
    pub flatten: bool,
    /// `#[serde(other)]`: the catch-all unit variant.
    pub other: bool,
    /// `with`, `deserialize_with`, `from`, `try_from`: the file's shape is not
    /// the type's.
    pub custom: Option<Span>,
}

impl Serde {
    pub fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut serde = Self::default();
        for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
            attr.parse_nested_meta(|meta| serde.read(&meta))?;
        }
        Ok(serde)
    }

    fn read(&mut self, meta: &ParseNestedMeta) -> syn::Result<()> {
        let key = meta
            .path
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        match key.as_str() {
            "rename" => {
                if let Some(name) = deserialize_name(meta)? {
                    self.rename = Some(name.value());
                }
            }
            "rename_all" => {
                if let Some(name) = deserialize_name(meta)? {
                    let rule = RenameRule::parse(&name.value())
                        .ok_or_else(|| syn::Error::new(name.span(), "unknown rename_all rule"))?;
                    self.rename_all = Some(rule);
                }
            }
            "tag" => self.tag = Some(meta.value()?.parse::<LitStr>()?.value()),
            "content" => {
                self.content = true;
                skip_value(meta)?;
            }
            "untagged" => self.untagged = true,
            "default" => {
                self.default = Some(if meta.input.peek(Token![=]) {
                    DefaultFrom::Function(meta.value()?.parse::<LitStr>()?.parse()?)
                } else {
                    DefaultFrom::Trait
                });
            }
            "skip" | "skip_deserializing" => self.skip = true,
            "flatten" => self.flatten = true,
            "other" => self.other = true,
            "with" | "deserialize_with" | "from" | "try_from" => {
                self.custom = Some(meta.path.span());
                skip_value(meta)?;
            }
            _ => skip_value(meta)?,
        }
        Ok(())
    }
}

/// `rename = "x"`, or the `deserialize` one of `rename(serialize = "…",
/// deserialize = "…")`.
fn deserialize_name(meta: &ParseNestedMeta) -> syn::Result<Option<LitStr>> {
    if meta.input.peek(Token![=]) {
        return Ok(Some(meta.value()?.parse()?));
    }
    let mut name = None;
    meta.parse_nested_meta(|inner| {
        if inner.path.is_ident("deserialize") {
            name = Some(inner.value()?.parse()?);
            Ok(())
        } else {
            skip_value(&inner)
        }
    })?;
    Ok(name)
}

fn skip_value(meta: &ParseNestedMeta) -> syn::Result<()> {
    if meta.input.peek(Token![=]) {
        meta.value()?.parse::<Lit>()?;
    } else if meta.input.peek(syn::token::Paren) {
        meta.parse_nested_meta(|inner| skip_value(&inner))?;
    }
    Ok(())
}

/// `#[botconf(…)]`: what serde does not say. Values are written as Rust
/// (`kind`, `panel`, `min`, `max`) or as JSON (`example`, `hint`).
#[derive(Default)]
pub struct Botconf {
    pub title: Option<LitStr>,
    pub runtime: bool,
    /// `per_entry`: a runtime map set entry by entry (implies `runtime`).
    pub per_entry: bool,
    pub secret: bool,
    /// `env` (a plain value) or `env = token | secret | plain`.
    pub env: Option<Ident>,
    pub kind: Option<TokenStream>,
    pub panel: Option<TokenStream>,
    pub min: Option<TokenStream>,
    pub max: Option<TokenStream>,
    pub example: Option<TokenStream>,
    pub hint: Option<TokenStream>,
    pub skip: bool,
    /// `crate = "path"`: where botconf is, when not `::botconf`.
    pub krate: Option<Path>,
}

impl Botconf {
    pub fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut botconf = Self::default();
        for attr in attrs.iter().filter(|attr| attr.path().is_ident("botconf")) {
            let tokens = attr.meta.require_list()?.tokens.clone();
            (|input: ParseStream| botconf.read(input)).parse2(tokens)?;
        }
        Ok(botconf)
    }

    fn read(&mut self, input: ParseStream) -> syn::Result<()> {
        while !input.is_empty() {
            let key = Ident::parse_any(input)?;
            let value = if input.peek(Token![=]) {
                input.parse::<Token![=]>()?;
                Some(until_comma(input)?)
            } else {
                None
            };
            let required = |value: Option<TokenStream>| {
                value.ok_or_else(|| syn::Error::new(key.span(), format!("`{key}` takes a value")))
            };
            let flag = |value: Option<TokenStream>| match value {
                None => Ok(true),
                Some(_) => Err(syn::Error::new(
                    key.span(),
                    format!("`{key}` takes no value"),
                )),
            };
            match key.to_string().as_str() {
                "title" => self.title = Some(syn::parse2(required(value)?)?),
                "runtime" => self.runtime = flag(value)?,
                "per_entry" => {
                    self.per_entry = flag(value)?;
                    self.runtime = true;
                }
                "secret" => self.secret = flag(value)?,
                "skip" => self.skip = flag(value)?,
                "env" => {
                    let what = match value {
                        None => Ident::new("Plain", key.span()),
                        Some(tokens) => {
                            let what: Ident = syn::parse2(tokens)?;
                            let variant = match what.to_string().as_str() {
                                "token" => "Token",
                                "secret" => "Secret",
                                "plain" => "Plain",
                                _ => {
                                    return Err(syn::Error::new(
                                        what.span(),
                                        "expected `token`, `secret` or `plain`",
                                    ));
                                }
                            };
                            Ident::new(variant, what.span())
                        }
                    };
                    self.env = Some(what);
                }
                "kind" => self.kind = Some(required(value)?),
                "panel" => self.panel = Some(required(value)?),
                "min" => self.min = Some(required(value)?),
                "max" => self.max = Some(required(value)?),
                "example" => self.example = Some(required(value)?),
                "hint" => self.hint = Some(required(value)?),
                "crate" => {
                    let path: LitStr = syn::parse2(required(value)?)?;
                    self.krate = Some(path.parse()?);
                }
                _ => return Err(syn::Error::new(key.span(), "unknown botconf attribute")),
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(())
    }
}

/// The tokens up to the next comma outside any brackets.
fn until_comma(input: ParseStream) -> syn::Result<TokenStream> {
    input.step(|cursor| {
        let mut rest = *cursor;
        let mut tokens = TokenStream::new();
        while let Some((tree, next)) = rest.token_tree() {
            if let TokenTree::Punct(punct) = &tree
                && punct.as_char() == ','
            {
                break;
            }
            tokens.extend([tree]);
            rest = next;
        }
        if tokens.is_empty() {
            return Err(cursor.error("expected a value"));
        }
        Ok((tokens, rest))
    })
}
