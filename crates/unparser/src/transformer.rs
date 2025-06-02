use rustpython_ast::{
    Alias, Arg, ArgWithDefault, Arguments, Comprehension, ExceptHandler,
    ExceptHandlerExceptHandler, Expr, ExprAttribute, ExprAwait, ExprBinOp, ExprBoolOp, ExprCall,
    ExprCompare, ExprConstant, ExprDict, ExprDictComp, ExprFormattedValue, ExprGeneratorExp,
    ExprIfExp, ExprJoinedStr, ExprLambda, ExprList, ExprListComp, ExprName, ExprNamedExpr, ExprSet,
    ExprSetComp, ExprSlice, ExprStarred, ExprSubscript, ExprTuple, ExprUnaryOp, ExprYield,
    ExprYieldFrom, Keyword, MatchCase, Pattern, PatternMatchAs, PatternMatchClass,
    PatternMatchMapping, PatternMatchOr, PatternMatchSequence, PatternMatchSingleton,
    PatternMatchStar, PatternMatchValue, Stmt, StmtAnnAssign, StmtAssert, StmtAssign, StmtAsyncFor,
    StmtAsyncFunctionDef, StmtAsyncWith, StmtAugAssign, StmtBreak, StmtClassDef, StmtContinue,
    StmtDelete, StmtExpr, StmtFor, StmtFunctionDef, StmtGlobal, StmtIf, StmtImport, StmtImportFrom,
    StmtMatch, StmtNonlocal, StmtPass, StmtRaise, StmtReturn, StmtTry, StmtTryStar, StmtTypeAlias,
    StmtWhile, StmtWith, TypeParam, TypeParamParamSpec, TypeParamTypeVar, TypeParamTypeVarTuple,
    WithItem, text_size::TextRange,
};

fn box_expr_option(expr: Option<Expr>) -> Option<Box<Expr>> {
    expr.map(Box::new)
}
#[allow(unused_mut)]
pub trait Transformer {
    #[allow(unused_variables)]
    fn on_enter_annotation(&mut self, expr: &Expr) {}
    #[allow(unused_variables)]
    fn on_exit_annotation(&mut self, expr: &Option<Expr>) {}

    fn visit_annotation(&mut self, expr: Box<Expr>) -> Option<Expr> {
        let unboxed_annotation = *expr;
        self.on_enter_annotation(&unboxed_annotation);
        let new_annotation = self.visit_expr(unboxed_annotation);
        self.on_exit_annotation(&new_annotation);
        new_annotation
    }

    fn visit_stmt_vec(&mut self, stmts: Vec<Stmt>) -> Vec<Stmt> {
        let mut new_stmts: Vec<Stmt> = Vec::new();

        for stmt in stmts {
            if let Some(new_stmt) = self.visit_stmt(stmt) {
                new_stmts.push(new_stmt);
            }
        }

        new_stmts
    }

    fn visit_stmt(&mut self, mut stmt: Stmt) -> Option<Stmt> {
        self.generic_visit_stmt(stmt)
    }

    fn generic_visit_stmt(&mut self, mut stmt: Stmt) -> Option<Stmt> {
        match stmt {
            Stmt::Delete(del) => self.visit_stmt_delete(del).map(Stmt::Delete),
            Stmt::Assert(assert) => self.visit_stmt_assert(assert).map(Stmt::Assert),
            Stmt::AnnAssign(ann_assign) => {
                self.visit_stmt_ann_assign(ann_assign).map(Stmt::AnnAssign)
            }
            Stmt::For(for_) => self.visit_stmt_for(for_).map(Stmt::For),
            Stmt::AsyncFor(async_for) => self.visit_stmt_async_for(async_for).map(Stmt::AsyncFor),
            Stmt::FunctionDef(func) => self.visit_stmt_function_def(func).map(Stmt::FunctionDef),
            Stmt::AsyncFunctionDef(async_func) => self
                .visit_stmt_async_function_def(async_func)
                .map(Stmt::AsyncFunctionDef),
            Stmt::AsyncWith(async_with) => {
                self.visit_stmt_async_with(async_with).map(Stmt::AsyncWith)
            }
            Stmt::With(with) => self.visit_stmt_with(with).map(Stmt::With),
            Stmt::Break(break_) => self.visit_stmt_break(break_).map(Stmt::Break),
            Stmt::Pass(pass) => self.visit_stmt_pass(pass).map(Stmt::Pass),
            Stmt::Continue(continue_) => self.visit_stmt_continue(continue_).map(Stmt::Continue),
            Stmt::Return(return_) => self.visit_stmt_return(return_).map(Stmt::Return),
            Stmt::Raise(raise) => self.visit_stmt_raise(raise).map(Stmt::Raise),
            Stmt::ClassDef(stmt_class_def) => self
                .visit_stmt_class_def(stmt_class_def)
                .map(Stmt::ClassDef),
            Stmt::Assign(stmt_assign) => self.visit_stmt_assign(stmt_assign).map(Stmt::Assign),
            Stmt::TypeAlias(stmt_type_alias) => self
                .visit_stmt_type_alias(stmt_type_alias)
                .map(Stmt::TypeAlias),
            Stmt::AugAssign(stmt_aug_assign) => self
                .visit_stmt_aug_assign(stmt_aug_assign)
                .map(Stmt::AugAssign),
            Stmt::While(stmt_while) => self.visit_stmt_while(stmt_while).map(Stmt::While),
            Stmt::If(stmt_if) => self.visit_stmt_if(stmt_if).map(Stmt::If),
            Stmt::Match(stmt_match) => self.visit_stmt_match(stmt_match).map(Stmt::Match),
            Stmt::Try(stmt_try) => self.visit_stmt_try(stmt_try).map(Stmt::Try),
            Stmt::TryStar(stmt_try_star) => {
                self.visit_stmt_try_star(stmt_try_star).map(Stmt::TryStar)
            }
            Stmt::Import(stmt_import) => self.visit_stmt_import(stmt_import).map(Stmt::Import),
            Stmt::ImportFrom(stmt_import_from) => self
                .visit_stmt_import_from(stmt_import_from)
                .map(Stmt::ImportFrom),
            Stmt::Global(stmt_global) => self.visit_stmt_global(stmt_global).map(Stmt::Global),
            Stmt::Nonlocal(stmt_nonlocal) => {
                self.visit_stmt_nonlocal(stmt_nonlocal).map(Stmt::Nonlocal)
            }
            Stmt::Expr(stmt_expr) => self.visit_stmt_expr(stmt_expr).map(Stmt::Expr),
        }
    }

    fn generic_visit_keyword_vec(&mut self, mut keywords: Vec<Keyword>) -> Vec<Keyword> {
        let mut new_keywords = Vec::new();

        for keyword in keywords {
            if let Some(new_keyword) = self.visit_keyword(keyword) {
                new_keywords.push(new_keyword);
            }
        }
        new_keywords
    }

    fn visit_keyword(&mut self, mut keyword: Keyword) -> Option<Keyword> {
        self.generic_visit_keyword(keyword)
    }

