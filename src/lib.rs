use std::{
    borrow::Cow,
    fs::{self, File},
    io::{BufReader, BufWriter},
    path::Path,
    path::PathBuf,
};

use full_moon::{
    ast::*,
    node::Node,
    tokenizer::{Symbol, Token, TokenReference, TokenType},
    visitors::VisitorMut,
    ShortString,
};
use glob_match::glob_match;
use log::info;
use punctuated::Pair;
use punctuated::Punctuated;
use rbx_dom_weak::{
    types::{Ref, Variant},
    Instance, WeakDom,
};
use span::ContainedSpan;
use std::time::Instant;

mod trivia;
use trivia::{FormatTriviaType, UpdateTrailingTrivia};

pub mod dom;
use dom::extension::*;

pub mod error;
use error::Problem;

#[derive(Default)]
struct Requires {
    globals: bool,
    plugin: bool,
    enums: bool,
}

impl Requires {
    /// Use this to check if globals are required
    pub fn globals(&self) -> bool {
        self.globals || (self.plugin || self.enums)
    }
}

#[derive(Default)]
struct PluginProxyVisitor {
    requires: Requires,
}

fn is_replacable_enum<T: HasAffixes>(node: &T) -> bool {
    node.prefix().identifier().is_some_and(|p| p == "Enum")
        && node
            .suffixes()
            .next()
            .and_then(|s| s.identifier())
            .is_some_and(|i| matches!(i, "StudioStyleGuideColor" | "StudioStyleGuideModifier"))
}

fn is_settings_call<T: HasAffixes>(node: &T) -> bool {
    node.prefix().identifier().is_some_and(|p| p == "settings")
        && node.suffixes().next().map_or(false, |s| matches!(s, Suffix::Call(_)))
}

impl PluginProxyVisitor {
    pub fn process_common<T: HasAffixes + Node>(&mut self, node: T) -> T {
        match node {
            node if is_replacable_enum(&node) => {
                self.requires.enums = true;
                node.with_prefix(Prefix::Name(TokenReference::new_identifier("Enums")))
            }
            node if is_settings_call(&node) => {
                self.requires.globals = true;
                node.with_prefix(Prefix::Name(TokenReference::new_identifier(index_global!("settings"))))
            }
            _ => node,
        }
    }
}

impl VisitorMut for PluginProxyVisitor {
    fn visit_var_expression(&mut self, node: VarExpression) -> VarExpression {
        self.process_common(node)
    }

    fn visit_function_call(&mut self, node: FunctionCall) -> FunctionCall {
        self.process_common(node)
    }

