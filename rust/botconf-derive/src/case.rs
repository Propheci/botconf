//! serde's `rename_all` rules, applied as serde applies them.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameRule {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

impl RenameRule {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "lowercase" => Self::Lower,
            "UPPERCASE" => Self::Upper,
            "PascalCase" => Self::Pascal,
            "camelCase" => Self::Camel,
            "snake_case" => Self::Snake,
            "SCREAMING_SNAKE_CASE" => Self::ScreamingSnake,
            "kebab-case" => Self::Kebab,
            "SCREAMING-KEBAB-CASE" => Self::ScreamingKebab,
            _ => return None,
        })
    }

    /// A variant name, written in `PascalCase`.
    pub fn apply_to_variant(self, variant: &str) -> String {
        match self {
            Self::Pascal => variant.to_owned(),
            Self::Lower => variant.to_ascii_lowercase(),
            Self::Upper => variant.to_ascii_uppercase(),
            Self::Camel => lower_first(variant),
            Self::Snake => {
                let mut snake = String::new();
                for (index, c) in variant.char_indices() {
                    if index > 0 && c.is_uppercase() {
                        snake.push('_');
                    }
                    snake.push(c.to_ascii_lowercase());
                }
                snake
            }
            Self::ScreamingSnake => Self::Snake.apply_to_variant(variant).to_ascii_uppercase(),
            Self::Kebab => Self::Snake.apply_to_variant(variant).replace('_', "-"),
            Self::ScreamingKebab => Self::ScreamingSnake
                .apply_to_variant(variant)
                .replace('_', "-"),
        }
    }

    /// A field name, written in `snake_case`.
    pub fn apply_to_field(self, field: &str) -> String {
        match self {
            Self::Lower | Self::Snake => field.to_owned(),
            Self::Upper | Self::ScreamingSnake => field.to_ascii_uppercase(),
            Self::Pascal => {
                let mut pascal = String::new();
                let mut capitalize = true;
                for c in field.chars() {
                    if c == '_' {
                        capitalize = true;
                    } else if capitalize {
                        pascal.push(c.to_ascii_uppercase());
                        capitalize = false;
                    } else {
                        pascal.push(c);
                    }
                }
                pascal
            }
            Self::Camel => lower_first(&Self::Pascal.apply_to_field(field)),
            Self::Kebab => field.replace('_', "-"),
            Self::ScreamingKebab => Self::ScreamingSnake.apply_to_field(field).replace('_', "-"),
        }
    }
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::RenameRule::{self, *};

    #[test]
    fn rules_rename_like_serde() {
        let cases: [(RenameRule, &str, &str); 8] = [
            (Lower, "outcome", "manytoone"),
            (Upper, "OUTCOME", "MANYTOONE"),
            (Pascal, "Outcome", "ManyToOne"),
            (Camel, "outcome", "manyToOne"),
            (Snake, "outcome", "many_to_one"),
            (ScreamingSnake, "OUTCOME", "MANY_TO_ONE"),
            (Kebab, "outcome", "many-to-one"),
            (ScreamingKebab, "OUTCOME", "MANY-TO-ONE"),
        ];
        for (rule, field, variant) in cases {
            assert_eq!(rule.apply_to_field("outcome"), field, "{rule:?}");
            assert_eq!(rule.apply_to_variant("ManyToOne"), variant, "{rule:?}");
        }
        assert_eq!(
            Camel.apply_to_field("max_message_length"),
            "maxMessageLength"
        );
        assert_eq!(Kebab.apply_to_field("token_env"), "token-env");
    }
}
