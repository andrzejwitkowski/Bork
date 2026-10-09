//! Pre-codegen allocation planning for record objects that can be addressed by `Ref<T>`.

use std::collections::{HashMap, HashSet};

use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirProgram, HirStmt, Ty};
use crate::region_walk::{
    self, ArenaCursor, RegionSite, RegionVisitor, WalkDriver, WalkError,
};
use crate::sema::{ArenaNode, ArenaReport};
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArenaClass {
    Lexical,
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifetimeDomain {
    Program,
    LexicalScope(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaPlan {
    pub allocation_site: Span,
    pub class_name: String,
    pub class: ArenaClass,
    pub lifetime_domain: LifetimeDomain,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPlan {
    pub allocations: Vec<ArenaPlan>,
}

impl MemoryPlan {
    pub fn allocation_at(&self, site: Span) -> Option<&ArenaPlan> {
        self.allocations
            .iter()
            .find(|allocation| allocation.allocation_site == site)
    }
}

#[derive(Debug, Clone, Default)]
struct Origins(HashSet<Span>);

impl Origins {
    fn one(site: Span) -> Self {
        Self(HashSet::from([site]))
    }

    fn extend(&mut self, other: &Self) {
        self.0.extend(other.0.iter().copied());
    }
}

pub fn plan(program: &HirProgram, report: &ArenaReport) -> (MemoryPlan, Vec<Diagnostic>) {
    let mut planner = Planner::default();
    if let Err(error) = region_walk::walk_program_readonly(program, &report.roots, &mut planner) {
        planner.diagnostics.push(Diagnostic {
            phase: Phase::Ownership,
            severity: Severity::Error,
            message: format!("cannot build memory plan: {}", error.as_str()),
            span: None,
        });
    }
    planner.propagate_program_lifetimes();
    planner
        .allocations
        .sort_by_key(|allocation| allocation.allocation_site.start);
    (
        MemoryPlan {
            allocations: planner.allocations,
        },
        planner.diagnostics,
    )
}

#[derive(Default)]
struct Planner {
    scope_ids: Vec<usize>,
    scopes: Vec<HashMap<String, Origins>>,
    expression_origins: HashMap<usize, Origins>,
    field_origins: HashMap<(Span, usize), Origins>,
    allocation_indices: HashMap<Span, usize>,
    allocations: Vec<ArenaPlan>,
    edges: Vec<(Span, Span)>,
    diagnostics: Vec<Diagnostic>,
}

impl Planner {
    fn expr_key(expr: &HirExpr) -> usize {
        expr as *const HirExpr as usize
    }

    fn origins(&self, expr: &HirExpr) -> Origins {
        self.expression_origins
            .get(&Self::expr_key(expr))
            .cloned()
            .unwrap_or_default()
    }

    fn remember(&mut self, expr: &HirExpr, origins: Origins) {
        self.expression_origins.insert(Self::expr_key(expr), origins);
    }

    fn lookup(&self, name: &str) -> Origins {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .cloned()
            .unwrap_or_default()
    }

    fn bind(&mut self, name: &str, origins: Origins) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_owned(), origins);
        }
    }

    fn assign(&mut self, name: &str, mut origins: Origins) {
        if let Some(scope) = self.scopes.iter().rposition(|scope| scope.contains_key(name)) {
            // ponytail: keep all possible assignments; branch-sensitive joins can narrow this later.
            origins.extend(&self.scopes[scope][name]);
            if scope + 1 < self.scopes.len() {
                self.mark_program(&origins);
            }
            self.scopes[scope].insert(name.to_owned(), origins);
        }
    }

    fn add_allocation(&mut self, expr: &HirExpr, class_name: &str) -> Origins {
        let Some(site) = expr.span else {
            self.diagnostics.push(Diagnostic {
                phase: Phase::Ownership,
                severity: Severity::Error,
                message: "record allocation has no source location".into(),
                span: None,
            });
            return Origins::default();
        };
        if !self.allocation_indices.contains_key(&site) {
            let scope = self.scope_ids.last().copied().unwrap_or(0);
            self.allocation_indices.insert(site, self.allocations.len());
            self.allocations.push(ArenaPlan {
                allocation_site: site,
                class_name: class_name.to_owned(),
                class: ArenaClass::Lexical,
                lifetime_domain: LifetimeDomain::LexicalScope(scope),
            });
        }
        Origins::one(site)
    }

    fn mark_dynamic(&mut self, origins: &Origins) {
        for site in &origins.0 {
            let Some(index) = self.allocation_indices.get(site).copied() else {
                continue;
            };
            let allocation = &mut self.allocations[index];
            if allocation.class == ArenaClass::Dynamic {
                continue;
            }
            allocation.class = ArenaClass::Dynamic;
        }
    }

    fn mark_program(&mut self, origins: &Origins) {
        self.mark_dynamic(origins);
        for site in &origins.0 {
            if let Some(index) = self.allocation_indices.get(site).copied() {
                let allocation = &mut self.allocations[index];
                allocation.lifetime_domain = LifetimeDomain::Program;
            }
        }
    }

    fn add_ref_edges(&mut self, sources: &Origins, targets: &Origins) {
        self.mark_dynamic(targets);
        if sources.0.is_empty() {
            self.mark_program(targets);
            return;
        }
        for source in &sources.0 {
            for target in &targets.0 {
                self.edges.push((*source, *target));
            }
        }
    }

    fn is_program(&self, site: Span) -> bool {
        self.allocation_indices.get(&site).is_some_and(|index| {
            self.allocations[*index].lifetime_domain == LifetimeDomain::Program
        })
    }

    fn propagate_program_lifetimes(&mut self) {
        loop {
            let promoted: Vec<_> = self
                .edges
                .iter()
                .filter_map(|(source, target)| {
                    (self.is_program(*source) && !self.is_program(*target)).then_some(*target)
                })
                .collect();
            if promoted.is_empty() {
                break;
            }
            for target in promoted {
                self.mark_program(&Origins::one(target));
            }
        }
    }

    fn combine<'a>(&self, expressions: impl IntoIterator<Item = &'a HirExpr>) -> Origins {
        let mut combined = Origins::default();
        for expression in expressions {
            combined.extend(&self.origins(expression));
        }
        combined
    }

    fn block_origins(&self, block: &HirBlock) -> Origins {
        let (body, _) = crate::hir::peel_blocks(block);
        match body.stmts.last() {
            Some(HirStmt::Expr(value)) => self.origins(value),
            _ => Origins::default(),
        }
    }
}