    fn generic_visit_keyword(&mut self, mut keyword: Keyword) -> Option<Keyword> {
        keyword.value = self
            .visit_expr(keyword.value)
            .expect("Cannot remove value from keyword");

        Some(keyword)
    }

    fn visit_stmt_class_def(&mut self, mut stmt: StmtClassDef) -> Option<StmtClassDef> {
        self.generic_visit_stmt_class_def(stmt)
    }

    fn generic_visit_stmt_class_def(&mut self, mut stmt: StmtClassDef) -> Option<StmtClassDef> {
        stmt.decorator_list = self.visit_expr_vec(stmt.decorator_list);

        stmt.type_params = self.generic_visit_type_param_vec(stmt.type_params);
        stmt.bases = self.visit_expr_vec(stmt.bases);
        stmt.keywords = self.generic_visit_keyword_vec(stmt.keywords);
        stmt.body = self.visit_stmt_vec(stmt.body);

        if stmt.body.is_empty() {
            stmt.body.push(Stmt::Pass(StmtPass {
                range: TextRange::default(),
            }));
        }

        Some(stmt)
    }

    fn visit_stmt_assign(&mut self, mut stmt: StmtAssign) -> Option<StmtAssign> {
        self.generic_visit_stmt_assign(stmt)
    }

    fn generic_visit_stmt_assign(&mut self, mut stmt: StmtAssign) -> Option<StmtAssign> {
        stmt.targets = self.visit_expr_vec(stmt.targets);
        if stmt.targets.is_empty() {
            panic!("Cannot remove all targets from assignment")
        }
        stmt.value = Box::new(
            self.visit_expr(*stmt.value)
                .expect("Cannot remove value from assignment"),
        );

        Some(stmt)
    }

    fn visit_stmt_type_alias(&mut self, mut stmt: StmtTypeAlias) -> Option<StmtTypeAlias> {
        self.generic_visit_stmt_type_alias(stmt)
    }

    fn generic_visit_stmt_type_alias(&mut self, mut stmt: StmtTypeAlias) -> Option<StmtTypeAlias> {
        stmt.name = Box::new(
            self.visit_expr(*stmt.name)
                .expect("Cannot remove name from type alias"),
        );
        stmt.type_params = self.generic_visit_type_param_vec(stmt.type_params);
        stmt.value = Box::new(
            self.visit_expr(*stmt.value)
                .expect("Cannot remove value from type alias"),
        );

        Some(stmt)
    }

    fn visit_stmt_aug_assign(&mut self, mut stmt: StmtAugAssign) -> Option<StmtAugAssign> {
        self.generic_visit_stmt_aug_assign(stmt)
    }

    fn generic_visit_stmt_aug_assign(&mut self, mut stmt: StmtAugAssign) -> Option<StmtAugAssign> {
        stmt.value = Box::new(
            self.visit_expr(*stmt.value)
                .expect("Cannot remove value from augmented assignment"),
        );
        stmt.target = Box::new(
            self.visit_expr(*stmt.target)
                .expect("Cannot remove target from augmented assignment"),
        );

        Some(stmt)
    }

    fn visit_stmt_while(&mut self, mut stmt: StmtWhile) -> Option<StmtWhile> {
        self.generic_visit_stmt_while(stmt)
    }

    fn generic_visit_stmt_while(&mut self, mut stmt: StmtWhile) -> Option<StmtWhile> {
        stmt.test = Box::new(
            self.visit_expr(*stmt.test)
                .expect("Cannot remove test from while statement"),
        );
        stmt.body = self.visit_stmt_vec(stmt.body);
        stmt.orelse = self.visit_stmt_vec(stmt.orelse);

        if stmt.body.is_empty() && stmt.orelse.is_empty() {
            return None;
        }

        Some(stmt)
    }

    fn visit_stmt_if(&mut self, mut stmt: StmtIf) -> Option<StmtIf> {
        self.generic_visit_stmt_if(stmt)
    }

    fn generic_visit_stmt_if(&mut self, mut stmt: StmtIf) -> Option<StmtIf> {
        stmt.test = Box::new(
            self.visit_expr(*stmt.test)
                .expect("Cannot remove test from if statement"),
        );
        stmt.body = self.visit_stmt_vec(stmt.body);
        stmt.orelse = self.visit_stmt_vec(stmt.orelse);

        if stmt.body.is_empty() && stmt.orelse.is_empty() {
            return None;
        }

        Some(stmt)
    }

    fn visit_pattern_match_or(&mut self, mut pattern: PatternMatchOr) -> Option<PatternMatchOr> {
        self.generic_visit_pattern_match_or(pattern)
    }

    fn generic_visit_pattern_match_or(
        &mut self,
        mut pattern: PatternMatchOr,
    ) -> Option<PatternMatchOr> {
        pattern.patterns = self.generic_visit_pattern_vec(pattern.patterns);
        if pattern.patterns.is_empty() {
            return None;
        }

        Some(pattern)
    }

    fn visit_pattern_match_as(&mut self, mut pattern: PatternMatchAs) -> Option<PatternMatchAs> {
        self.generic_visit_pattern_match_as(pattern)
    }

    fn generic_visit_pattern_match_as(
        &mut self,
        mut pattern: PatternMatchAs,
    ) -> Option<PatternMatchAs> {
        if let Some(inner_pattern) = pattern.pattern {
            pattern.pattern = self.visit_pattern(*inner_pattern).map(Box::new);
        }

        Some(pattern)
    }

    fn visit_pattern_match_mapping(
        &mut self,
        mut pattern: PatternMatchMapping,
    ) -> Option<PatternMatchMapping> {
        self.generic_visit_pattern_match_mapping(pattern)
    }

    fn generic_visit_pattern_match_mapping(
        &mut self,
        mut pattern: PatternMatchMapping,
    ) -> Option<PatternMatchMapping> {
        pattern.keys = self.visit_expr_vec(pattern.keys);
        pattern.patterns = self.generic_visit_pattern_vec(pattern.patterns);
        Some(pattern)
    }

    fn visit_pattern_match_star(
        &mut self,
        mut pattern: PatternMatchStar,
    ) -> Option<PatternMatchStar> {
        self.generic_visit_pattern_match_star(pattern)
    }

    fn generic_visit_pattern_match_star(
        &mut self,
        mut pattern: PatternMatchStar,
    ) -> Option<PatternMatchStar> {
        Some(pattern)
    }

    fn visit_pattern_match_class(
        &mut self,
        mut pattern: PatternMatchClass,
    ) -> Option<PatternMatchClass> {
        self.generic_visit_pattern_match_class(pattern)
    }

    fn generic_visit_pattern_match_class(
        &mut self,
        mut pattern: PatternMatchClass,
    ) -> Option<PatternMatchClass> {
        pattern.cls = Box::new(
            self.visit_expr(*pattern.cls)
                .expect("Cannot remove class from pattern match class"),
        );
        pattern.patterns = self.generic_visit_pattern_vec(pattern.patterns);
        pattern.kwd_patterns = self.generic_visit_pattern_vec(pattern.kwd_patterns);
        Some(pattern)
    }

