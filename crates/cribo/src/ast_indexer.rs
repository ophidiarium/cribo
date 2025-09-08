//! AST indexing module for assigning stable node indices to AST nodes.
//!
//! This module provides functionality to traverse an AST and assign sequential
//! indices to all nodes. These indices enable:
//! - Efficient node lookup by index
//! - Stable references across transformations
//! - Foundation for source map generation
//! - Memory-efficient AST management

use std::cell::RefCell;

use ruff_python_ast::{
    Alias, Arguments, AtomicNodeIndex, Comprehension, Decorator, ExceptHandler, Expr, Keyword,
    MatchCase, ModModule, NodeIndex, Parameter, Parameters, Pattern, Stmt, TypeParam, WithItem,
    visitor::transformer::{
        Transformer, walk_alias, walk_arguments, walk_body, walk_comprehension, walk_decorator,
        walk_except_handler, walk_expr, walk_keyword, walk_match_case, walk_parameter,
        walk_parameters, walk_pattern, walk_stmt, walk_type_param, walk_with_item,
    },
};

/// Number of indices reserved per module (1 million)
pub const MODULE_INDEX_RANGE: u32 = 1_000_000;

/// Result of indexing an AST module
#[derive(Debug)]
pub struct IndexedAst {
    /// The total number of nodes indexed
    pub node_count: u32,
}

/// Visitor that assigns indices to all AST nodes
struct IndexingVisitor {
    /// Current index to assign (using `RefCell` for interior mutability)
    current_index: RefCell<u32>,
    /// Base index for this module (e.g., 0, `1_000_000`, `2_000_000`)
    base_index: u32,
}

impl IndexingVisitor {
    fn new(base_index: u32) -> Self {
        Self {
            current_index: RefCell::new(base_index),
            base_index,
        }
    }

    /// Assign an index to a node
    fn assign_index(&self, node_index: &AtomicNodeIndex) -> NodeIndex {
        let mut current = self.current_index.borrow_mut();

        // Check for overflow within module range
        let relative_index = *current - self.base_index;
        assert!(
            relative_index < MODULE_INDEX_RANGE,
            "Module index overflow: attempted to assign index {} (relative: {}) which exceeds \
             MODULE_INDEX_RANGE ({})",
            *current,
            relative_index,
            MODULE_INDEX_RANGE
        );

        node_index.set(*current);
        let index = AtomicNodeIndex::from(*current).load();
        *current += 1;
        index
    }
}

impl Transformer for IndexingVisitor {
    fn visit_body(&self, body: &mut [Stmt]) {
        walk_body(self, body);
    }

    fn visit_stmt(&self, stmt: &mut Stmt) {
        match stmt {
            Stmt::FunctionDef(func) => self.assign_index(&func.node_index),
            Stmt::ClassDef(class) => self.assign_index(&class.node_index),
            Stmt::Import(import) => self.assign_index(&import.node_index),
            Stmt::ImportFrom(import) => self.assign_index(&import.node_index),
            Stmt::Assign(assign) => self.assign_index(&assign.node_index),
            // Assign indices to all other statement types
            Stmt::Return(s) => self.assign_index(&s.node_index),
            Stmt::Delete(s) => self.assign_index(&s.node_index),
            Stmt::AugAssign(s) => self.assign_index(&s.node_index),
            Stmt::AnnAssign(s) => self.assign_index(&s.node_index),
            Stmt::TypeAlias(s) => self.assign_index(&s.node_index),
            Stmt::For(s) => self.assign_index(&s.node_index),
            Stmt::While(s) => self.assign_index(&s.node_index),
            Stmt::If(s) => self.assign_index(&s.node_index),
            Stmt::With(s) => self.assign_index(&s.node_index),
            Stmt::Match(s) => self.assign_index(&s.node_index),
            Stmt::Raise(s) => self.assign_index(&s.node_index),
            Stmt::Try(s) => self.assign_index(&s.node_index),
            Stmt::Assert(s) => self.assign_index(&s.node_index),
            Stmt::Global(s) => self.assign_index(&s.node_index),
            Stmt::Nonlocal(s) => self.assign_index(&s.node_index),
            Stmt::Expr(s) => self.assign_index(&s.node_index),
            Stmt::Pass(s) => self.assign_index(&s.node_index),
            Stmt::Break(s) => self.assign_index(&s.node_index),
            Stmt::Continue(s) => self.assign_index(&s.node_index),
            Stmt::IpyEscapeCommand(s) => self.assign_index(&s.node_index),
        };

        walk_stmt(self, stmt);
    }