impl RegionVisitor for Planner {
    fn begin_function_body(
        &mut self,
        _name: &str,
        root: &ArenaNode,
        _body: &HirBlock,
    ) -> Result<(), WalkError> {
        self.scope_ids.push(root.id);
        self.scopes.push(HashMap::new());
        Ok(())
    }

    fn end_function_body(&mut self) -> Result<(), WalkError> {
        self.scopes.pop();
        self.scope_ids.pop();
        Ok(())
    }

    fn enter_region(
        &mut self,
        _site: RegionSite,
        child: &ArenaNode,
        _body: &HirBlock,
    ) -> Result<(), WalkError> {
        self.scope_ids.push(child.id);
        self.scopes.push(HashMap::new());
        Ok(())
    }

    fn loop_latch(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        Ok(())
    }

    fn exit_region(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        self.scopes.pop();
        self.scope_ids.pop();
        Ok(())
    }

    fn skip_closure(&mut self, _child: &ArenaNode) -> Result<(), WalkError> {
        Ok(())
    }

    fn presence_match<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        bindings: &[crate::hir::HirConditionalBinding],
        some_block: &HirBlock,
        none_block: Option<&HirBlock>,
        _result_ty: &Ty,
    ) -> Result<(), WalkError> {
        let (first, rest) = bindings
            .split_first()
            .expect("nonempty conditional bindings");
        driver.walk_expr(self, &first.value)?;
        let origins = self.origins(&first.value);
        region_walk::region_enter(driver, self, RegionSite::PresenceSome, some_block)?;
        self.bind(&first.name.name, origins);
        for binding in rest {
            driver.walk_expr(self, &binding.value)?;
            self.bind(&binding.name.name, self.origins(&binding.value));
        }
        let (body, _) = crate::hir::peel_blocks(some_block);
        driver.walk_block(self, body, None)?;
        region_walk::region_exit(driver, self, RegionSite::PresenceSome)?;
        if let Some(block) = none_block {
            driver.walk_region(self, RegionSite::PresenceNone, block)?;
        }
        Ok(())
    }

    fn on_stmt<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        stmt: &HirStmt,
    ) -> Result<(), WalkError> {
        match stmt {
            HirStmt::VarDecl { name, value, .. } => {
                driver.walk_expr(self, value)?;
                self.bind(name, self.origins(value));
            }
            HirStmt::Assign { target, value } => {
                if let Some(index) = target.index() {
                    driver.walk_expr(self, index)?;
                }
                if let Some(receiver) = target.receiver() {
                    driver.walk_expr(self, receiver)?;
                }
                driver.walk_expr(self, value)?;
                let value_origins = self.origins(value);
                match target {
                    crate::hir::HirAssignTarget::Name { name } => {
                        self.assign(name, value_origins);
                    }
                    crate::hir::HirAssignTarget::Field {
                        receiver,
                        field_index,
                        ..
                    } if value.ty.is_managed_ref() => {
                        let receivers = self.origins(receiver);
                        self.add_ref_edges(&receivers, &value_origins);
                        for source in receivers.0 {
                            self.field_origins
                                .entry((source, *field_index))
                                .or_default()
                                .extend(&value_origins);
                        }
                    }
                    crate::hir::HirAssignTarget::Field { .. }
                        if value
                            .ty
                            .array_elem()
                            .is_some_and(Ty::is_managed_ref) =>
                    {
                        self.mark_program(&value_origins);
                    }
                    crate::hir::HirAssignTarget::Index { name, .. }
                        if value.ty.is_managed_ref() =>
                    {
                        let mut origins = self.lookup(name);
                        origins.extend(&value_origins);
                        self.mark_program(&origins);
                        self.assign(name, origins);
                    }
                    _ => {}
                }
            }
            HirStmt::Return { value: Some(value) } => {
                driver.walk_expr(self, value)?;
                if value.ty.is_managed_ref() {
                    self.mark_program(&self.origins(value));
                }
            }
            HirStmt::Expr(value) => driver.walk_expr(self, value)?,
            HirStmt::Return { value: None }
            | HirStmt::Break { .. }
            | HirStmt::Continue { .. } => {}
            HirStmt::Block(_)
            | HirStmt::MoveBlock { .. }
            | HirStmt::For { .. }
            | HirStmt::While { .. } => unreachable!("region statements are dispatched by walker"),
        }
        Ok(())
    }

    fn after_expr<C: ArenaCursor>(
        &mut self,
        _driver: &mut WalkDriver<'_, C>,
        expr: &HirExpr,
    ) -> Result<(), WalkError> {
        let origins = match &expr.kind {
            HirExprKind::ObjectConstruct { class_name, fields } => {
                let allocation = self.add_allocation(expr, class_name);
                for (field_index, field) in fields.iter().enumerate() {
                    if field.ty.is_managed_ref() {
                        let targets = self.origins(field);
                        self.add_ref_edges(&allocation, &targets);
                        for source in &allocation.0 {
                            self.field_origins
                                .entry((*source, field_index))
                                .or_default()
                                .extend(&targets);
                        }
                    }
                }
                allocation
            }
            HirExprKind::Ident { name, .. } => self.lookup(name),
            HirExprKind::RefCreate(inner) => {
                let origins = self.origins(inner);
                self.mark_dynamic(&origins);
                origins
            }
            HirExprKind::Some(inner)
            | HirExprKind::Unary { expr: inner, .. }
            | HirExprKind::Field {
                receiver: inner, ..
            } => self.origins(inner),
            HirExprKind::ObjectField {
                receiver,
                field_index,
                ..
            } if expr.ty.is_managed_ref() => {
                let mut origins = Origins::default();
                for source in self.origins(receiver).0 {
                    if let Some(field) = self.field_origins.get(&(source, *field_index)) {
                        origins.extend(field);
                    }
                }
                origins
            }
            HirExprKind::ObjectField { receiver, .. }
            | HirExprKind::Index { receiver, .. }
            | HirExprKind::Slice { receiver, .. } => self.origins(receiver),
            HirExprKind::ArrayLit { elements } => {
                let origins = self.combine(elements);
                if expr
                    .ty
                    .array_elem()
                    .is_some_and(Ty::is_managed_ref)
                {
                    // ponytail: promote all Ref-array origins; destination-aware escape analysis
                    // can narrow this once field/call summaries exist.
                    self.mark_program(&origins);
                }
                origins
            }
            HirExprKind::Binary { lhs, rhs, .. } => self.combine([lhs.as_ref(), rhs.as_ref()]),
            HirExprKind::Call { args, .. } => {
                for argument in args
                    .iter()
                    .filter(|argument| {
                        argument.ty.record_name().is_some() || argument.ty.is_managed_ref()
                    })
                {
                    // ponytail: calls have no escape summaries, so record arguments are program-lived.
                    self.mark_program(&self.origins(argument));
                }
                Origins::default()
            }
            HirExprKind::If {
                then_block,
                else_block,
                ..
            }
            | HirExprKind::PresenceMatch {
                some_block: then_block,
                none_block: else_block,
                ..
            } => {
                let mut origins = self.block_origins(then_block);
                if let Some(block) = else_block {
                    origins.extend(&self.block_origins(block));
                }
                if expr.ty.is_managed_ref() || expr.ty.record_name().is_some() {
                    self.mark_program(&origins);
                }
                origins
            }
            HirExprKind::Int { .. }
            | HirExprKind::Float { .. }
            | HirExprKind::Bool { .. }
            | HirExprKind::Str { .. }
            | HirExprKind::None => Origins::default(),
        };
        self.remember(expr, origins);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_of(source: &str) -> MemoryPlan {
        let checked = crate::frontend::check(source);
        assert!(checked.is_ok(), "{:?}", checked.diagnostics);
        let (plan, diagnostics) = plan(
            checked.hir.as_ref().expect("typed HIR"),
            checked.report.as_ref().expect("arena report"),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        plan
    }

    #[test]
    fn chained_header_propagates_origins_through_dependent_fields() {
        let plan = plan_of("class Node { next: Ref<Node> }\nfun main() {\n val parent = Node()\n val child = Node()\n parent.next = child\n val r: Ref<Node> = parent\n if val (a = r, b = a.next) { b.next = r }\n}");
        assert_eq!(plan.allocations.len(), 2);
        assert!(plan
            .allocations
            .iter()
            .all(|a| a.class == ArenaClass::Dynamic));
    }

    #[test]
    fn ordinary_record_stays_in_the_current_lexical_arena() {
        let plan = plan_of(
            r#"
class Node { name: String }
fun main() { val node = Node("local") }
"#,
        );

        assert_eq!(plan.allocations.len(), 1);
        let allocation = &plan.allocations[0];
        assert_eq!(allocation.class, ArenaClass::Lexical);
        assert!(matches!(
            allocation.lifetime_domain,
            LifetimeDomain::LexicalScope(_)
        ));
    }

    #[test]
    fn returned_ref_target_is_dynamic_under_the_domain_root() {
        let plan = plan_of(
            r#"
class Node { name: String }
fun make(): Ref<Node> {
    val node = Node("escaping")
    return node
}
fun main() {}
"#,
        );

        let allocation = plan
            .allocations
            .iter()
            .find(|allocation| allocation.class_name == "Node")
            .expect("Node allocation");
        assert_eq!(allocation.class, ArenaClass::Dynamic);
        assert_eq!(allocation.lifetime_domain, LifetimeDomain::Program);
    }

    #[test]
    fn confined_ref_target_uses_the_lexical_parent() {
        let plan = plan_of(
            r#"
class Node { name: String }
fun main() {
    val node = Node("scoped")
    val reference: Ref<Node> = node
    if val live = reference { live.name }
}
"#,
        );

        let allocation = &plan.allocations[0];
        assert_eq!(allocation.class, ArenaClass::Dynamic);
        assert!(matches!(
            allocation.lifetime_domain,
            LifetimeDomain::LexicalScope(_)
        ));
    }

    #[test]
    fn escaping_ref_owner_promotes_captured_targets_transitively() {
        let plan = plan_of(
            r#"
class Node {
    name: String
    next: Ref<Node>
}
fun make(): Ref<Node> {
    val target = Node("target")
    val owner = Node("owner", target)
    return owner
}
fun main() {}
"#,
        );

        assert_eq!(plan.allocations.len(), 2);
        assert!(plan.allocations.iter().all(|allocation| {
            allocation.class == ArenaClass::Dynamic
                && allocation.lifetime_domain == LifetimeDomain::Program
        }));
    }

    #[test]
    fn managed_ref_passed_to_a_call_is_conservatively_program_lived() {
        let plan = plan_of(
            r#"
class Node { name: String }
fun consume(node: Ref<Node>) {}
fun main() {
    val node = Node("unknown escape")
    consume(node)
}
"#,
        );

        assert_eq!(plan.allocations[0].lifetime_domain, LifetimeDomain::Program);
    }

    #[test]
    fn record_passed_to_a_call_is_conservatively_program_lived() {
        let plan = plan_of(
            r#"
class Node { name: String }
fun consume(node: Node) {}
fun main() {
    val node = Node("unknown escape")
    consume(node)
}
"#,
        );

        assert_eq!(plan.allocations[0].lifetime_domain, LifetimeDomain::Program);
    }

    #[test]
    fn record_argument_returned_as_ref_is_conservatively_program_lived() {
        let plan = plan_of(
            r#"
class Node { name: String }
fun keep(node: Node): Ref<Node> { return node }
fun main() {
    val node = Node("returned escape")
    val result = keep(node)
}
"#,
        );

        let allocation = &plan.allocations[0];
        assert_eq!(allocation.class, ArenaClass::Dynamic);
        assert_eq!(allocation.lifetime_domain, LifetimeDomain::Program);
    }

    #[test]
    fn ref_array_origins_are_conservatively_program_lived() {
        let plan = plan_of(
            r#"
class Node { name: String }
fun main() {
    val node = Node("array escape")
    val reference: Ref<Node> = node
    val refs = [reference, None]
}
"#,
        );

        assert_eq!(plan.allocations[0].lifetime_domain, LifetimeDomain::Program);
    }

    #[test]
    fn ref_array_index_store_promotes_new_target_origins() {
        let plan = plan_of(
            r#"
class Node { name: String }
fun main() {
    var refs: [Ref<Node>; 2] = [None, None]
    val node = Node("index escape")
    refs[0] = node
}
"#,
        );

        let allocation = plan
            .allocations
            .iter()
            .find(|allocation| allocation.class_name == "Node")
            .expect("Node allocation");
        assert_eq!(allocation.lifetime_domain, LifetimeDomain::Program);
    }
}