    fn visit_pattern_match_sequence(
        &mut self,
        mut pattern: PatternMatchSequence,
    ) -> Option<PatternMatchSequence> {
        self.generic_visit_pattern_match_sequence(pattern)
    }

    fn generic_visit_pattern_match_sequence(
        &mut self,
        mut pattern: PatternMatchSequence,
    ) -> Option<PatternMatchSequence> {
        pattern.patterns = self.generic_visit_pattern_vec(pattern.patterns);
        if pattern.patterns.is_empty() {
            return None;
        }

        Some(pattern)
    }

    fn visit_pattern_match_singleton(
        &mut self,
        mut pattern: PatternMatchSingleton,
    ) -> Option<PatternMatchSingleton> {
        self.generic_visit_pattern_match_singleton(pattern)
    }

    fn generic_visit_pattern_match_singleton(
        &mut self,
        mut pattern: PatternMatchSingleton,
    ) -> Option<PatternMatchSingleton> {
        Some(pattern)
    }

    fn visit_pattern_match_value(
        &mut self,
        mut pattern: PatternMatchValue,
    ) -> Option<PatternMatchValue> {
        self.generic_visit_pattern_match_value(pattern)
    }

    fn generic_visit_pattern_match_value(
        &mut self,
        mut pattern: PatternMatchValue,
    ) -> Option<PatternMatchValue> {
        pattern.value = Box::new(
            self.visit_expr(*pattern.value)
                .expect("Cannot remove value from pattern match value"),
        );
        Some(pattern)
    }

    fn generic_visit_pattern_vec(&mut self, patterns: Vec<Pattern>) -> Vec<Pattern> {
        let mut new_patterns: Vec<Pattern> = Vec::new();
        for pattern in patterns {
            if let Some(new_pattern) = self.visit_pattern(pattern) {
                new_patterns.push(new_pattern);
            }
        }

        new_patterns
    }

    fn visit_pattern(&mut self, pattern: Pattern) -> Option<Pattern> {
        self.generic_visit_pattern(pattern)
    }

    fn generic_visit_pattern(&mut self, pattern: Pattern) -> Option<Pattern> {
        match pattern {
            Pattern::MatchValue(pattern_match_value) => self
                .visit_pattern_match_value(pattern_match_value)
                .map(Pattern::MatchValue),
            Pattern::MatchSingleton(pattern_match_singleton) => self
                .visit_pattern_match_singleton(pattern_match_singleton)
                .map(Pattern::MatchSingleton),
            Pattern::MatchSequence(pattern_match_sequence) => self
                .visit_pattern_match_sequence(pattern_match_sequence)
                .map(Pattern::MatchSequence),
            Pattern::MatchMapping(pattern_match_mapping) => self
                .visit_pattern_match_mapping(pattern_match_mapping)
                .map(Pattern::MatchMapping),
            Pattern::MatchClass(pattern_match_class) => self
                .visit_pattern_match_class(pattern_match_class)
                .map(Pattern::MatchClass),
            Pattern::MatchStar(pattern_match_star) => self
                .visit_pattern_match_star(pattern_match_star)
                .map(Pattern::MatchStar),
            Pattern::MatchAs(pattern_match_as) => self
                .visit_pattern_match_as(pattern_match_as)
                .map(Pattern::MatchAs),
            Pattern::MatchOr(pattern_match_or) => self
                .visit_pattern_match_or(pattern_match_or)
                .map(Pattern::MatchOr),
        }
    }

    fn generic_visit_match_case_vec(&mut self, cases: Vec<MatchCase>) -> Vec<MatchCase> {
        let mut new_cases: Vec<MatchCase> = Vec::new();
        for case in cases {
            if let Some(new_case) = self.visit_match_case(case) {
                new_cases.push(new_case);
            }
        }

        new_cases
    }

    fn visit_match_case(&mut self, mut case: MatchCase) -> Option<MatchCase> {
        self.generic_visit_match_case(case)
    }

    fn generic_visit_match_case(&mut self, mut case: MatchCase) -> Option<MatchCase> {
        case.pattern = self
            .visit_pattern(case.pattern)
            .expect("Cannot remove pattern from match case");
        if let Some(guard) = case.guard {
            case.guard = box_expr_option(self.visit_expr(*guard));
        }

        case.body = self.visit_stmt_vec(case.body);
        if case.body.is_empty() {
            return None;
        }
        Some(case)
    }

    fn visit_stmt_match(&mut self, mut stmt: StmtMatch) -> Option<StmtMatch> {
        self.generic_visit_stmt_match(stmt)
    }

