use rustpython_ast::{
    Alias, Arg, Arguments, BoolOp, CmpOp, Comprehension, ExceptHandler, ExceptHandlerExceptHandler,
    Expr, ExprAttribute, ExprAwait, ExprBinOp, ExprBoolOp, ExprCall, ExprCompare, ExprConstant,
    ExprDict, ExprDictComp, ExprFormattedValue, ExprGeneratorExp, ExprIfExp, ExprJoinedStr,
    ExprLambda, ExprList, ExprListComp, ExprName, ExprNamedExpr, ExprSet, ExprSetComp, ExprSlice,
    ExprStarred, ExprSubscript, ExprTuple, ExprUnaryOp, ExprYield, ExprYieldFrom, Keyword,
    MatchCase, Operator, Pattern, PatternMatchAs, PatternMatchClass, PatternMatchMapping,
    PatternMatchOr, PatternMatchSequence, PatternMatchSingleton, PatternMatchStar,
    PatternMatchValue, Stmt, StmtAnnAssign, StmtAssert, StmtAssign, StmtAsyncFor,
    StmtAsyncFunctionDef, StmtAsyncWith, StmtAugAssign, StmtBreak, StmtClassDef, StmtContinue,
    StmtDelete, StmtExpr, StmtFor, StmtFunctionDef, StmtGlobal, StmtIf, StmtImport, StmtImportFrom,
    StmtMatch, StmtNonlocal, StmtPass, StmtRaise, StmtReturn, StmtTry, StmtTryStar, StmtTypeAlias,
    StmtWhile, StmtWith, TypeParam, TypeParamParamSpec, TypeParamTypeVar, TypeParamTypeVarTuple,
    UnaryOp, WithItem, text_size::TextRange,
};
use rustpython_ast::{Constant, ConversionFlag, Int};
use std::ops::Deref;

use crate::utils::replace_first_and_last;

enum Precedence {
    NamedExpr = 1,
    Tuple = 2,
    Yield = 3,
    Test = 4,
    Or = 5,
    And = 6,
    Not = 7,
    Cmp = 8,

    Bor = 9,
    Bxor = 10,
    Band = 11,
    Shift = 12,
    Arith = 13,
    Term = 14,
    Factor = 15,
    Power = 16,
    Await = 17,
    Atom = 18,
}

impl Precedence {
    fn value(self) -> usize {
        self as usize
    }
}

const EXPR_PRECEDENCE: usize = 9;

fn get_precedence(node: &Expr<TextRange>) -> usize {
    match node {
        Expr::NamedExpr(_) => Precedence::NamedExpr.value(),
        Expr::Tuple(_) => Precedence::Tuple.value(),
        Expr::Yield(_) => Precedence::Yield.value(),
        Expr::YieldFrom(_) => Precedence::Yield.value(),
        Expr::IfExp(_) => Precedence::Test.value(),
        Expr::Lambda(_) => Precedence::Test.value(),
        Expr::BoolOp(data) => match data.op {
            BoolOp::Or => Precedence::Or.value(),
            BoolOp::And => Precedence::And.value(),
        },
        Expr::UnaryOp(data) => match data.op {
            UnaryOp::Not => Precedence::Not.value(),
            UnaryOp::UAdd => Precedence::Factor.value(),
            UnaryOp::USub => Precedence::Factor.value(),
            UnaryOp::Invert => Precedence::Factor.value(),
        },
        Expr::Compare(_) => Precedence::Cmp.value(),
        Expr::BinOp(data) => match data.op {
            Operator::BitOr => Precedence::Bor.value(),
            Operator::BitXor => Precedence::Bxor.value(),
            Operator::BitAnd => Precedence::Band.value(),
            Operator::LShift => Precedence::Shift.value(),
            Operator::RShift => Precedence::Shift.value(),
            Operator::Add => Precedence::Arith.value(),
            Operator::Sub => Precedence::Arith.value(),
            Operator::Div => Precedence::Term.value(),
            Operator::FloorDiv => Precedence::Term.value(),
            Operator::Mult => Precedence::Term.value(),
            Operator::MatMult => Precedence::Term.value(),
            Operator::Mod => Precedence::Term.value(),
            Operator::Pow => Precedence::Power.value(),
        },
        Expr::Await(_) => Precedence::Await.value(),
        _ => Precedence::Test.value(),
    }
}

pub struct Unparser {
    pub source: String,
    indent: usize,
    in_try_star: bool,
    precedence_level: usize,
}

impl Default for Unparser {
    fn default() -> Self {
        Self::new()
    }
}

impl Unparser {
    pub fn new() -> Self {
        Unparser {
            in_try_star: false,
            indent: 0,
            precedence_level: Precedence::Test.value(),
            source: String::new(),
        }
    }

    fn fill(&mut self, str_: &str) {
        if !self.source.is_empty() {
            self.write_str(&("\n".to_owned() + &" ".repeat(self.indent * 4) + str_))
        } else {
            self.write_str(str_);
        }
    }

    fn write_str(&mut self, str_: &str) {
        self.source += str_
    }

    fn write_type_comment(&mut self, type_comment: &Option<String>) {
        if let Some(str_) = type_comment {
            self.write_str("  # type: ignore");
            self.write_str(str_);
        }
    }

    fn block<F>(&mut self, f: F)
    where
        F: FnOnce(&mut Self),
    {
        self.indent += 1;
        f(self);
        self.indent -= 1;
    }

    fn delimit_precedence<F>(&mut self, node: &Expr<TextRange>, f: F)
    where
        F: FnOnce(&mut Self),
    {
        let should_delimit = self.precedence_level > get_precedence(node);
        if should_delimit {
            self.write_str("(");
        }
        f(self);
        if should_delimit {
            self.write_str(")");
        }
    }

    fn with_precedence<F>(&mut self, prec: Precedence, f: F)
    where
        F: FnOnce(&mut Self),
    {
        let prev_prec = self.precedence_level;
        self.precedence_level = prec.value();
        f(self);
        self.precedence_level = prev_prec;
    }

    fn with_precedence_num<F>(&mut self, prec: usize, f: F)
    where
        F: FnOnce(&mut Self),
    {
        let prev_prec = self.precedence_level;
        self.precedence_level = prec;
        f(self);
        self.precedence_level = prev_prec;
    }

