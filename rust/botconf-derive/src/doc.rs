//! Doc comments as descriptions.

use syn::{Attribute, Expr, ExprLit, Lit, Meta};

/// The doc comment, its paragraphs' lines joined into one, its intra-doc
/// links (`` [`Type`] ``) written as plain code.
pub fn doc_of(attrs: &[Attribute]) -> String {
    let text = attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            Meta::NameValue(pair) if pair.path.is_ident("doc") => match &pair.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(text),
                    ..
                }) => Some(text.value()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut paragraphs: Vec<Vec<&str>> = vec![Vec::new()];
    for line in text.lines().map(str::trim) {
        match (line.is_empty(), paragraphs.last_mut()) {
            (true, Some(last)) if last.is_empty() => {}
            (true, _) => paragraphs.push(Vec::new()),
            (false, Some(last)) => last.push(line),
            (false, None) => unreachable!("there is always a paragraph"),
        }
    }
    let joined = paragraphs
        .iter()
        .filter(|lines| !lines.is_empty())
        .map(|lines| lines.join(" "))
        .collect::<Vec<_>>()
        .join("\n\n");
    unlink(&joined)
}

/// `` [`Type`] `` → `` `Type` ``; a link with a target (`[x](y)`, `[x][y]`)
/// is left alone.
fn unlink(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[`") {
        let after = &rest[start + 1..];
        let Some(end) = after.find("`]") else { break };
        let code = &after[..end + 1];
        let tail = &after[end + 2..];
        out.push_str(&rest[..start]);
        if tail.starts_with('(') || tail.starts_with('[') {
            out.push('[');
            out.push_str(code);
            out.push(']');
        } else {
            out.push_str(code);
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_join_and_links_unlink() {
        let item: syn::DeriveInput = syn::parse_quote! {
            /// Where the [`Core`] listens:
            ///   an address.
            ///
            ///
            /// See [`the docs`](https://x).
            struct S;
        };
        assert_eq!(
            doc_of(&item.attrs),
            "Where the `Core` listens: an address.\n\nSee [`the docs`](https://x)."
        );
    }
}