    fn generic_visit_stmt_match(&mut self, mut stmt: StmtMatch) -> Option<StmtMatch> {
        stmt.subject = Box::new(
            self.visit_expr(*stmt.subject)
                .expect("Cannot remove subject from match statement"),
        );
        stmt.cases = self.generic_visit_match_case_vec(stmt.cases);
        if stmt.cases.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn generic_visit_except_handler_vec(
        &mut self,
        handlers: Vec<ExceptHandler>,
    ) -> Vec<ExceptHandler> {
        let mut new_handlers: Vec<ExceptHandler> = Vec::new();

        for handler in handlers {
            if let Some(new_handler) = self.visit_except_handler(handler) {
                new_handlers.push(new_handler);
            }
        }

        new_handlers
    }

    fn visit_except_handler(&mut self, mut handler: ExceptHandler) -> Option<ExceptHandler> {
        self.generic_visit_except_handler(handler)
    }

    fn generic_visit_except_handler(
        &mut self,
        mut handler: ExceptHandler,
    ) -> Option<ExceptHandler> {
        match handler {
            ExceptHandler::ExceptHandler(except_handler) => self
                .visit_except_handler_except_handler(except_handler)
                .map(ExceptHandler::ExceptHandler),
        }
    }

    fn visit_except_handler_except_handler(
        &mut self,
        mut except_handler: ExceptHandlerExceptHandler,
    ) -> Option<ExceptHandlerExceptHandler> {
        self.generic_visit_except_handler_except_handler(except_handler)
    }

    fn generic_visit_except_handler_except_handler(
        &mut self,
        mut except_handler: ExceptHandlerExceptHandler,
    ) -> Option<ExceptHandlerExceptHandler> {
        except_handler.body = self.visit_stmt_vec(except_handler.body);
        if except_handler.body.is_empty() {
            return None;
        }
        Some(except_handler)
    }

    fn visit_stmt_try(&mut self, mut stmt: StmtTry) -> Option<StmtTry> {
        self.generic_visit_stmt_try(stmt)
    }

    fn generic_visit_stmt_try(&mut self, mut stmt: StmtTry) -> Option<StmtTry> {
        stmt.body = self.visit_stmt_vec(stmt.body);
        stmt.finalbody = self.visit_stmt_vec(stmt.finalbody);
        stmt.handlers = self.generic_visit_except_handler_vec(stmt.handlers);
        stmt.orelse = self.visit_stmt_vec(stmt.orelse);

        if stmt.body.is_empty() {
            return None;
        }

        Some(stmt)
    }

    fn visit_stmt_try_star(&mut self, mut stmt: StmtTryStar) -> Option<StmtTryStar> {
        self.generic_visit_stmt_try_star(stmt)
    }

    fn generic_visit_stmt_try_star(&mut self, mut stmt: StmtTryStar) -> Option<StmtTryStar> {
        stmt.body = self.visit_stmt_vec(stmt.body);
        stmt.finalbody = self.visit_stmt_vec(stmt.finalbody);
        stmt.handlers = self.generic_visit_except_handler_vec(stmt.handlers);
        stmt.orelse = self.visit_stmt_vec(stmt.orelse);

        if stmt.body.is_empty() {
            return None;
        }

        Some(stmt)
    }

    fn generic_visit_alias_vec(&mut self, aliases: Vec<Alias>) -> Vec<Alias> {
        let mut new_aliases: Vec<Alias> = Vec::new();

        for alias in aliases {
            if let Some(new_alias) = self.visit_alias(alias) {
                new_aliases.push(new_alias);
            }
        }

        new_aliases
    }

    fn visit_alias(&mut self, mut alias: Alias) -> Option<Alias> {
        self.generic_visit_alias(alias)
    }

    fn generic_visit_alias(&mut self, mut alias: Alias) -> Option<Alias> {
        Some(alias)
    }

    fn visit_stmt_import(&mut self, mut stmt: StmtImport) -> Option<StmtImport> {
        self.generic_visit_stmt_import(stmt)
    }

    fn generic_visit_stmt_import(&mut self, mut stmt: StmtImport) -> Option<StmtImport> {
        stmt.names = self.generic_visit_alias_vec(stmt.names);

        if stmt.names.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_stmt_import_from(&mut self, mut stmt: StmtImportFrom) -> Option<StmtImportFrom> {
        self.generic_visit_stmt_import_from(stmt)
    }

    fn generic_visit_stmt_import_from(
        &mut self,
        mut stmt: StmtImportFrom,
    ) -> Option<StmtImportFrom> {
        stmt.names = self.generic_visit_alias_vec(stmt.names);

        if stmt.names.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_stmt_global(&mut self, mut stmt: StmtGlobal) -> Option<StmtGlobal> {
        self.generic_visit_stmt_global(stmt)
    }

    fn generic_visit_stmt_global(&mut self, mut stmt: StmtGlobal) -> Option<StmtGlobal> {
        if stmt.names.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_stmt_nonlocal(&mut self, mut stmt: StmtNonlocal) -> Option<StmtNonlocal> {
        self.generic_visit_stmt_nonlocal(stmt)
    }

    fn generic_visit_stmt_nonlocal(&mut self, mut stmt: StmtNonlocal) -> Option<StmtNonlocal> {
        if stmt.names.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_stmt_expr(&mut self, mut stmt: StmtExpr) -> Option<StmtExpr> {
        self.generic_visit_stmt_expr(stmt)
    }

    fn generic_visit_stmt_expr(&mut self, mut stmt: StmtExpr) -> Option<StmtExpr> {
        match self.visit_expr(*stmt.value) {
            Some(new_expr) => {
                stmt.value = Box::new(new_expr);
                Some(stmt)
            }
            None => None,
        }
    }

    fn visit_stmt_raise(&mut self, mut stmt: StmtRaise) -> Option<StmtRaise> {
        self.generic_visit_stmt_raise(stmt)
    }

    fn generic_visit_stmt_raise(&mut self, mut stmt: StmtRaise) -> Option<StmtRaise> {
        if let Some(exc) = stmt.exc {
            stmt.exc = box_expr_option(self.visit_expr(*exc));
        }

        if let Some(cause) = stmt.cause {
            stmt.cause = box_expr_option(self.visit_expr(*cause));
        }

        Some(stmt)
    }

    fn visit_stmt_return(&mut self, mut stmt: StmtReturn) -> Option<StmtReturn> {
        self.generic_visit_stmt_return(stmt)
    }

    fn generic_visit_stmt_return(&mut self, mut stmt: StmtReturn) -> Option<StmtReturn> {
        if let Some(value) = stmt.value {
            stmt.value = box_expr_option(self.visit_expr(*value));
        }

        Some(stmt)
    }

    fn visit_stmt_continue(&mut self, mut stmt: StmtContinue) -> Option<StmtContinue> {
        self.generic_visit_stmt_continue(stmt)
    }

    fn generic_visit_stmt_continue(&mut self, mut stmt: StmtContinue) -> Option<StmtContinue> {
        Some(stmt)
    }

    fn visit_stmt_pass(&mut self, mut stmt: StmtPass) -> Option<StmtPass> {
        self.generic_visit_stmt_pass(stmt)
    }

    fn generic_visit_stmt_pass(&mut self, mut stmt: StmtPass) -> Option<StmtPass> {
        Some(stmt)
    }

    fn visit_stmt_break(&mut self, mut stmt: StmtBreak) -> Option<StmtBreak> {
        self.generic_visit_stmt_break(stmt)
    }

    fn generic_visit_stmt_break(&mut self, mut stmt: StmtBreak) -> Option<StmtBreak> {
        Some(stmt)
    }

    fn visit_stmt_with(&mut self, mut stmt: StmtWith) -> Option<StmtWith> {
        self.generic_visit_stmt_with(stmt)
    }

    fn generic_visit_stmt_with(&mut self, mut stmt: StmtWith) -> Option<StmtWith> {
        stmt.items = self.generic_visit_with_item_vec(stmt.items);
        stmt.body = self.visit_stmt_vec(stmt.body);

        if stmt.body.is_empty() {
            return None;
        }

        Some(stmt)
    }

    fn visit_stmt_async_with(&mut self, mut stmt: StmtAsyncWith) -> Option<StmtAsyncWith> {
        self.generic_visit_stmt_async_with(stmt)
    }

    fn generic_visit_stmt_async_with(&mut self, mut stmt: StmtAsyncWith) -> Option<StmtAsyncWith> {
        stmt.items = self.generic_visit_with_item_vec(stmt.items);
        stmt.body = self.visit_stmt_vec(stmt.body);
        if stmt.body.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_stmt_function_def(&mut self, mut stmt: StmtFunctionDef) -> Option<StmtFunctionDef> {
        self.generic_visit_stmt_function_def(stmt)
    }

    fn generic_visit_stmt_function_def(
        &mut self,
        mut stmt: StmtFunctionDef,
    ) -> Option<StmtFunctionDef> {
        stmt.type_params = self.generic_visit_type_param_vec(stmt.type_params);
        stmt.decorator_list = self.visit_expr_vec(stmt.decorator_list);
        stmt.args = Box::new(self.visit_arguments(*stmt.args));
        if let Some(returns) = stmt.returns {
            stmt.returns = box_expr_option(self.visit_annotation(returns));
        }
        stmt.body = self.visit_stmt_vec(stmt.body);
        if stmt.body.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_stmt_async_function_def(
        &mut self,
        mut stmt: StmtAsyncFunctionDef,
    ) -> Option<StmtAsyncFunctionDef> {
        self.generic_visit_stmt_async_function_def(stmt)
    }

    fn generic_visit_stmt_async_function_def(
        &mut self,
        mut stmt: StmtAsyncFunctionDef,
    ) -> Option<StmtAsyncFunctionDef> {
        stmt.type_params = self.generic_visit_type_param_vec(stmt.type_params);
        stmt.decorator_list = self.visit_expr_vec(stmt.decorator_list);
        stmt.args = Box::new(self.visit_arguments(*stmt.args));
        if let Some(returns) = stmt.returns {
            stmt.returns = box_expr_option(self.visit_annotation(returns));
        }
        stmt.body = self.visit_stmt_vec(stmt.body);
        if stmt.body.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_stmt_for(&mut self, mut stmt: StmtFor) -> Option<StmtFor> {
        self.generic_visit_for(stmt)
    }

    fn generic_visit_for(&mut self, mut stmt: StmtFor) -> Option<StmtFor> {
        stmt.body = self.visit_stmt_vec(stmt.body);
        stmt.iter = Box::new(
            self.visit_expr(*stmt.iter)
                .expect("Cannot remove iter from async for"),
        );
        stmt.orelse = self.visit_stmt_vec(stmt.orelse);
        stmt.target = Box::new(
            self.visit_expr(*stmt.target)
                .expect("Cannot remove target from async for"),
        );
        if stmt.body.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_stmt_async_for(&mut self, mut stmt: StmtAsyncFor) -> Option<StmtAsyncFor> {
        self.generic_visit_async_for(stmt)
    }

    fn generic_visit_async_for(&mut self, mut stmt: StmtAsyncFor) -> Option<StmtAsyncFor> {
        stmt.body = self.visit_stmt_vec(stmt.body);
        stmt.iter = Box::new(
            self.visit_expr(*stmt.iter)
                .expect("Cannot remove iter from async for"),
        );
        stmt.orelse = self.visit_stmt_vec(stmt.orelse);
        stmt.target = Box::new(
            self.visit_expr(*stmt.target)
                .expect("Cannot remove target from async for"),
        );
        if stmt.body.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_stmt_ann_assign(&mut self, mut stmt: StmtAnnAssign) -> Option<StmtAnnAssign> {
        self.generic_visit_ann_assign(stmt)
    }

    fn generic_visit_ann_assign(&mut self, mut stmt: StmtAnnAssign) -> Option<StmtAnnAssign> {
        stmt.annotation = Box::new(
            self.visit_annotation(stmt.annotation)
                .expect("Cannot remove annotation from annotated assignment"),
        );

        stmt.target = Box::new(
            self.visit_expr(*stmt.target)
                .expect("Cannot remove target from annotated assignment"),
        );

        if let Some(value) = stmt.value {
            stmt.value = box_expr_option(self.visit_expr(*value));
        }

        Some(stmt)
    }

    fn visit_stmt_assert(&mut self, mut stmt: StmtAssert) -> Option<StmtAssert> {
        self.generic_visit_assert(stmt)
    }

    fn generic_visit_assert(&mut self, mut stmt: StmtAssert) -> Option<StmtAssert> {
        if let Some(msg) = stmt.msg {
            stmt.msg = box_expr_option(self.visit_expr(*msg));
        }

        stmt.test = Box::new(
            self.visit_expr(*stmt.test)
                .expect("Assertion test cannot be removed"),
        );

        Some(stmt)
    }

    fn visit_stmt_delete(&mut self, mut stmt: StmtDelete) -> Option<StmtDelete> {
        self.generic_visit_delete(stmt)
    }

    fn generic_visit_delete(&mut self, mut stmt: StmtDelete) -> Option<StmtDelete> {
        stmt.targets = self.visit_expr_vec(stmt.targets);
        if stmt.targets.is_empty() {
            return None;
        }
        Some(stmt)
    }

    fn visit_expr_vec(&mut self, exprs: Vec<Expr>) -> Vec<Expr> {
        let mut new_exprs: Vec<Expr> = Vec::new();

        for expr in exprs {
            if let Some(new_expr) = self.visit_expr(expr) {
                new_exprs.push(new_expr);
            }
        }

        new_exprs
    }

    fn visit_expr(&mut self, expr: Expr) -> Option<Expr> {
        self.generic_visit_expr(expr)
    }

    fn generic_visit_expr(&mut self, expr: Expr) -> Option<Expr> {
        match expr {
            Expr::BoolOp(expr_bool_op) => self.visit_expr_bool_op(expr_bool_op).map(Expr::BoolOp),
            Expr::NamedExpr(expr_named_expr) => self
                .visit_expr_named_expr(expr_named_expr)
                .map(Expr::NamedExpr),
            Expr::BinOp(expr_bin_op) => self.visit_expr_bin_op(expr_bin_op).map(Expr::BinOp),
            Expr::UnaryOp(expr_unary_op) => {
                self.visit_expr_unary_op(expr_unary_op).map(Expr::UnaryOp)
            }
            Expr::Lambda(expr_lambda) => self.visit_expr_lambda(expr_lambda).map(Expr::Lambda),
            Expr::IfExp(expr_if_exp) => self.visit_expr_if_exp(expr_if_exp).map(Expr::IfExp),
            Expr::Dict(expr_dict) => self.visit_expr_dict(expr_dict).map(Expr::Dict),
            Expr::Set(expr_set) => self.visit_expr_set(expr_set).map(Expr::Set),
            Expr::ListComp(expr_list_comp) => self
                .visit_expr_list_comp(expr_list_comp)
                .map(Expr::ListComp),
            Expr::SetComp(expr_set_comp) => {
                self.visit_expr_set_comp(expr_set_comp).map(Expr::SetComp)
            }
            Expr::DictComp(expr_dict_comp) => self
                .visit_expr_dict_comp(expr_dict_comp)
                .map(Expr::DictComp),
            Expr::GeneratorExp(expr_generator_exp) => self
                .visit_expr_generator_exp(expr_generator_exp)
                .map(Expr::GeneratorExp),
            Expr::Await(expr_await) => self.visit_expr_await(expr_await).map(Expr::Await),
            Expr::Yield(expr_yield) => self.visit_expr_yield(expr_yield).map(Expr::Yield),
            Expr::YieldFrom(expr_yield_from) => self
                .visit_expr_yield_from(expr_yield_from)
                .map(Expr::YieldFrom),
            Expr::Compare(expr_compare) => self.visit_expr_compare(expr_compare).map(Expr::Compare),
            Expr::Call(expr_call) => self.visit_expr_call(expr_call).map(Expr::Call),
            Expr::FormattedValue(expr_formatted_value) => self
                .visit_expr_formatted_value(expr_formatted_value)
                .map(Expr::FormattedValue),
            Expr::JoinedStr(expr_joined_str) => self
                .visit_expr_joined_str(expr_joined_str)
                .map(Expr::JoinedStr),
            Expr::Constant(expr_constant) => {
                self.visit_expr_constant(expr_constant).map(Expr::Constant)
            }
            Expr::Attribute(expr_attribute) => self
                .visit_expr_attribute(expr_attribute)
                .map(Expr::Attribute),
            Expr::Subscript(expr_subscript) => self
                .visit_expr_subscript(expr_subscript)
                .map(Expr::Subscript),
            Expr::Starred(expr_starred) => self.visit_expr_starred(expr_starred).map(Expr::Starred),
            Expr::Name(expr_name) => self.visit_expr_name(expr_name).map(Expr::Name),
            Expr::List(expr_list) => self.visit_expr_list(expr_list).map(Expr::List),
            Expr::Tuple(expr_tuple) => self.visit_expr_tuple(expr_tuple).map(Expr::Tuple),
            Expr::Slice(expr_slice) => self.visit_expr_slice(expr_slice).map(Expr::Slice),
        }
    }

    fn visit_expr_slice(&mut self, mut expr: ExprSlice) -> Option<ExprSlice> {
        self.generic_visit_expr_slice(expr)
    }

    fn generic_visit_expr_slice(&mut self, mut expr: ExprSlice) -> Option<ExprSlice> {
        if let Some(lower) = expr.lower {
            expr.lower = box_expr_option(self.visit_expr(*lower));
        }

        if let Some(upper) = expr.upper {
            expr.upper = box_expr_option(self.visit_expr(*upper));
        }

        if let Some(step) = expr.step {
            expr.step = box_expr_option(self.visit_expr(*step));
        }

        Some(expr)
    }

    fn visit_expr_tuple(&mut self, mut expr: ExprTuple) -> Option<ExprTuple> {
        self.generic_visit_expr_tuple(expr)
    }

    fn generic_visit_expr_tuple(&mut self, mut expr: ExprTuple) -> Option<ExprTuple> {
        expr.elts = self.visit_expr_vec(expr.elts);

        Some(expr)
    }

    fn visit_expr_list(&mut self, mut expr: ExprList) -> Option<ExprList> {
        self.generic_visit_expr_list(expr)
    }

    fn generic_visit_expr_list(&mut self, mut expr: ExprList) -> Option<ExprList> {
        expr.elts = self.visit_expr_vec(expr.elts);
        Some(expr)
    }

    fn visit_expr_name(&mut self, mut expr: ExprName) -> Option<ExprName> {
        self.generic_visit_expr_name(expr)
    }

    fn generic_visit_expr_name(&mut self, mut expr: ExprName) -> Option<ExprName> {
        Some(expr)
    }

    fn visit_expr_starred(&mut self, mut expr: ExprStarred) -> Option<ExprStarred> {
        self.generic_visit_expr_starred(expr)
    }

    fn generic_visit_expr_starred(&mut self, mut expr: ExprStarred) -> Option<ExprStarred> {
        expr.value = Box::new(
            self.visit_expr(*expr.value)
                .expect("Cannot remove value from starred expression"),
        );

        Some(expr)
    }

    fn visit_expr_subscript(&mut self, mut expr: ExprSubscript) -> Option<ExprSubscript> {
        self.generic_visit_expr_subscript(expr)
    }

    fn generic_visit_expr_subscript(&mut self, mut expr: ExprSubscript) -> Option<ExprSubscript> {
        expr.value = Box::new(
            self.visit_expr(*expr.value)
                .expect("Cannot remove value from subscript expression"),
        );
        expr.slice = Box::new(
            self.visit_expr(*expr.slice)
                .expect("Cannot remove slice from subscript expression"),
        );
        Some(expr)
    }

    fn visit_expr_attribute(&mut self, mut expr: ExprAttribute) -> Option<ExprAttribute> {
        self.generic_visit_expr_attribute(expr)
    }

    fn generic_visit_expr_attribute(&mut self, mut expr: ExprAttribute) -> Option<ExprAttribute> {
        expr.value = Box::new(
            self.visit_expr(*expr.value)
                .expect("Cannot remove value from attribute expression"),
        );
        Some(expr)
    }

    fn visit_expr_constant(&mut self, mut expr: ExprConstant) -> Option<ExprConstant> {
        self.generic_visit_expr_constant(expr)
    }

    fn generic_visit_expr_constant(&mut self, mut expr: ExprConstant) -> Option<ExprConstant> {
        Some(expr)
    }

    fn visit_expr_joined_str(&mut self, mut expr: ExprJoinedStr) -> Option<ExprJoinedStr> {
        self.generic_visit_expr_joined_str(expr)
    }

    fn generic_visit_expr_joined_str(&mut self, mut expr: ExprJoinedStr) -> Option<ExprJoinedStr> {
        expr.values = self.visit_expr_vec(expr.values);

        Some(expr)
    }

    fn visit_expr_formatted_value(
        &mut self,
        mut expr: ExprFormattedValue,
    ) -> Option<ExprFormattedValue> {
        self.generic_visit_expr_formatted_value(expr)
    }

    fn generic_visit_expr_formatted_value(
        &mut self,
        mut expr: ExprFormattedValue,
    ) -> Option<ExprFormattedValue> {
        expr.value = Box::new(
            self.visit_expr(*expr.value)
                .expect("Cannot remove value from formatted value expression"),
        );
        if let Some(format_spec) = expr.format_spec {
            expr.format_spec = box_expr_option(self.visit_expr(*format_spec));
        }

        Some(expr)
    }

    fn visit_expr_call(&mut self, mut expr: ExprCall) -> Option<ExprCall> {
        self.generic_visit_expr_call(expr)
    }

    fn generic_visit_expr_call(&mut self, mut expr: ExprCall) -> Option<ExprCall> {
        expr.func = Box::new(
            self.visit_expr(*expr.func)
                .expect("Cannot remove func from call expression"),
        );
        expr.args = self.visit_expr_vec(expr.args);
        expr.keywords = self.generic_visit_keyword_vec(expr.keywords);
        Some(expr)
    }

    fn visit_expr_compare(&mut self, mut expr: ExprCompare) -> Option<ExprCompare> {
        self.generic_visit_expr_compare(expr)
    }

    fn generic_visit_expr_compare(&mut self, mut expr: ExprCompare) -> Option<ExprCompare> {
        expr.left = Box::new(
            self.visit_expr(*expr.left)
                .expect("Cannot remove left from compare expression"),
        );
        expr.comparators = self.visit_expr_vec(expr.comparators);
        Some(expr)
    }

    fn visit_expr_yield_from(&mut self, mut expr: ExprYieldFrom) -> Option<ExprYieldFrom> {
        self.generic_visit_expr_yield_from(expr)
    }

    fn generic_visit_expr_yield_from(&mut self, mut expr: ExprYieldFrom) -> Option<ExprYieldFrom> {
        expr.value = Box::new(
            self.visit_expr(*expr.value)
                .expect("Cannot remove value from yield from expression"),
        );
        Some(expr)
    }

    fn visit_expr_yield(&mut self, mut expr: ExprYield) -> Option<ExprYield> {
        self.generic_visit_expr_yield(expr)
    }

    fn generic_visit_expr_yield(&mut self, mut expr: ExprYield) -> Option<ExprYield> {
        if let Some(value) = expr.value {
            expr.value = box_expr_option(self.visit_expr(*value));
        }

        Some(expr)
    }

    fn visit_expr_await(&mut self, mut expr: ExprAwait) -> Option<ExprAwait> {
        self.generic_visit_expr_await(expr)
    }

    fn generic_visit_expr_await(&mut self, mut expr: ExprAwait) -> Option<ExprAwait> {
        match self.visit_expr(*expr.value) {
            Some(new_value) => {
                expr.value = Box::new(new_value);
                Some(expr)
            }
            None => None,
        }
    }

    fn generic_visit_comprehension_vec(&mut self, comps: Vec<Comprehension>) -> Vec<Comprehension> {
        let mut new_comps: Vec<Comprehension> = Vec::new();

        for comp in comps {
            if let Some(new_comp) = self.visit_comprehension(comp) {
                new_comps.push(new_comp);
            }
        }
        new_comps
    }

    fn visit_comprehension(&mut self, mut comp: Comprehension) -> Option<Comprehension> {
        self.generic_visit_comprehension(comp)
    }

    fn generic_visit_comprehension(&mut self, mut comp: Comprehension) -> Option<Comprehension> {
        comp.ifs = self.visit_expr_vec(comp.ifs);
        comp.iter = self
            .visit_expr(comp.iter)
            .expect("Cannot remove iter from comprehension");
        comp.target = self
            .visit_expr(comp.target)
            .expect("Cannot remove target from comprehension");

        Some(comp)
    }

    fn visit_expr_generator_exp(&mut self, mut expr: ExprGeneratorExp) -> Option<ExprGeneratorExp> {
        self.generic_visit_expr_generator_expr(expr)
    }

    fn generic_visit_expr_generator_expr(
        &mut self,
        mut expr: ExprGeneratorExp,
    ) -> Option<ExprGeneratorExp> {
        expr.elt = Box::new(
            self.visit_expr(*expr.elt)
                .expect("Cannot remove elt from generator expression"),
        );
        expr.generators = self.generic_visit_comprehension_vec(expr.generators);
        Some(expr)
    }

    fn visit_expr_dict_comp(&mut self, mut expr: ExprDictComp) -> Option<ExprDictComp> {
        self.generic_visit_expr_dict_comp(expr)
    }

    fn generic_visit_expr_dict_comp(&mut self, mut expr: ExprDictComp) -> Option<ExprDictComp> {
        expr.key = Box::new(
            self.visit_expr(*expr.key)
                .expect("Cannot remove key from dict comprehension"),
        );
        expr.value = Box::new(
            self.visit_expr(*expr.value)
                .expect("Cannot remove value from dict comprehension"),
        );
        expr.generators = self.generic_visit_comprehension_vec(expr.generators);
        Some(expr)
    }

    fn visit_expr_set_comp(&mut self, mut expr: ExprSetComp) -> Option<ExprSetComp> {
        self.generic_visit_expr_set_comp(expr)
    }

    fn generic_visit_expr_set_comp(&mut self, mut expr: ExprSetComp) -> Option<ExprSetComp> {
        expr.elt = Box::new(
            self.visit_expr(*expr.elt)
                .expect("Cannot remove elt from set comprehension"),
        );
        expr.generators = self.generic_visit_comprehension_vec(expr.generators);
        Some(expr)
    }

    fn visit_expr_list_comp(&mut self, mut expr: ExprListComp) -> Option<ExprListComp> {
        self.generic_visit_expr_list_comp(expr)
    }

    fn generic_visit_expr_list_comp(&mut self, mut expr: ExprListComp) -> Option<ExprListComp> {
        expr.elt = Box::new(
            self.visit_expr(*expr.elt)
                .expect("Cannot remove elt from list comprehension"),
        );
        expr.generators = self.generic_visit_comprehension_vec(expr.generators);
        Some(expr)
    }

    fn visit_expr_set(&mut self, mut expr: ExprSet) -> Option<ExprSet> {
        self.generic_visit_expr_set(expr)
    }

    fn generic_visit_expr_set(&mut self, mut expr: ExprSet) -> Option<ExprSet> {
        expr.elts = self.visit_expr_vec(expr.elts);
        Some(expr)
    }

    fn visit_expr_dict(&mut self, mut expr: ExprDict) -> Option<ExprDict> {
        self.generic_visit_expr_dict(expr)
    }

    fn generic_visit_expr_dict(&mut self, mut expr: ExprDict) -> Option<ExprDict> {
        let mut new_keys: Vec<Option<Expr>> = Vec::new();
        for key in expr.keys {
            if let Some(key_value) = key {
                new_keys.push(self.visit_expr(key_value));
            }
        }
        expr.keys = new_keys;
        expr.values = self.visit_expr_vec(expr.values);
        Some(expr)
    }

    fn visit_expr_if_exp(&mut self, mut expr: ExprIfExp) -> Option<ExprIfExp> {
        self.generic_visit_expr_if_exp(expr)
    }

    fn generic_visit_expr_if_exp(&mut self, mut expr: ExprIfExp) -> Option<ExprIfExp> {
        expr.test = Box::new(
            self.visit_expr(*expr.test)
                .expect("Cannot remove test from if expression"),
        );
        expr.body = Box::new(
            self.visit_expr(*expr.body)
                .expect("Cannot remove body from if expression"),
        );
        expr.orelse = Box::new(
            self.visit_expr(*expr.orelse)
                .expect("Cannot remove orelse from if expression"),
        );
        Some(expr)
    }

    fn visit_expr_lambda(&mut self, mut expr: ExprLambda) -> Option<ExprLambda> {
        self.generic_visit_expr_lambda(expr)
    }

    fn generic_visit_expr_lambda(&mut self, mut expr: ExprLambda) -> Option<ExprLambda> {
        expr.args = Box::new(self.visit_arguments(*expr.args));
        expr.body = Box::new(
            self.visit_expr(*expr.body)
                .expect("Cannot remove body from lambda expression"),
        );
        Some(expr)
    }

    fn visit_expr_unary_op(&mut self, mut expr: ExprUnaryOp) -> Option<ExprUnaryOp> {
        self.generic_visit_expr_unary_op(expr)
    }

    fn generic_visit_expr_unary_op(&mut self, mut expr: ExprUnaryOp) -> Option<ExprUnaryOp> {
        expr.operand = Box::new(
            self.visit_expr(*expr.operand)
                .expect("Cannot remove operand from unary operation"),
        );
        Some(expr)
    }

    fn visit_expr_bin_op(&mut self, mut expr: ExprBinOp) -> Option<ExprBinOp> {
        self.generic_visit_expr_bin_op(expr)
    }

    fn generic_visit_expr_bin_op(&mut self, mut expr: ExprBinOp) -> Option<ExprBinOp> {
        expr.left = Box::new(
            self.visit_expr(*expr.left)
                .expect("Cannot remove left from binary operation"),
        );
        expr.right = Box::new(
            self.visit_expr(*expr.right)
                .expect("Cannot remove right from binary operation"),
        );
        Some(expr)
    }

    fn visit_expr_named_expr(&mut self, mut expr: ExprNamedExpr) -> Option<ExprNamedExpr> {
        self.generic_visit_expr_named_expr(expr)
    }

    fn generic_visit_expr_named_expr(&mut self, mut expr: ExprNamedExpr) -> Option<ExprNamedExpr> {
        expr.target = Box::new(
            self.visit_expr(*expr.target)
                .expect("Cannot remove target from named expression"),
        );
        expr.value = Box::new(
            self.visit_expr(*expr.value)
                .expect("Cannot remove value from named expression"),
        );
        Some(expr)
    }

    fn visit_expr_bool_op(&mut self, mut expr: ExprBoolOp) -> Option<ExprBoolOp> {
        self.generic_visit_expr_bool_op(expr)
    }

    fn generic_visit_expr_bool_op(&mut self, mut expr: ExprBoolOp) -> Option<ExprBoolOp> {
        expr.values = self.visit_expr_vec(expr.values);
        if expr.values.is_empty() {
            panic!("Cannot remove all values from bool op");
        }
        Some(expr)
    }

    fn visit_arg(&mut self, arg: Arg) -> Option<Arg> {
        self.generic_visit_arg(arg)
    }

    fn generic_visit_arg(&mut self, mut arg: Arg) -> Option<Arg> {
        if let Some(annotation) = arg.annotation {
            arg.annotation = box_expr_option(self.visit_annotation(annotation));
        }
        Some(arg)
    }

    fn visit_arg_with_default(&mut self, mut arg: ArgWithDefault) -> Option<ArgWithDefault> {
        self.generic_visit_arg_with_default(arg)
    }

    fn generic_visit_arg_with_default(
        &mut self,
        mut arg: ArgWithDefault,
    ) -> Option<ArgWithDefault> {
        arg.def = self
            .visit_arg(arg.def)
            .expect("Cannot remove def from arg with default");
        if let Some(default) = arg.default {
            arg.default = box_expr_option(self.visit_expr(*default));
        }

        Some(arg)
    }

    fn generic_visit_args_with_default_vec(
        &mut self,
        mut node: Vec<ArgWithDefault>,
    ) -> Vec<ArgWithDefault> {
        let mut new_nodes: Vec<ArgWithDefault> = Vec::new();

        for arg in node {
            if let Some(new_arg) = self.visit_arg_with_default(arg) {
                new_nodes.push(new_arg);
            }
        }
        new_nodes
    }

    fn visit_arguments(&mut self, mut arguments: Arguments) -> Arguments {
        self.generic_visit_arguments(arguments)
    }

    fn generic_visit_arguments(&mut self, mut arguments: Arguments) -> Arguments {
        arguments.args = self.generic_visit_args_with_default_vec(arguments.args);
        if let Some(kwarg) = arguments.kwarg {
            arguments.kwarg = self.visit_arg(*kwarg).map(Box::new);
        }
        arguments.kwonlyargs = self.generic_visit_args_with_default_vec(arguments.kwonlyargs);
        arguments.posonlyargs = self.generic_visit_args_with_default_vec(arguments.posonlyargs);
        if let Some(vararg) = arguments.vararg {
            arguments.vararg = self.visit_arg(*vararg).map(Box::new);
        }
        arguments
    }

    fn generic_visit_type_param_vec(&mut self, mut params: Vec<TypeParam>) -> Vec<TypeParam> {
        let mut new_params: Vec<TypeParam> = Vec::new();
        for param in params {
            if let Some(new_param) = self.visit_type_param(param) {
                new_params.push(new_param);
            }
        }
        new_params
    }

    fn visit_type_param(&mut self, mut param: TypeParam) -> Option<TypeParam> {
        self.generic_visit_type_param(param)
    }

    fn generic_visit_type_param(&mut self, mut param: TypeParam) -> Option<TypeParam> {
        match param {
            TypeParam::ParamSpec(param_spec) => self
                .visit_type_param_spec(param_spec)
                .map(TypeParam::ParamSpec),
            TypeParam::TypeVar(param_var) => {
                self.visit_type_param_var(param_var).map(TypeParam::TypeVar)
            }
            TypeParam::TypeVarTuple(param_var_tuple) => self
                .visit_type_param_var_tuple(param_var_tuple)
                .map(TypeParam::TypeVarTuple),
        }
    }

    fn visit_type_param_spec(
        &mut self,
        mut param_spec: TypeParamParamSpec,
    ) -> Option<TypeParamParamSpec> {
        self.generic_visit_type_param_spec(param_spec)
    }

    fn generic_visit_type_param_spec(
        &mut self,
        mut param_spec: TypeParamParamSpec,
    ) -> Option<TypeParamParamSpec> {
        Some(param_spec)
    }

    fn visit_type_param_var(
        &mut self,
        mut param_var: TypeParamTypeVar,
    ) -> Option<TypeParamTypeVar> {
        self.generic_visit_type_param_var(param_var)
    }

    fn generic_visit_type_param_var(
        &mut self,
        mut param_var: TypeParamTypeVar,
    ) -> Option<TypeParamTypeVar> {
        if let Some(bound) = param_var.bound {
            param_var.bound = box_expr_option(self.visit_annotation(bound));
        }
        Some(param_var)
    }

    fn visit_type_param_var_tuple(
        &mut self,
        mut param_var_tuple: TypeParamTypeVarTuple,
    ) -> Option<TypeParamTypeVarTuple> {
        self.generic_visit_type_param_var_tuple(param_var_tuple)
    }

    fn generic_visit_type_param_var_tuple(
        &mut self,
        mut param_var_tuple: TypeParamTypeVarTuple,
    ) -> Option<TypeParamTypeVarTuple> {
        Some(param_var_tuple)
    }

    fn generic_visit_with_item_vec(&mut self, with_items: Vec<WithItem>) -> Vec<WithItem> {
        let mut new_with_items: Vec<WithItem> = Vec::new();

        for with_item in with_items {
            if let Some(new_with_item) = self.visit_with_item(with_item) {
                new_with_items.push(new_with_item);
            }
        }

        new_with_items
    }

    fn visit_with_item(&mut self, mut with_item: WithItem) -> Option<WithItem> {
        self.generic_visit_with_item(with_item)
    }

    fn generic_visit_with_item(&mut self, mut with_item: WithItem) -> Option<WithItem> {
        with_item.context_expr = self
            .visit_expr(with_item.context_expr)
            .expect("Cannot remove context expr from with item");
        if let Some(optional_vars) = with_item.optional_vars {
            with_item.optional_vars = box_expr_option(self.visit_expr(*optional_vars));
        }
        Some(with_item)
    }
}