    pub fn unparse_stmt(&mut self, node: &Stmt<TextRange>) {
        match node {
            Stmt::FunctionDef(data) => self.unparse_stmt_function_def(data),
            Stmt::AsyncFunctionDef(data) => self.unparse_stmt_async_function_def(data),
            Stmt::ClassDef(data) => self.unparse_stmt_class_def(data),
            Stmt::Return(data) => self.unparse_stmt_return(data),
            Stmt::Delete(data) => self.unparse_stmt_delete(data),
            Stmt::Assign(data) => self.unparse_stmt_assign(data),
            Stmt::TypeAlias(data) => self.unparse_stmt_type_alias(data),
            Stmt::AugAssign(data) => self.unparse_stmt_aug_assign(data),
            Stmt::AnnAssign(data) => self.unparse_stmt_ann_assign(data),
            Stmt::For(data) => self.unparse_stmt_for(data),
            Stmt::AsyncFor(data) => self.unparse_stmt_async_for(data),
            Stmt::While(data) => self.unparse_stmt_while(data),
            Stmt::If(data) => self.unparse_stmt_if(data, false),
            Stmt::With(data) => self.unparse_stmt_with(data),
            Stmt::AsyncWith(data) => self.unparse_stmt_async_with(data),
            Stmt::Match(data) => self.unparse_stmt_match(data),
            Stmt::Raise(data) => self.unparse_stmt_raise(data),
            Stmt::Try(data) => self.unparse_stmt_try(data),
            Stmt::TryStar(data) => self.unparse_stmt_try_star(data),
            Stmt::Assert(data) => self.unparse_stmt_assert(data),
            Stmt::Import(data) => self.unparse_stmt_import(data),
            Stmt::ImportFrom(data) => self.unparse_stmt_import_from(data),
            Stmt::Global(data) => self.unparse_stmt_global(data),
            Stmt::Nonlocal(data) => self.unparse_stmt_nonlocal(data),
            Stmt::Expr(data) => self.unparse_stmt_expr(data),
            Stmt::Pass(data) => self.unparse_stmt_pass(data),
            Stmt::Break(data) => self.unparse_stmt_break(data),
            Stmt::Continue(data) => self.unparse_stmt_continue(data),
        }
    }

    fn unparse_stmt_pass(&mut self, _node: &StmtPass<TextRange>) {
        self.fill("pass")
    }

    fn unparse_stmt_break(&mut self, _node: &StmtBreak<TextRange>) {
        self.fill("break")
    }

    fn unparse_stmt_continue(&mut self, _node: &StmtContinue<TextRange>) {
        self.fill("continue")
    }

    fn unparse_stmt_function_def(&mut self, node: &StmtFunctionDef<TextRange>) {
        for decorator in &node.decorator_list {
            self.fill("@");
            self.unparse_expr(decorator);
        }
        self.fill("def ");
        self.write_str(&node.name);

        if !node.type_params.is_empty() {
            self.write_str("[");
            let mut type_params_iter = node.type_params.iter().peekable();
            while let Some(type_param) = type_params_iter.next() {
                self.unparse_type_param(type_param);
                if type_params_iter.peek().is_some() {
                    self.write_str(", ");
                }
            }
            self.write_str("]");
        }
        self.write_str("(");

        self.unparse_arguments(&node.args);

        self.write_str(")");
        if let Some(returns) = &node.returns {
            self.write_str(" -> ");
            self.unparse_expr(returns);
        }
        self.write_str(":");
        self.write_type_comment(&node.type_comment);
        self.block(|block_self| {
            for value in &node.body {
                block_self.unparse_stmt(value);
            }
        });
    }

    fn unparse_stmt_async_function_def(&mut self, node: &StmtAsyncFunctionDef<TextRange>) {
        for decorator in &node.decorator_list {
            self.fill("@");
            self.unparse_expr(decorator);
        }
        self.fill("async def ");
        self.write_str(&node.name);
        if !node.type_params.is_empty() {
            self.write_str("[");
            let mut type_params_iter = node.type_params.iter().peekable();
            while let Some(type_param) = type_params_iter.next() {
                self.unparse_type_param(type_param);
                if type_params_iter.peek().is_some() {
                    self.write_str(", ");
                }
            }
            self.write_str("]");
        }
        self.write_str("(");

        self.unparse_arguments(&node.args);

        self.write_str(")");
        if let Some(returns) = &node.returns {
            self.write_str(" -> ");
            self.unparse_expr(returns);
        }
        self.write_str(":");
        self.write_type_comment(&node.type_comment);
        self.block(|block_self| {
            for value in &node.body {
                block_self.unparse_stmt(value);
            }
        });
    }