    fn visit_expr(&self, expr: &mut Expr) {
        match expr {
            Expr::BoolOp(e) => self.assign_index(&e.node_index),
            Expr::BinOp(e) => self.assign_index(&e.node_index),
            Expr::UnaryOp(e) => self.assign_index(&e.node_index),
            Expr::Lambda(e) => self.assign_index(&e.node_index),
            Expr::If(e) => self.assign_index(&e.node_index),
            Expr::Dict(e) => self.assign_index(&e.node_index),
            Expr::Set(e) => self.assign_index(&e.node_index),
            Expr::ListComp(e) => self.assign_index(&e.node_index),
            Expr::SetComp(e) => self.assign_index(&e.node_index),
            Expr::DictComp(e) => self.assign_index(&e.node_index),
            Expr::Generator(e) => self.assign_index(&e.node_index),
            Expr::Await(e) => self.assign_index(&e.node_index),
            Expr::Yield(e) => self.assign_index(&e.node_index),
            Expr::YieldFrom(e) => self.assign_index(&e.node_index),
            Expr::Compare(e) => self.assign_index(&e.node_index),
            Expr::Call(e) => self.assign_index(&e.node_index),
            Expr::NumberLiteral(e) => self.assign_index(&e.node_index),
            Expr::StringLiteral(e) => self.assign_index(&e.node_index),
            Expr::FString(e) => self.assign_index(&e.node_index),
            Expr::BytesLiteral(e) => self.assign_index(&e.node_index),
            Expr::BooleanLiteral(e) => self.assign_index(&e.node_index),
            Expr::NoneLiteral(e) => self.assign_index(&e.node_index),
            Expr::EllipsisLiteral(e) => self.assign_index(&e.node_index),
            Expr::Attribute(e) => self.assign_index(&e.node_index),
            Expr::Subscript(e) => self.assign_index(&e.node_index),
            Expr::Starred(e) => self.assign_index(&e.node_index),
            Expr::Name(e) => self.assign_index(&e.node_index),
            Expr::List(e) => self.assign_index(&e.node_index),
            Expr::Tuple(e) => self.assign_index(&e.node_index),
            Expr::Slice(e) => self.assign_index(&e.node_index),
            Expr::IpyEscapeCommand(e) => self.assign_index(&e.node_index),
            Expr::Named(e) => self.assign_index(&e.node_index),
            Expr::TString(e) => self.assign_index(&e.node_index),
        };

        walk_expr(self, expr);
    }

    fn visit_decorator(&self, decorator: &mut Decorator) {
        self.assign_index(&decorator.node_index);
        walk_decorator(self, decorator);
    }

    fn visit_comprehension(&self, comprehension: &mut Comprehension) {
        self.assign_index(&comprehension.node_index);
        walk_comprehension(self, comprehension);
    }

    fn visit_except_handler(&self, handler: &mut ExceptHandler) {
        match handler {
            ExceptHandler::ExceptHandler(h) => self.assign_index(&h.node_index),
        };
        walk_except_handler(self, handler);
    }

    fn visit_arguments(&self, arguments: &mut Arguments) {
        self.assign_index(&arguments.node_index);
        walk_arguments(self, arguments);
    }

    fn visit_parameters(&self, parameters: &mut Parameters) {
        self.assign_index(&parameters.node_index);

        // Handle ParameterWithDefault nodes before walking
        for arg in &mut parameters.posonlyargs {
            self.assign_index(&arg.node_index);
        }
        for arg in &mut parameters.args {
            self.assign_index(&arg.node_index);
        }
        for arg in &mut parameters.kwonlyargs {
            self.assign_index(&arg.node_index);
        }

        walk_parameters(self, parameters);
    }

    fn visit_parameter(&self, parameter: &mut Parameter) {
        self.assign_index(&parameter.node_index);
        walk_parameter(self, parameter);
    }

    // Note: ParameterWithDefault is handled within Parameters traversal

    fn visit_keyword(&self, keyword: &mut Keyword) {
        self.assign_index(&keyword.node_index);
        walk_keyword(self, keyword);
    }

    fn visit_alias(&self, alias: &mut Alias) {
        self.assign_index(&alias.node_index);
        walk_alias(self, alias);
    }

    fn visit_with_item(&self, with_item: &mut WithItem) {
        self.assign_index(&with_item.node_index);
        walk_with_item(self, with_item);
    }

    fn visit_match_case(&self, match_case: &mut MatchCase) {
        self.assign_index(&match_case.node_index);
        walk_match_case(self, match_case);
    }

    fn visit_pattern(&self, pattern: &mut Pattern) {
        match pattern {
            Pattern::MatchValue(p) => self.assign_index(&p.node_index),
            Pattern::MatchSingleton(p) => self.assign_index(&p.node_index),
            Pattern::MatchSequence(p) => self.assign_index(&p.node_index),
            Pattern::MatchMapping(p) => self.assign_index(&p.node_index),
            Pattern::MatchClass(p) => self.assign_index(&p.node_index),
            Pattern::MatchStar(p) => self.assign_index(&p.node_index),
            Pattern::MatchAs(p) => self.assign_index(&p.node_index),
            Pattern::MatchOr(p) => self.assign_index(&p.node_index),
        };
        walk_pattern(self, pattern);
    }

    fn visit_type_param(&self, type_param: &mut TypeParam) {
        match type_param {
            TypeParam::TypeVar(t) => self.assign_index(&t.node_index),
            TypeParam::ParamSpec(t) => self.assign_index(&t.node_index),
            TypeParam::TypeVarTuple(t) => self.assign_index(&t.node_index),
        };
        walk_type_param(self, type_param);
    }
}

/// Index all nodes in a module AST with a specific module ID
pub fn index_module_with_id(module: &mut ModModule, module_id: u32) -> IndexedAst {
    let base_index = module_id * MODULE_INDEX_RANGE;
    let visitor = IndexingVisitor::new(base_index);

    // Assign index to the module itself
    visitor.assign_index(&module.node_index);

    // Visit the body statements
    visitor.visit_body(&mut module.body);

    let current_index = *visitor.current_index.borrow();
    IndexedAst {
        node_count: current_index - visitor.base_index,
    }
}
