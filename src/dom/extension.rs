use full_moon::{
    ast::*,
    tokenizer::{StringLiteralQuoteType, Symbol, Token, TokenReference, TokenType},
    ShortString,
};
use punctuated::Pair;
use punctuated::Punctuated;
use rbx_dom_weak::{types::Ref, Instance, WeakDom};
use span::ContainedSpan;

use crate::dom::rbx_path::DotPath;

pub const GLOBAL_VAR_NAME: &str = "_proxyGlobals";

/// Macro for producing string literal that indexes the global variable
#[macro_export]
macro_rules! index_global {
    ($s: expr) => {
        concat!("_proxyGlobals", ".", $s)
    };
}

/// Macro for getting nth argument as a string (if arg is string), as achieving it may be tedious
#[macro_export]
macro_rules! nth_arg_string {
    ($args: expr, $n: expr) => {
        match $args {
            FunctionArgs::Parentheses { arguments, .. } => arguments.iter().nth($n).and_then(|arg| {
                if let Expression::String(token) = arg {
                    token.identifier()
                } else {
                    None
                }
            }),

            FunctionArgs::String(token) => {
                if $n == 0 {
                    token.identifier()
                } else {
                    None
                }
            }
            _ => None,
        }
    };
}

pub enum SearchAction {
    /// Continues search, bool specifies if a match was found
    Found(bool),
    /// Stops search and includes this instance
    Return,
    /// Stops search
    Break,
}

pub enum ForEachAction {
    /// Continues foreach
    Continue,
    /// Stops foreach
    Break,
}

pub trait WeakDomExt {
    /// Iterates through the entire descendant tree using DFS and performs an operation
    ///
    /// # Example
    ///
    /// ```ignore
    /// WeakDom.foreach_descendant(&instance, |descendant, path| {
    ///     if descendant.class.as_str() == "Script" {
    ///         println!("Found script at {}, depth: {}", path.to_string(), path.depth());
    ///         return ForEachAction::Break;
    ///     }
    ///     return ForEachAction::Continue;
    /// }
    /// ```
    ///
    /// # Arguments
    ///
    /// * `parent` - Instance to iterate through
    /// * `predicate` - function, receives Instance and current_depth, return ForEachAction - choose to either break or continue iteration, perform any changes here
    /// * `depth` - amount of levels to search, 0 for no limit
    fn foreach_descendant<F>(&self, parent: &Instance, class_predicate: &mut F, depth: u8)
    where
        F: FnMut(&Instance, &DotPath) -> ForEachAction;

    /// Iterates through the entire descendant tree and returns a Vec of all Instance Refs that match
    ///
    /// # Arguments
    ///
    /// * `parent` - Instance to search through
    /// * `predicate` - function, return SearchAction - determines whether Instance should be included in Vec, and if search should stop
    /// * `depth` - amount of levels to search, 0 for no limit
    fn find_descendants<F>(&self, parent: &Instance, class_predicate: F, depth: u8) -> Vec<Ref>
    where
        F: Fn(&Instance) -> SearchAction;

    /// Finds a child class in the tree
    ///
    /// # Arguments
    ///
    /// * `parent` - Instance to search through
    /// * `class_predicate` - function, return true if the class matches
    /// * `depth` - amount of levels to search, 0 for no limit
    fn find_first_child_class<F>(&self, parent: &Instance, class_predicate: F, depth: u8) -> Option<Ref>
    where
        F: Fn(&str) -> bool;
}

impl WeakDomExt for WeakDom {
    fn foreach_descendant<F>(&self, parent: &Instance, predicate: &mut F, depth: u8)
    where
        F: FnMut(&Instance, &DotPath) -> ForEachAction,
    {
        let mut queue = vec![(parent, DotPath::default())];

        while let Some((current, mut path)) = queue.pop() {
            for child_id in current.children() {
                let child = self.get_by_ref(*child_id).expect("child points to null ref?");

                path.push(&child.name);
                if let ForEachAction::Break = predicate(child, &path) {
                    return;
                }

                if path.depth() < depth.into() || depth == 0 {
                    queue.push((child, path.clone()));
                }

                path.pop();
            }
        }
    }

    fn find_descendants<F>(&self, parent: &Instance, predicate: F, depth: u8) -> Vec<Ref>
    where
        F: Fn(&Instance) -> SearchAction,
    {
        let mut matches = Vec::new();

        self.foreach_descendant(
            parent,
            &mut |child, _| match predicate(child) {
                SearchAction::Found(bool) => {
                    if bool {
                        matches.push(child.referent());
                    }
                    ForEachAction::Continue
                }
                SearchAction::Return => {
                    matches.push(child.referent());
                    ForEachAction::Break
                }
                SearchAction::Break => ForEachAction::Break,
            },
            depth,
        );

        matches
    }

    fn find_first_child_class<F>(&self, parent: &Instance, class_predicate: F, depth: u8) -> Option<Ref>
    where
        F: Fn(&str) -> bool,
    {
        let mut result = None;
        self.foreach_descendant(
            parent,
            &mut |child, _| {
                if class_predicate(child.class.as_str()) {
                    result = Some(child.referent());
                    return ForEachAction::Break;
                }
                ForEachAction::Continue
            },
            depth,
        );

        result
    }
}

pub trait TokenRefExt {
    fn new_type(token: TokenType) -> TokenReference;
    fn new_identifier(identifier: &str) -> TokenReference;
    fn with_trivia(&self, leading_whitespace: Option<&str>, trailing_whitespace: Option<&str>) -> TokenReference;
    /// Gets string from TokenReference if contains a identifier or string
    fn identifier(&self) -> Option<&str>;
}