    // Using visit_expression for functions so one can be replaced with just an identifier
    fn visit_expression(&mut self, node: Expression) -> Expression {
        // replace script:FindFirstAncestorOfClass('Plugin') with plugin global
        if let Expression::FunctionCall(function_call) = &node {
            for suf in function_call.suffixes() {
                // find a MethodCall in suffixes that searches for "Plugin"
                if let Suffix::Call(Call::MethodCall(method_call)) = suf {
                    if let Some(name) = method_call.name().identifier() {
                        // preserve trivia by grabbing last token from parentheses or string
                        let token_ref = {
                            match method_call.args() {
                                FunctionArgs::Parentheses { parentheses, .. } => parentheses.tokens().1,
                                FunctionArgs::String(token_ref) => token_ref,
                                _ => &TokenReference::new(Vec::new(), Token::new(TokenType::Eof), Vec::new()),
                            }
                        };

                        match name {
                            "FindFirstAncestorOfClass" | "FindFirstAncestorWhichIsA" => {
                                if nth_arg_string!(method_call.args(), 0).is_some_and(|a| matches!(a, "Plugin")) {
                                    self.requires.plugin = true;
                                    return new_identifier_expression("plugin", Some(token_ref));
                                }
                            }
                            "GetService" => {
                                self.requires.globals = true;

                                let suffixes = vec![
                                    Suffix::Index(Index::Dot {
                                        dot: TokenReference::new_type(TokenType::Symbol { symbol: Symbol::Dot }),
                                        name: TokenReference::new_identifier("game"),
                                    }),
                                    Suffix::Call(Call::MethodCall(method_call.clone())),
                                ];

                                return Expression::FunctionCall(
                                    FunctionCall::new(Prefix::Name(TokenReference::new_identifier(GLOBAL_VAR_NAME)))
                                        .with_suffixes(suffixes),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        node
    }
}

fn indent_string(s: String) -> String {
    let mut result = String::with_capacity(s.len() + s.lines().count());
    let mut is_first_line = true;
    for line in s.lines() {
        if !is_first_line {
            result.push('\n');
        }
        if !line.is_empty() {
            result.push('\t');
        }
        result.push_str(line);
        is_first_line = false;
    }
    result
}

/// Wraps main plugin source with: return { init = function(_proxyGlobals) ... end }
fn wrap_main_source(ast: Ast) -> String {
    let code_block = indent_string(full_moon::print(&ast));

    let init_func = FunctionBody::new()
        .with_parameters(std::iter::once(Pair::End(Parameter::Name(TokenReference::new_identifier(GLOBAL_VAR_NAME)))).collect())
        .with_parameters_parentheses(ContainedSpan::new(
            TokenReference::symbol("(").unwrap(),
            TokenReference::symbol(")\n").unwrap(),
        ))
        .with_block(Block::new().with_last_stmt(Some((
            LastStmt::Return(Return::new().with_token(TokenReference::new_type(TokenType::Identifier {
                identifier: ShortString::new(code_block),
            }))),
            None,
        ))));

    let mut returns = Punctuated::new();
    returns.push(Pair::End(Expression::TableConstructor(
        TableConstructor::new().with_fields(
            std::iter::once(Pair::End(Field::NameKey {
                key: TokenReference::new_identifier("init"),
                equal: TokenReference::new_type(TokenType::Symbol { symbol: Symbol::Equal }).with_trivia(Some(" "), Some(" ")),
                value: Expression::Function(Box::new((
                    TokenReference::new_type(TokenType::Symbol {
                        symbol: Symbol::Function,
                    }),
                    init_func,
                ))),
            }))
            .collect(),
        ),
    )));

    full_moon::print(
        &ast.with_nodes(Block::new().with_last_stmt(Some((LastStmt::Return(Return::new().with_returns(returns)), None)))),
    )
}

pub struct DomTranspiler {
    tree: WeakDom,
    source_script: Ref,
    exclude_libs: bool,
}

impl DomTranspiler {
    pub fn new(tree: WeakDom) -> Result<Self, Problem> {
        let source_script = tree
            .find_first_child_class(
                tree.root(),
                |class| matches!(class, "ModuleScript" | "Script" | "LocalScript"),
                2,
            )
            .ok_or(Problem::NoMainSource)?;

        Ok(Self {
            tree,
            source_script,
            exclude_libs: true,
        })
    }

    /// Controls the exclusion of standard libraries that typically don't need plugin access.
    ///
    /// * **Default: true** (libraries are excluded)
    /// * Set to `false` to include all libraries, even standard ones.
    ///
    /// Use `false` if your plugin relies on a module that shares a name with
    /// a standard library (like React or Fusion) and needs plugin-specific methods.
    ///
    /// # Affected libraries
    ///
    /// (also any descendants of the path to these libraries)
    /// * React/Roact, jsdotlua descendants
    /// * Fusion
    ///
    /// # Returns
    /// `&mut Self` for method chaining
    pub fn exclude_libs(&mut self, exclude_libs: bool) -> &mut Self {
        self.exclude_libs = exclude_libs;
        self
    }

    /// Check if path could be a library that does not require plugin access
    fn is_excluded(&self, p: &str) -> bool {
        self.exclude_libs
            && (glob_match("**/[Rr][eo]act*/**", p) || glob_match("**/*jsdotlua*/**", p) || glob_match("**/Fusion/**", p))
    }

    /// Saves the edited dom to a file path
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to save the dom to.<br>
    /// Extension must be: <br>
    /// `.rbxm`, `.rbxl` (**binary**) or <br>
    /// `.rbxmx`, `.rbxlx` (**xml**)
    pub fn save_to_file(&self, file_path: &Path) -> Result<(), Problem> {
        let extension = RbxFileType::from_path(file_path)?;
        let output = BufWriter::new(File::create(file_path).map_err(|error| Problem::IOError("create the output file", error))?);

        match extension {
            RbxFileType::XML => {
                rbx_xml::to_writer_default(output, &self.tree, &[self.source_script]).map_err(Problem::XMLEncodeError)
            }
            RbxFileType::Binary => {
                rbx_binary::to_writer(output, &self.tree, &[self.source_script]).map_err(Problem::BinaryEncodeError)
            }
        }?;
        Ok(())
    }

    /// Transpiles the entire dom tree, which can then be saved to a file
    ///
    /// # Returns
    /// `Result<&mut Self, Problem>` for method chaining and error handling
    pub fn transpile_tree(&mut self) -> Result<&mut Self, Problem> {
        let now = Instant::now();

        let mut script_stack = Vec::new();
        let mut total_count: usize = 0;

        self.tree.foreach_descendant(
            self.tree.get_by_ref(self.source_script).unwrap(),
            &mut |child, path| {
                if child.class == "ModuleScript" {
                    total_count += 1;
                    if !self.is_excluded(&path.path_string()) {
                        script_stack.push((child.referent(), path.depth()));
                    }
                }
                ForEachAction::Continue
            },
            0,
        );

        info!("Script total: {}, time: {:.2?}", total_count, now.elapsed());
        info!("Skipped {} scripts", total_count.abs_diff(script_stack.len()));

        for (referent, depth) in script_stack {
            let script = self.tree.get_by_ref_mut(referent).unwrap();
            Self::process_script(script, depth)?;
        }

        let script = self.tree.get_by_ref_mut(self.source_script).unwrap();
        Self::process_script(script, 0)?;

        info!("Transpiled in {:.2?}", now.elapsed());

        Ok(self)
    }

    fn process_script(script: &mut Instance, depth: usize) -> Result<(), Problem> {
        let source = script.properties.get_mut("Source");
        if let Some(Variant::String(source_string)) = source {
            if depth == 0 {
                script.class = String::from("ModuleScript");
                *source_string = wrap_main_source(Self::transpile_source(source_string, depth)?);
            } else {
                *source_string = full_moon::print(&Self::transpile_source(source_string, depth)?);
            };

            return Ok(());
        }
        Err(Problem::NoScriptSource(script.name.clone()))
    }

    /// Transpiles a string containing the source code
    ///
    /// # Arguments
    ///
    /// `source` - The source code for a module/script
    /// `path_depth` - The depth of the script in the dom tree, used for requiring the plugin globals
    pub fn transpile_source(source: &str, path_depth: usize) -> Result<Ast, Problem> {
        let mut visitor = PluginProxyVisitor::default();
        let mut ast = visitor.visit_ast(full_moon::parse(source).map_err(Problem::TranspilerError)?);

        let mut requires: Vec<(Stmt, Option<TokenReference>)> = Vec::with_capacity(3);

        if visitor.requires.globals() && path_depth > 0 {
            requires.push((Stmt::LocalAssignment(new_global_require(path_depth)), None));
        }
        if visitor.requires.plugin || path_depth == 0 {
            requires.push((
                Stmt::LocalAssignment(new_local_assignment(
                    "plugin",
                    Expression::Symbol(TokenReference::new_identifier(index_global!("plugin")).with_trivia(None, Some("\n"))),
                )),
                None,
            ))
        }
        if visitor.requires.enums {
            requires.push((
                Stmt::LocalAssignment(new_local_assignment(
                    "Enums",
                    Expression::Symbol(TokenReference::new_identifier(index_global!("Enums")).with_trivia(None, Some("\n"))),
                )),
                None,
            ))
        }

        if let Some(last_req) = requires.last_mut() {
            *last_req = (
                last_req
                    .0
                    .update_trailing_trivia(FormatTriviaType::Append(vec![Token::new(TokenType::SingleLineComment {
                        comment: ShortString::new(" Autogenerated with PluginProxy Transpiler\n\n"),
                    })])),
                None,
            );

            requires.extend(ast.nodes().stmts_with_semicolon().cloned());

            *ast.nodes_mut() = Block::new()
                .with_stmts(requires)
                .with_last_stmt(ast.nodes().last_stmt_with_semicolon().cloned());
        }

        Ok(ast)
    }
}

pub enum RbxFileType {
    XML,
    Binary,
}
impl RbxFileType {
    pub fn from_path(file_path: &Path) -> Result<RbxFileType, Problem> {
        match file_path.extension().map(|extension| extension.to_string_lossy()) {
            Some(Cow::Borrowed("rbxmx")) | Some(Cow::Borrowed("rbxlx")) => Ok(RbxFileType::XML),
            Some(Cow::Borrowed("rbxm")) | Some(Cow::Borrowed("rbxl")) => Ok(RbxFileType::Binary),
            _ => Err(Problem::InvalidExtension(file_path.to_path_buf())),
        }
    }
}

pub fn from_dom(tree: WeakDom) -> Result<DomTranspiler, Problem> {
    DomTranspiler::new(tree)
}

pub fn from_file(file_path: &PathBuf) -> Result<DomTranspiler, Problem> {
    let file_name = file_path.file_name().unwrap_or_default().to_string_lossy();
    let file_source = BufReader::new(fs::File::open(file_path).map_err(|error| Problem::IOError("read the place file", error))?);

    info!("Decoding {file_name}...");
    let tree = match RbxFileType::from_path(file_path)? {
        RbxFileType::XML => rbx_xml::from_reader_default(file_source).map_err(Problem::XMLDecodeError),
        RbxFileType::Binary => rbx_binary::from_reader(file_source).map_err(Problem::BinaryDecodeError),
    }?;

    DomTranspiler::new(tree)
}