    fn unparse_stmt_class_def(&mut self, node: &StmtClassDef<TextRange>) {
        for decorator in &node.decorator_list {
            self.fill("@");
            self.unparse_expr(decorator);
        }

        self.fill("class ");
        self.write_str(&node.name);

        if !node.type_params.is_empty() {
            self.write_str("[");
            let mut type_params_iter = node.type_params.iter().peekable();
            while let Some(type_param) = type_params_iter.next() {
                self.unparse_type_param(type_param);
                if type_params_iter.peek().is_some() {
                    self.write_str(", ");
                }
            }
            self.write_str("]");
        }

        let mut bases_iter = node.bases.iter().peekable();
        let mut keywords_iter = node.keywords.iter().peekable();
        let has_parens = bases_iter.peek().is_some() || keywords_iter.peek().is_some();
        if has_parens {
            self.write_str("(");
        }

        while let Some(base) = bases_iter.next() {
            self.unparse_expr(base);
            if bases_iter.peek().is_some() || keywords_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
        while let Some(keyword) = keywords_iter.next() {
            self.unparse_keyword(keyword);
            if keywords_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
        if has_parens {
            self.write_str(")");
        }
        self.write_str(":");

        self.block(|block_self| {
            for value in &node.body {
                block_self.unparse_stmt(value);
            }
        });
    }

    fn unparse_stmt_return(&mut self, node: &StmtReturn<TextRange>) {
        self.fill("return ");
        if let Some(value) = &node.value {
            self.unparse_expr(value);
        }
    }
    fn unparse_stmt_delete(&mut self, node: &StmtDelete<TextRange>) {
        self.fill("del ");
        let mut targets_iter = node.targets.iter().peekable();

        while let Some(target) = targets_iter.next() {
            self.unparse_expr(target);
            if targets_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
    }

    fn unparse_stmt_assign(&mut self, node: &StmtAssign<TextRange>) {
        let mut targets_iter = node.targets.iter().peekable();
        self.fill("");
        while let Some(target) = targets_iter.next() {
            self.with_precedence(Precedence::Tuple, |prec_self| {
                prec_self.unparse_expr(target);
            });

            if targets_iter.peek().is_some() {
                self.write_str(" = ");
            }
        }
        self.write_str(" = ");
        self.unparse_expr(&node.value);
        self.write_type_comment(&node.type_comment);
    }

    fn unparse_stmt_type_alias(&mut self, node: &StmtTypeAlias<TextRange>) {
        self.fill("type ");
        self.unparse_expr(&node.name);
        if !node.type_params.is_empty() {
            self.write_str("[");
            let mut type_params_iter = node.type_params.iter().peekable();
            while let Some(type_param) = type_params_iter.next() {
                self.unparse_type_param(type_param);
                if type_params_iter.peek().is_some() {
                    self.write_str(", ");
                }
            }
            self.write_str("]");
        }
        self.write_str(" = ");
        self.unparse_expr(&node.value);
    }

    fn unparse_stmt_aug_assign(&mut self, node: &StmtAugAssign<TextRange>) {
        self.fill("");
        self.unparse_expr(&node.target);
        self.write_str(" ");
        self.unparse_operator(&node.op);
        self.write_str("= ");
        self.unparse_expr(&node.value);
    }

    fn unparse_stmt_ann_assign(&mut self, node: &StmtAnnAssign<TextRange>) {
        self.fill("");
        self.unparse_expr(&node.target);
        self.write_str(": ");
        self.unparse_expr(&node.annotation);
        if let Some(value) = &node.value {
            self.write_str(" = ");
            self.unparse_expr(value);
        }
    }

    fn unparse_stmt_for(&mut self, node: &StmtFor<TextRange>) {
        self.fill("for ");
        self.unparse_expr(&node.target);
        self.write_str(" in ");
        self.unparse_expr(&node.iter);
        self.write_str(":");
        self.write_type_comment(&node.type_comment);
        self.block(|block_self| {
            for value in &node.body {
                block_self.unparse_stmt(value);
            }
        });
        if !node.orelse.is_empty() {
            self.fill("else:");
            self.block(|block_self| {
                for stmt in &node.orelse {
                    block_self.unparse_stmt(stmt);
                }
            });
        }
    }
    fn unparse_stmt_async_for(&mut self, node: &StmtAsyncFor<TextRange>) {
        self.fill("async for ");
        self.unparse_expr(&node.target);
        self.write_str(" in ");
        self.unparse_expr(&node.iter);
        self.write_str(":");
        self.write_type_comment(&node.type_comment);
        self.block(|block_self| {
            for value in &node.body {
                block_self.unparse_stmt(value);
            }
        });
        if !node.orelse.is_empty() {
            self.fill("else:");
            self.block(|block_self| {
                for stmt in &node.orelse {
                    block_self.unparse_stmt(stmt);
                }
            });
        }
    }
    fn unparse_stmt_while(&mut self, node: &StmtWhile<TextRange>) {
        self.fill("while ");
        self.unparse_expr(&node.test);
        self.write_str(":");
        self.block(|block_self| {
            for stmt in &node.body {
                block_self.unparse_stmt(stmt);
            }
        });

        if !node.orelse.is_empty() {
            self.fill("else:");
            self.block(|block_self| {
                for stmt in &node.orelse {
                    block_self.unparse_stmt(stmt);
                }
            });
        }
    }

    fn unparse_stmt_if(&mut self, node: &StmtIf<TextRange>, inner_if: bool) {
        if inner_if {
            self.fill("elif ");
        } else {
            self.fill("if ");
        }

        self.unparse_expr(&node.test);
        self.write_str(":");
        self.block(|block_self| {
            for stmt in &node.body {
                block_self.unparse_stmt(stmt);
            }
        });
        match node.orelse.as_slice() {
            [Stmt::If(inner_if)] => {
                self.unparse_stmt_if(inner_if, true);
            }
            [] => {}
            _ => {
                self.fill("else:");
                self.block(|block_self| {
                    for stmt in &node.orelse {
                        block_self.unparse_stmt(stmt);
                    }
                });
            }
        }
    }

    fn unparse_stmt_with(&mut self, node: &StmtWith<TextRange>) {
        self.fill("with ");
        let mut items_iter = node.items.iter().peekable();
        while let Some(item) = items_iter.next() {
            self.unparse_withitem(item);
            if items_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
        self.write_str(":");
        self.block(|block_self| {
            for stmt in &node.body {
                block_self.unparse_stmt(stmt);
            }
        });
    }
    fn unparse_stmt_async_with(&mut self, node: &StmtAsyncWith<TextRange>) {
        self.fill("async with ");
        let mut items_iter = node.items.iter().peekable();
        while let Some(item) = items_iter.next() {
            self.unparse_withitem(item);
            if items_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
        self.write_str(":");
        self.block(|block_self| {
            for stmt in &node.body {
                block_self.unparse_stmt(stmt);
            }
        });
    }

    fn unparse_stmt_match(&mut self, node: &StmtMatch<TextRange>) {
        self.fill("match ");
        self.unparse_expr(&node.subject);
        self.write_str(":");
        self.block(|block_self| {
            for case in &node.cases {
                block_self.unparse_match_case(case);
            }
        });
    }

    fn unparse_stmt_raise(&mut self, node: &StmtRaise<TextRange>) {
        self.fill("raise ");
        if let Some(exc) = &node.exc {
            self.unparse_expr(exc);
        }
        if let Some(cause) = &node.cause {
            self.write_str(" from ");
            self.unparse_expr(cause);
        }
    }

    fn unparse_stmt_try(&mut self, node: &StmtTry<TextRange>) {
        let prev_try_star = self.in_try_star;
        self.in_try_star = false;
        self.fill("try:");
        self.block(|block_self| {
            for stmt in &node.body {
                block_self.unparse_stmt(stmt);
            }
        });

        for handler in &node.handlers {
            self.unparse_excepthandler(handler);
        }

        if !node.orelse.is_empty() {
            self.fill("else:");
            self.block(|block_self| {
                for stmt in &node.orelse {
                    block_self.unparse_stmt(stmt);
                }
            });
        }

        if !node.finalbody.is_empty() {
            self.fill("finally:");
            self.block(|block_self| {
                for stmt in &node.finalbody {
                    block_self.unparse_stmt(stmt);
                }
            });
        }
        self.in_try_star = prev_try_star;
    }
    fn unparse_stmt_try_star(&mut self, node: &StmtTryStar<TextRange>) {
        let prev_try_star = self.in_try_star;
        self.in_try_star = true;
        self.fill("try:");
        self.block(|block_self| {
            for stmt in &node.body {
                block_self.unparse_stmt(stmt);
            }
        });

        for handler in &node.handlers {
            self.unparse_excepthandler(handler);
        }

        if !node.orelse.is_empty() {
            self.fill("else:");
            self.block(|block_self| {
                for stmt in &node.orelse {
                    block_self.unparse_stmt(stmt);
                }
            });
        }

        if !node.finalbody.is_empty() {
            self.fill("finally:");
            self.block(|block_self| {
                for stmt in &node.finalbody {
                    block_self.unparse_stmt(stmt);
                }
            });
        }
        self.in_try_star = prev_try_star;
    }
    fn unparse_stmt_assert(&mut self, node: &StmtAssert<TextRange>) {
        self.fill("assert ");
        self.unparse_expr(&node.test);
        if let Some(msg) = &node.msg {
            self.write_str(", ");
            self.unparse_expr(msg);
        }
    }

    fn unparse_stmt_import(&mut self, node: &StmtImport<TextRange>) {
        self.fill("import ");
        let mut iter = node.names.iter().peekable();
        while let Some(name) = iter.next() {
            self.unparse_alias(name);
            if iter.peek().is_some() {
                self.write_str(", ");
            }
        }
    }
    fn unparse_stmt_import_from(&mut self, node: &StmtImportFrom<TextRange>) {
        self.fill("from ");
        let level = node.level.unwrap_or(Int::new(0));
        self.write_str(&".".repeat(level.to_usize()));
        let module = match &node.module {
            Some(name) => name.to_string(),
            None => "".to_string(),
        };
        self.write_str(&(module + " import "));
        let mut iter = node.names.iter().peekable();
        while let Some(name) = iter.next() {
            self.unparse_alias(name);
            if iter.peek().is_some() {
                self.write_str(", ");
            }
        }
    }
    fn unparse_stmt_global(&mut self, node: &StmtGlobal<TextRange>) {
        self.fill("global ");
        let mut iter = node.names.iter().peekable();
        while let Some(name) = iter.next() {
            self.write_str(name);
            if iter.peek().is_some() {
                self.write_str(", ");
            }
        }
    }
    fn unparse_stmt_nonlocal(&mut self, node: &StmtNonlocal<TextRange>) {
        self.fill("nonlocal ");
        let mut iter = node.names.iter().peekable();
        while let Some(name) = iter.next() {
            self.write_str(name);
            if iter.peek().is_some() {
                self.write_str(", ");
            }
        }
    }
    fn unparse_stmt_expr(&mut self, node: &StmtExpr<TextRange>) {
        // Check if this is a comment or docstring (string literal on its own line)
        if let Expr::Constant(ExprConstant {
            value: Constant::Str(content),
            ..
        }) = &*node.value
        {
            // Special case for comments created using the string constant trick
            // If it's the first line in the file and starts with shebang, handle it specially
            if content.starts_with("#!/") {
                // Don't add an indentation or newline before the shebang
                // Just write it at the start of the file
                self.write_str(content);
                self.fill("");
                return;
            }
            // Handle empty lines for spacing
            else if content.is_empty() {
                self.fill("");
                return;
            }
            // Handle regular comments (starting with #)
            else if content.starts_with('#') {
                // Write the comment as-is without quotes
                self.fill("");
                self.write_str(content);
                return;
            }
            // Handle module header comments with special formatting
            else if content.contains("─ Module:")
                || content.contains("─ Entry Module:")
                || content == "Preserved imports"
            {
                self.fill("");
                self.write_str(&format!("# {}", content));
                return;
            }
            // Handle docstrings - format with triple quotes
            else if self.is_docstring(content) {
                self.fill("");
                self.write_docstring(content);
                self.fill("");
                return;
            }
        }

        // Default handling for non-comment expression statements
        self.fill("");
        self.with_precedence(Precedence::Yield, |block_self| {
            block_self.unparse_expr(&node.value);
        });
    }

    pub fn unparse_expr(&mut self, node: &Expr<TextRange>) {
        match node {
            Expr::BoolOp(data) => self.unparse_expr_bool_op(data),
            Expr::NamedExpr(data) => self.unparse_expr_named_expr(data),
            Expr::BinOp(data) => self.unparse_expr_bin_op(data),
            Expr::UnaryOp(data) => self.unparse_expr_unary_op(data),
            Expr::Lambda(data) => self.unparse_expr_lambda(data),
            Expr::IfExp(data) => self.unparse_expr_if_exp(data),
            Expr::Dict(data) => self.unparse_expr_dict(data),
            Expr::Set(data) => self.unparse_expr_set(data),
            Expr::ListComp(data) => self.unparse_expr_list_comp(data),
            Expr::SetComp(data) => self.unparse_expr_set_comp(data),
            Expr::DictComp(data) => self.unparse_expr_dict_comp(data),
            Expr::GeneratorExp(data) => self.unparse_expr_generator_exp(data),
            Expr::Await(data) => self.unparse_expr_await(data),
            Expr::Yield(data) => self.unparse_expr_yield(data),
            Expr::YieldFrom(data) => self.unparse_expr_yield_from(data),
            Expr::Compare(data) => self.unparse_expr_compare(data),
            Expr::Call(data) => self.unparse_expr_call(data),
            Expr::FormattedValue(data) => self.unparse_expr_formatted_value(data),
            Expr::JoinedStr(data) => self.unparse_expr_joined_str(data, false),
            Expr::Constant(data) => self.unparse_expr_constant(data),
            Expr::Attribute(data) => self.unparse_expr_attribute(data),
            Expr::Subscript(data) => self.unparse_expr_subscript(data),
            Expr::Starred(data) => self.unparse_expr_starred(data),
            Expr::Name(data) => self.unparse_expr_name(data),
            Expr::List(data) => self.unparse_expr_list(data),
            Expr::Tuple(data) => self.unparse_expr_tuple(data),
            Expr::Slice(data) => self.unparse_expr_slice(data),
        }
    }

    fn unparse_expr_bool_op(&mut self, node: &ExprBoolOp<TextRange>) {
        let enum_member = Expr::BoolOp(node.to_owned());
        let mut operator_precedence = get_precedence(&enum_member);
        let operator = match node.op {
            BoolOp::And => " and ",
            BoolOp::Or => " or ",
        };

        let mut values_iter = node.values.iter().peekable();
        self.delimit_precedence(&enum_member, |block_self| {
            while let Some(expr) = values_iter.next() {
                operator_precedence += 1;
                block_self.with_precedence_num(operator_precedence, |prec_self| {
                    prec_self.unparse_expr(expr);
                });
                if values_iter.peek().is_some() {
                    block_self.write_str(operator);
                }
            }
        });
    }

    fn unparse_expr_named_expr(&mut self, node: &ExprNamedExpr<TextRange>) {
        let enum_member = Expr::NamedExpr(node.to_owned());
        self.delimit_precedence(&enum_member, |block_self| {
            block_self.with_precedence(Precedence::Atom, |prec_self| {
                prec_self.unparse_expr(&node.target);
                prec_self.write_str(" := ");
                prec_self.unparse_expr(&node.value);
            });
        })
    }

    fn unparse_expr_bin_op(&mut self, node: &ExprBinOp<TextRange>) {
        let enum_member = Expr::BinOp(node.to_owned());

        self.delimit_precedence(&enum_member, |block_self| {
            block_self.unparse_expr(&node.left);
            block_self.write_str(" ");
            block_self.unparse_operator(&node.op);
            block_self.write_str(" ");
            block_self.unparse_expr(&node.right);
        })
    }

    fn unparse_expr_unary_op(&mut self, node: &ExprUnaryOp<TextRange>) {
        let enum_member = Expr::UnaryOp(node.to_owned());
        let operator = match node.op {
            UnaryOp::Invert => "~",
            UnaryOp::Not => "not ",
            UnaryOp::UAdd => "+",
            UnaryOp::USub => "-",
        };

        self.delimit_precedence(&enum_member, |block_self| {
            block_self.write_str(operator);
            block_self.unparse_expr(&node.operand)
        })
    }
    fn unparse_expr_lambda(&mut self, node: &ExprLambda<TextRange>) {
        let enum_member = Expr::Lambda(node.to_owned());

        self.delimit_precedence(&enum_member, |block_self| {
            block_self.write_str("lambda ");
            block_self.unparse_arguments(&node.args);
            block_self.write_str(": ");
            block_self.unparse_expr(&node.body);
        })
    }
    fn unparse_expr_if_exp(&mut self, node: &ExprIfExp<TextRange>) {
        let enum_member = Expr::IfExp(node.to_owned());
        self.delimit_precedence(&enum_member, |block_self| {
            block_self.with_precedence_num(Precedence::Test.value() + 1, |prec_self| {
                prec_self.unparse_expr(&node.body);
                prec_self.write_str(" if ");
                prec_self.unparse_expr(&node.test);
            });
            block_self.with_precedence(Precedence::Test, |prec_self| {
                prec_self.write_str(" else ");
                prec_self.unparse_expr(&node.orelse);
            });
        })
    }

    fn unparse_expr_dict(&mut self, node: &ExprDict<TextRange>) {
        let mut zipped = node.keys.iter().zip(node.values.iter()).peekable();

        self.write_str("{");
        while let Some((key, value)) = zipped.next() {
            match key {
                Some(key_value) => {
                    self.unparse_expr(key_value);
                    self.write_str(": ");
                }
                None => {
                    self.write_str("**");
                }
            }
            self.with_precedence_num(EXPR_PRECEDENCE, |prec_self| {
                prec_self.unparse_expr(value);
            });

            if zipped.peek().is_some() {
                self.write_str(", ");
            }
        }
        self.write_str("}");
    }

    fn unparse_expr_set(&mut self, node: &ExprSet<TextRange>) {
        if !node.elts.is_empty() {
            self.write_str("{");
            let mut elts_iter = node.elts.iter().peekable();
            while let Some(expr) = elts_iter.next() {
                self.unparse_expr(expr);
                if elts_iter.peek().is_some() {
                    self.write_str(", ");
                }
            }
            self.write_str("}");
        } else {
            self.write_str("{*()}");
        }
    }

    fn unparse_expr_list_comp(&mut self, node: &ExprListComp<TextRange>) {
        self.write_str("[");
        self.unparse_expr(&node.elt);
        for generator in &node.generators {
            self.unparse_comprehension(generator);
        }
        self.write_str("]");
    }

    fn unparse_expr_set_comp(&mut self, node: &ExprSetComp<TextRange>) {
        self.write_str("{");
        self.unparse_expr(&node.elt);

        for generator in &node.generators {
            self.unparse_comprehension(generator);
        }
        self.write_str("}");
    }

    fn unparse_expr_dict_comp(&mut self, node: &ExprDictComp<TextRange>) {
        self.write_str("{");
        self.unparse_expr(&node.key);
        self.write_str(": ");
        self.unparse_expr(&node.value);

        for generator in &node.generators {
            self.unparse_comprehension(generator);
        }
        self.write_str("}");
    }

    fn unparse_expr_generator_exp(&mut self, node: &ExprGeneratorExp<TextRange>) {
        self.write_str("(");
        self.unparse_expr(&node.elt);

        for generator in &node.generators {
            self.unparse_comprehension(generator);
        }
        self.write_str(")");
    }

    fn unparse_expr_await(&mut self, node: &ExprAwait<TextRange>) {
        let enum_member = Expr::Await(node.to_owned());
        self.delimit_precedence(&enum_member, |block_self| {
            block_self.write_str("await ");
            block_self.with_precedence(Precedence::Atom, |prec_self| {
                prec_self.unparse_expr(&node.value);
            });
        })
    }

    fn unparse_expr_yield(&mut self, node: &ExprYield<TextRange>) {
        let enum_member = Expr::Yield(node.to_owned());
        self.delimit_precedence(&enum_member, |block_self| {
            block_self.write_str("yield");
            if let Some(expr) = &node.value {
                block_self.write_str(" ");
                block_self.with_precedence(Precedence::Atom, |prec_self| {
                    prec_self.unparse_expr(expr);
                });
            }
        })
    }

    fn unparse_expr_yield_from(&mut self, node: &ExprYieldFrom<TextRange>) {
        let enum_member = Expr::YieldFrom(node.to_owned());
        self.delimit_precedence(&enum_member, |block_self| {
            block_self.write_str("yield from ");

            block_self.with_precedence(Precedence::Atom, |prec_self| {
                prec_self.unparse_expr(&node.value);
            });
        })
    }

    fn unparse_expr_compare(&mut self, node: &ExprCompare<TextRange>) {
        let enum_member = Expr::Compare(node.to_owned());
        let zipped = node.ops.iter().zip(node.comparators.iter());
        self.delimit_precedence(&enum_member, |block_self| {
            block_self.unparse_expr(&node.left);
            for (op, comp) in zipped {
                let operator = match op {
                    CmpOp::Eq => " == ",
                    CmpOp::Gt => " > ",
                    CmpOp::GtE => " >= ",
                    CmpOp::In => " in ",
                    CmpOp::Is => " is ",
                    CmpOp::IsNot => " is not ",
                    CmpOp::Lt => " < ",
                    CmpOp::LtE => " <= ",
                    CmpOp::NotEq => " != ",
                    CmpOp::NotIn => " not in ",
                };
                block_self.write_str(operator);
                block_self.unparse_expr(comp);
            }
        })
    }

    fn unparse_expr_call(&mut self, node: &ExprCall<TextRange>) {
        self.unparse_expr(&node.func);
        let mut args_iter = node.args.iter().peekable();
        let mut keywords_iter = node.keywords.iter().peekable();
        self.write_str("(");
        while let Some(arg) = args_iter.next() {
            self.unparse_expr(arg);
            if args_iter.peek().is_some() || keywords_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
        while let Some(keyword) = keywords_iter.next() {
            self.unparse_keyword(keyword);
            if keywords_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
        self.write_str(")");
    }

    fn unparse_expr_formatted_value(&mut self, node: &ExprFormattedValue<TextRange>) {
        self.write_str("{");
        let mut inner_unparser = Unparser::new();
        inner_unparser.unparse_expr(&node.value);
        let inner_expr = inner_unparser.source.as_str();
        if inner_expr.starts_with("{") {
            self.write_str(" ");
        }
        self.write_str(inner_expr);
        if node.conversion != ConversionFlag::None {
            self.write_str("!");
            let buf = &[node.conversion as u8];
            let c = std::str::from_utf8(buf).unwrap();
            self.write_str(c);
        }
        if let Some(format_spec) = &node.format_spec {
            self.write_str(":");
            match format_spec.deref() {
                Expr::JoinedStr(joined_str) => {
                    if !joined_str.values.is_empty() {
                        self.unparse_expr_joined_str(joined_str, true);
                    }
                }
                _ => self.unparse_expr(format_spec),
            };
        }
        self.write_str("}");
    }

    fn unparse_expr_joined_str(&mut self, node: &ExprJoinedStr<TextRange>, is_spec: bool) {
        if !is_spec {
            self.write_str("f");
        }
        let mut expr_source = String::new();

        let mut formatted_values_sources: Vec<String> = Vec::new();
        for expr in node.values.iter() {
            let mut inner_unparser = Unparser::new();
            match expr {
                Expr::Constant(ExprConstant { value, .. }) => {
                    if let Constant::Str(str_) = value {
                        let escaped = str_.replace('{', "{{").replace('}', "}}");
                        inner_unparser.write_str(&escaped);
                    } else {
                        unreachable!()
                    }
                    expr_source += inner_unparser.source.as_str();
                }
                Expr::FormattedValue(formatted) => {
                    expr_source += &("{".to_owned()
                        + formatted_values_sources.len().to_string().as_str()
                        + "}");
                    inner_unparser.unparse_expr_formatted_value(formatted);
                    formatted_values_sources.push(inner_unparser.source);
                }
                _ => {
                    inner_unparser.unparse_expr(expr);
                    expr_source += inner_unparser.source.as_str();
                }
            }
        }

        if is_spec {
            for (i, formatted) in formatted_values_sources.iter().enumerate() {
                let to_replace = "{".to_owned() + i.to_string().as_str() + "}";
                expr_source = expr_source.replace(&to_replace, formatted)
            }
            self.write_str(&expr_source);
        } else {
            let mut escaped_source =
                rustpython_literal::escape::UnicodeEscape::new_repr(&expr_source)
                    .str_repr()
                    .to_string()
                    .unwrap();
            for (i, formatted) in formatted_values_sources.iter().enumerate() {
                let to_replace = "{".to_owned() + i.to_string().as_str() + "}";
                escaped_source = escaped_source.replace(&to_replace, formatted)
            }

            let has_single = escaped_source.contains("'");
            let has_double = escaped_source.contains("\"");

            if has_single
                && has_double
                && escaped_source.starts_with("\"")
                && escaped_source.ends_with("\"")
            {
                escaped_source = replace_first_and_last(&escaped_source, "\"\"\"")
            } else if has_single
                && has_double
                && escaped_source.starts_with("'")
                && escaped_source.ends_with("'")
            {
                escaped_source = replace_first_and_last(&escaped_source, "'''")
            } else if has_single {
                escaped_source = replace_first_and_last(&escaped_source, "\"")
            }

            self.write_str(&escaped_source);
        }
    }

    fn _unparse_constant(&mut self, constant: &Constant) {
        let inf_str = "1e309";
        match constant {
            Constant::Tuple(values) => {
                self.write_str("(");
                let mut values_iter = values.iter().peekable();
                while let Some(value) = values_iter.next() {
                    self._unparse_constant(value);
                    if values_iter.peek().is_some() || values.len() == 1 {
                        self.write_str(", ");
                    }
                }
                self.write_str(")");
            }
            Constant::Ellipsis => self.write_str("..."),
            Constant::Bool(value) => {
                if *value {
                    self.write_str("True")
                } else {
                    self.write_str("False")
                }
            }
            Constant::Bytes(value) => {
                let escaped = rustpython_literal::escape::AsciiEscape::new_repr(value)
                    .bytes_repr()
                    .to_string()
                    .unwrap();
                self.write_str(&escaped);
            }
            Constant::Int(value) => self.write_str(&value.to_string()),
            Constant::Str(value) => {
                let escaped = rustpython_literal::escape::UnicodeEscape::new_repr(value)
                    .str_repr()
                    .to_string()
                    .unwrap();

                self.write_str(&escaped);
            }
            Constant::None => self.write_str("None"),
            Constant::Complex { real, imag } => {
                if real.is_infinite() || imag.is_infinite() {
                    self.write_str(&constant.to_string().replace("inf", inf_str));
                } else {
                    self.write_str(&constant.to_string());
                }
            }
            Constant::Float(value) => {
                if value.is_infinite() {
                    self.write_str(inf_str);
                } else {
                    let mut str_value = value.to_string();
                    if value.fract() == 0.0 {
                        let mut trailing_zeroes = 0;
                        while str_value.ends_with("0") {
                            str_value.pop();
                            trailing_zeroes += 1;
                        }
                        str_value = format!("{}e{}", str_value, trailing_zeroes);
                    } else if str_value.starts_with(&format!("0.{}", "0".repeat(5))) {
                        let mut trimmed = str_value[2..].to_owned();
                        let mut factor = 1;
                        while trimmed.starts_with("0") {
                            trimmed = trimmed[1..].to_owned();
                            factor += 1;
                        }
                        str_value = format!("{}e-{}", trimmed, factor);
                    }
                    self.write_str(&str_value);
                }
            }
        }
    }

    fn unparse_expr_constant(&mut self, node: &ExprConstant<TextRange>) {
        if node.kind.as_deref().is_some_and(|kind| kind == "u") {
            self.write_str("u");
        }
        self._unparse_constant(&node.value)
    }

    fn unparse_expr_attribute(&mut self, node: &ExprAttribute<TextRange>) {
        self.unparse_expr(&node.value);
        self.write_str(".");
        self.write_str(&node.attr);
    }
    fn unparse_expr_subscript(&mut self, node: &ExprSubscript<TextRange>) {
        self.with_precedence(Precedence::Atom, |prec_self| {
            prec_self.unparse_expr(&node.value);
        });
        self.write_str("[");
        self.unparse_expr(&node.slice);
        self.write_str("]");
    }
    fn unparse_expr_starred(&mut self, node: &ExprStarred<TextRange>) {
        self.write_str("*");
        self.with_precedence_num(EXPR_PRECEDENCE, |prec_self| {
            prec_self.unparse_expr(&node.value);
        });
    }

    fn unparse_expr_name(&mut self, node: &ExprName<TextRange>) {
        self.write_str(node.id.as_str())
    }
    fn unparse_expr_list(&mut self, node: &ExprList<TextRange>) {
        let mut elts_iter = node.elts.iter().peekable();
        self.write_str("[");
        while let Some(expr) = elts_iter.next() {
            self.unparse_expr(expr);
            if elts_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
        self.write_str("]");
    }

    fn unparse_expr_tuple(&mut self, node: &ExprTuple<TextRange>) {
        let mut elts_iter = node.elts.iter().peekable();
        let should_delimit =
            node.elts.is_empty() || self.precedence_level > Precedence::Tuple.value();
        if should_delimit {
            self.write_str("(");
        }

        while let Some(expr) = elts_iter.next() {
            self.unparse_expr(expr);
            if elts_iter.peek().is_some() || node.elts.len() == 1 {
                self.write_str(", ");
            }
        }
        if should_delimit {
            self.write_str(")");
        }
    }

    fn unparse_expr_slice(&mut self, node: &ExprSlice<TextRange>) {
        if let Some(lower) = &node.lower {
            self.unparse_expr(lower);
        }
        self.write_str(":");
        if let Some(upper) = &node.upper {
            self.unparse_expr(upper);
        }
        if let Some(step) = &node.step {
            self.write_str(":");
            self.unparse_expr(step);
        }
    }

    fn unparse_operator(&mut self, node: &Operator) {
        self.write_str(match node {
            Operator::Add => "+",
            Operator::Sub => "-",
            Operator::BitOr => "|",
            Operator::BitAnd => "&",
            Operator::BitXor => "^",
            Operator::Div => "/",
            Operator::FloorDiv => "//",
            Operator::LShift => "<<",
            Operator::MatMult => "@",
            Operator::Mod => "%",
            Operator::Pow => "**",
            Operator::RShift => ">>",
            Operator::Mult => "*",
        })
    }

    fn unparse_comprehension(&mut self, node: &Comprehension<TextRange>) {
        if node.is_async {
            self.write_str(" async for ");
        } else {
            self.write_str(" for ");
        }
        self.with_precedence(Precedence::Tuple, |prec_self| {
            prec_self.unparse_expr(&node.target);
        });

        self.write_str(" in ");

        self.with_precedence_num(Precedence::Test.value() + 1, |prec_self| {
            prec_self.unparse_expr(&node.iter);
            for if_ in &node.ifs {
                prec_self.write_str(" if ");
                prec_self.unparse_expr(if_);
            }
        });
    }

    fn unparse_excepthandler(&mut self, node: &ExceptHandler<TextRange>) {
        match node {
            ExceptHandler::ExceptHandler(data) => self.unparse_excepthandler_except_handler(data),
        }
    }

    fn unparse_excepthandler_except_handler(
        &mut self,
        node: &ExceptHandlerExceptHandler<TextRange>,
    ) {
        self.fill("except");
        if self.in_try_star {
            self.write_str("*")
        }

        if let Some(type_) = &node.type_ {
            self.write_str(" ");
            self.unparse_expr(type_);
        }
        if let Some(name) = &node.name {
            self.write_str(" as ");
            self.write_str(name);
        }

        self.write_str(":");
        self.block(|block_self| {
            for stmt in &node.body {
                block_self.unparse_stmt(stmt);
            }
        });
    }

    fn unparse_arguments(&mut self, node: &Arguments<TextRange>) {
        let mut posonly_iter = node.posonlyargs.iter().peekable();
        let mut args_iter = node.args.iter().peekable();
        let mut kw_iter = node.kwonlyargs.iter().peekable();
        while let Some(posonly) = posonly_iter.next() {
            self.unparse_arg(posonly.as_arg());
            if let Some(default) = &posonly.default {
                self.write_str("=");
                self.unparse_expr(default);
            }

            if posonly_iter.peek().is_some() {
                self.write_str(", ");
            }
        }

        if !node.posonlyargs.is_empty() {
            self.write_str(", /,");
        }

        while let Some(arg) = args_iter.next() {
            self.unparse_arg(arg.as_arg());
            if let Some(default) = &arg.default {
                self.write_str("=");
                self.unparse_expr(default);
            }
            if args_iter.peek().is_some()
                || node.vararg.is_some()
                || kw_iter.peek().is_some()
                || node.kwarg.is_some()
            {
                self.write_str(", ");
            }
        }

        if let Some(vararg) = &node.vararg {
            self.write_str("*");
            self.write_str(&vararg.arg);

            if let Some(annotation) = &vararg.annotation {
                self.write_str(": ");
                self.unparse_expr(annotation);
            }
            if kw_iter.peek().is_some() || node.kwarg.is_some() {
                self.write_str(", ");
            }
        } else if !node.kwonlyargs.is_empty() {
            self.write_str("*, ");
        }

        while let Some(kw) = kw_iter.next() {
            self.unparse_arg(kw.as_arg());
            if let Some(default) = &kw.default {
                self.write_str("=");
                self.unparse_expr(default);
            }
            if kw_iter.peek().is_some() || node.kwarg.is_some() {
                self.write_str(", ");
            }
        }

        if let Some(kwarg) = &node.kwarg {
            self.write_str("**");
            self.write_str(&kwarg.arg);
            if let Some(annotation) = &kwarg.annotation {
                self.write_str(": ");
                self.unparse_expr(annotation);
            }
        }
    }

    fn unparse_arg(&mut self, node: &Arg<TextRange>) {
        self.write_str(node.arg.as_str());
        if let Some(annotation) = &node.annotation {
            self.write_str(": ");
            self.unparse_expr(annotation);
        }
    }

    fn unparse_keyword(&mut self, node: &Keyword<TextRange>) {
        if let Some(arg) = &node.arg {
            self.write_str(arg.as_str());
            self.write_str("=");
        } else {
            self.write_str("**");
        }

        self.unparse_expr(&node.value);
    }

    fn unparse_alias(&mut self, node: &Alias<TextRange>) {
        self.write_str(node.name.as_str());
        if node.asname.is_some() {
            self.write_str(&format!(" as {}", node.asname.as_ref().unwrap()));
        }
    }

    fn unparse_withitem(&mut self, node: &WithItem<TextRange>) {
        self.unparse_expr(&node.context_expr);
        if let Some(var) = &node.optional_vars {
            self.write_str(" as ");
            self.unparse_expr(var);
        }
    }

    fn unparse_match_case(&mut self, node: &MatchCase<TextRange>) {
        self.fill("case ");
        self.unparse_pattern(&node.pattern);
        if let Some(guard) = &node.guard {
            self.write_str(" if ");
            self.unparse_expr(guard);
        }
        self.write_str(":");
        self.block(|block_self| {
            for stmt in &node.body {
                block_self.unparse_stmt(stmt);
            }
        });
    }

    fn unparse_pattern(&mut self, node: &Pattern<TextRange>) {
        match node {
            Pattern::MatchValue(data) => self.unparse_pattern_match_value(data),
            Pattern::MatchSingleton(data) => self.unparse_pattern_match_singleton(data),
            Pattern::MatchSequence(data) => self.unparse_pattern_match_sequence(data),
            Pattern::MatchMapping(data) => self.unparse_pattern_match_mapping(data),
            Pattern::MatchClass(data) => self.unparse_pattern_match_class(data),
            Pattern::MatchStar(data) => self.unparse_pattern_match_star(data),
            Pattern::MatchAs(data) => self.unparse_pattern_match_as(data),
            Pattern::MatchOr(data) => self.unparse_pattern_match_or(data),
        }
    }

    fn unparse_pattern_match_value(&mut self, node: &PatternMatchValue<TextRange>) {
        self.unparse_expr(&node.value)
    }

    fn unparse_pattern_match_singleton(&mut self, node: &PatternMatchSingleton<TextRange>) {
        self._unparse_constant(&node.value);
    }

    fn unparse_pattern_match_sequence(&mut self, node: &PatternMatchSequence<TextRange>) {
        let mut patterns_iter = node.patterns.iter().peekable();
        self.write_str("[");
        while let Some(pattern) = patterns_iter.next() {
            self.unparse_pattern(pattern);
            if patterns_iter.peek().is_some() {
                self.write_str(" , ");
            }
        }
        self.write_str("]");
    }

    fn unparse_pattern_match_mapping(&mut self, node: &PatternMatchMapping<TextRange>) {
        let mut pairs_iter = node.keys.iter().zip(node.patterns.iter()).peekable();
        self.write_str("{");
        while let Some((key, pattern)) = pairs_iter.next() {
            self.unparse_expr(key);
            self.write_str(": ");
            self.unparse_pattern(pattern);
            if pairs_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
        if let Some(rest) = &node.rest {
            if !node.keys.is_empty() {
                self.write_str(", ");
            }
            self.write_str("**");
            self.write_str(rest.as_str());
        }

        self.write_str("}");
    }

    fn unparse_pattern_match_class(&mut self, node: &PatternMatchClass<TextRange>) {
        let mut patterns_iter = node.patterns.iter().peekable();
        let mut kwd_iter = node
            .kwd_attrs
            .iter()
            .zip(node.kwd_patterns.iter())
            .peekable();
        self.unparse_expr(&node.cls);
        self.write_str("(");
        while let Some(pattern) = patterns_iter.next() {
            self.unparse_pattern(pattern);
            if patterns_iter.peek().is_some() || kwd_iter.peek().is_some() {
                self.write_str(", ");
            }
        }
        while let Some((attr, pattern)) = kwd_iter.next() {
            self.write_str(attr.as_str());
            self.write_str("=");
            self.unparse_pattern(pattern);
            if kwd_iter.peek().is_some() {
                self.write_str(", ");
            }
        }

        self.write_str(")");
    }

    fn unparse_pattern_match_star(&mut self, node: &PatternMatchStar<TextRange>) {
        let name = match &node.name {
            Some(name) => name.as_str(),
            None => "_",
        };
        self.write_str("*");
        self.write_str(name);
    }

    fn unparse_pattern_match_as(&mut self, node: &PatternMatchAs<TextRange>) {
        match &node.name {
            Some(name) => match &node.pattern {
                Some(pattern) => {
                    let with_parens = self.precedence_level > Precedence::Test.value();
                    if with_parens {
                        self.write_str("(");
                    }
                    self.with_precedence(Precedence::Bor, |prec_self| {
                        prec_self.unparse_pattern(pattern);
                    });
                    self.write_str(" as ");
                    self.write_str(name);

                    if with_parens {
                        self.write_str(")");
                    }
                }
                None => {
                    self.write_str(name);
                }
            },
            None => {
                self.write_str("_");
            }
        };
    }

    fn unparse_pattern_match_or(&mut self, node: &PatternMatchOr<TextRange>) {
        let mut patterns_iter = node.patterns.iter().peekable();
        while let Some(pattern) = patterns_iter.next() {
            self.unparse_pattern(pattern);
            if patterns_iter.peek().is_some() {
                self.write_str(" | ");
            }
        }
    }

    fn unparse_type_param(&mut self, node: &TypeParam<TextRange>) {
        match node {
            TypeParam::TypeVar(data) => self.unparse_type_param_type_var(data),
            TypeParam::ParamSpec(data) => self.unparse_type_param_param_spec(data),
            TypeParam::TypeVarTuple(data) => self.unparse_type_param_type_var_tuple(data),
        }
    }

    fn unparse_type_param_type_var(&mut self, node: &TypeParamTypeVar<TextRange>) {
        self.write_str(&node.name);
        if let Some(bound) = &node.bound {
            self.write_str(": ");
            self.unparse_expr(bound);
        }
    }

    fn unparse_type_param_param_spec(&mut self, node: &TypeParamParamSpec<TextRange>) {
        self.write_str("**");
        self.write_str(&node.name);
    }

    /// Check if a string literal should be treated as a docstring
    fn is_docstring(&self, content: &str) -> bool {
        // A docstring is typically a string literal that:
        // 1. Is not a comment (doesn't start with #)
        // 2. Contains meaningful documentation text
        // 3. Is not a special module marker or preserved import
        //
        // In Python, docstrings can be very short (even single words like "Constructor.")
        // so we don't impose arbitrary length restrictions.

        // Exclude obvious non-docstring patterns
        if content.starts_with('#') || content.starts_with("#!/") || content.is_empty() {
            return false;
        }

        // Exclude special serpen module markers
        if content.contains("─ Module:")
            || content.contains("─ Entry Module:")
            || content == "Preserved imports"
        {
            return false;
        }

        // For now, treat any remaining string literal in a statement expression as a potential docstring
        // TODO: Add positional context awareness to check if this is the first statement in a module/function/class
        // This would require tracking the parsing context or passing additional information to this method
        true
    }

    /// Write a docstring with proper triple-quote formatting
    fn write_docstring(&mut self, content: &str) {
        // Handle edge cases with quotes in the content
        let contains_triple_single = content.contains("'''");
        let contains_triple_double = content.contains("\"\"\"");

        // Choose quote style to minimize escaping - prefer double quotes (Python convention)
        let use_single_quotes = contains_triple_double && !contains_triple_single;
        let quote_style = if use_single_quotes { "'''" } else { "\"\"\"" };

        // Escape content if it conflicts with chosen quote style
        let escaped_content = if quote_style == "\"\"\"" && contains_triple_double {
            content.replace("\"\"\"", "\\\"\\\"\\\"")
        } else if quote_style == "'''" && contains_triple_single {
            content.replace("'''", "\\'\\'\\'")
        } else {
            content.to_string()
        };

        // Write the docstring with proper formatting
        self.write_str(quote_style);

        if content.contains('\n') {
            // Multi-line docstring - follow PEP 257 format
            if !content.starts_with('\n') {
                self.write_str("\n");
            }
            self.write_str(&escaped_content);
            if !content.ends_with('\n') {
                self.write_str("\n");
            }
        } else {
            // Single-line docstring
            self.write_str(&escaped_content);
        }

        self.write_str(quote_style);
    }

    fn unparse_type_param_type_var_tuple(&mut self, node: &TypeParamTypeVarTuple<TextRange>) {
        self.write_str("*");
        self.write_str(&node.name);
    }
}