/// Creates new trivia for TokenReference if a string is provided
fn trivia(whitespace: Option<&str>) -> Vec<Token> {
    match whitespace {
        Some(whitespace) => vec![Token::new(TokenType::Whitespace {
            characters: ShortString::new(whitespace),
        })],
        None => Vec::new(),
    }
}

impl TokenRefExt for TokenReference {
    fn new_type(token_type: TokenType) -> Self {
        TokenReference::new(Vec::new(), Token::new(token_type), Vec::new())
    }
    fn new_identifier(identifier: &str) -> Self {
        TokenReference::new_type(TokenType::Identifier {
            identifier: ShortString::new(identifier),
        })
    }
    fn with_trivia(&self, leading_whitespace: Option<&str>, trailing_whitespace: Option<&str>) -> Self {
        TokenReference::new(trivia(leading_whitespace), self.token().clone(), trivia(trailing_whitespace))
    }
    fn identifier(&self) -> Option<&str> {
        match self.token_type() {
            TokenType::Identifier { identifier } => Some(identifier.as_str()),
            TokenType::StringLiteral {
                literal,
                multi_line_depth: _,
                quote_type: _,
            } => Some(literal.as_str()),
            _ => None,
        }
    }
}

pub trait AffixExt {
    /// Gets string from Suffix / Prefix if contains token with identifier or string
    fn identifier(&self) -> Option<&str>;
}

impl AffixExt for Suffix {
    fn identifier(&self) -> Option<&str> {
        if let Suffix::Index(Index::Dot { name, .. }) = self {
            name.identifier()
        } else {
            None
        }
    }
}

impl AffixExt for Prefix {
    fn identifier(&self) -> Option<&str> {
        if let Prefix::Name(name) = self {
            name.identifier()
        } else {
            None
        }
    }
}

pub trait HasAffixes {
    fn prefix(&self) -> &Prefix;
    fn suffixes(&self) -> impl Iterator<Item = &Suffix>;
    fn with_prefix(self, prefix: Prefix) -> Self;
    fn with_suffixes(self, suffixes: Vec<Suffix>) -> Self;
}

impl HasAffixes for VarExpression {
    fn prefix(&self) -> &Prefix {
        self.prefix()
    }
    fn suffixes(&self) -> impl Iterator<Item = &Suffix> {
        self.suffixes()
    }
    fn with_prefix(self, prefix: Prefix) -> Self {
        self.with_prefix(prefix)
    }
    fn with_suffixes(self, suffixes: Vec<Suffix>) -> Self {
        self.with_suffixes(suffixes)
    }
}

impl HasAffixes for FunctionCall {
    fn prefix(&self) -> &Prefix {
        self.prefix()
    }
    fn suffixes(&self) -> impl Iterator<Item = &Suffix> {
        self.suffixes()
    }
    fn with_prefix(self, prefix: Prefix) -> Self {
        self.with_prefix(prefix)
    }
    fn with_suffixes(self, suffixes: Vec<Suffix>) -> Self {
        self.with_suffixes(suffixes)
    }
}

/// Creates a new identifier by returning an expression
///
/// # Arguments
///
/// `token_ref` - optional, closest token to newline, can be passed to preserve trivia (newline, whitespace)
pub fn new_identifier_expression(identifier: &str, token_reference: Option<&TokenReference>) -> Expression {
    let token_type = TokenType::Identifier {
        identifier: ShortString::new(identifier),
    };
    let token_reference = match token_reference {
        Some(token_reference) => token_reference.with_token(Token::new(token_type)),
        None => TokenReference::new_type(token_type),
    };

    Expression::Symbol(token_reference)
}

pub fn new_local_assignment(name: &str, expression: Expression) -> LocalAssignment {
    let mut name_list = Punctuated::new();
    name_list.push(Pair::End(TokenReference::new_identifier(name)));

    let mut expression_list = Punctuated::new();
    expression_list.push(Pair::End(expression));

    LocalAssignment::new(name_list)
        .with_equal_token(Some(
            TokenReference::new_type(TokenType::Symbol { symbol: Symbol::Equal }).with_trivia(Some(" "), Some(" ")),
        ))
        .with_expressions(expression_list)
}

/// Creates a new local assignment that requires the main plugin source and gets its globals
///
/// Example: local _proxyGlobals = require(script.Parent.Parent).Globals
pub fn new_global_require(depth: usize) -> LocalAssignment {
    let require_func =
        FunctionCall::new(Prefix::Name(TokenReference::new_identifier("require"))).with_suffixes(vec![Suffix::Call(
            Call::AnonymousCall(FunctionArgs::Parentheses {
                parentheses: ContainedSpan::new(TokenReference::symbol("(").unwrap(), TokenReference::symbol(")").unwrap()),
                arguments: std::iter::once(Pair::End(new_identifier_expression(
                    &DotPath::new_ancestor_path(depth).to_string(),
                    None,
                )))
                .collect(),
            }),
        )]);

    new_local_assignment(
        GLOBAL_VAR_NAME,
        Expression::Var(Var::Expression(Box::new(
            VarExpression::new(Prefix::Expression(Box::new(Expression::FunctionCall(require_func)))).with_suffixes(vec![
                Suffix::Index(Index::Dot {
                    dot: TokenReference::new_type(TokenType::Symbol { symbol: Symbol::Dot }),
                    name: TokenReference::new_identifier("Globals").with_trivia(None, Some("\n")),
                }),
            ]),
        ))),
    )
}
