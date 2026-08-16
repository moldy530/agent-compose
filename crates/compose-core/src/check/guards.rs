//! Grammar 7.3.1's closed set of guard shapes, read for coverage and for
//! disjointness (Decision D82).
//!
//! Two checks read one table. Exhaustiveness (7.3.1) asks what a guard
//! **guarantees** — the variants for which it is true whatever else is true of
//! the run — and exclusivity (7.6.1 rule 2) asks what it leaves **possible** —
//! the variants for which it is not provably false. The table is the same
//! either way, and it is closed and syntactic so that two conforming validators
//! accept exactly the same compositions:
//!
//! | shape of `g` | `guaranteed(g, f)` | `possible(g, f)` |
//! |---|---|---|
//! | `n.output.f == L` | `{L}` | `{L}` |
//! | `n.output.f != L` | `V \ {L}` | `V \ {L}` |
//! | `n.output.f in [L₁ … Lₖ]` | `{L₁ … Lₖ}` | `{L₁ … Lₖ}` |
//! | `!g₁` | `V \ possible(g₁, f)` | `V \ guaranteed(g₁, f)` |
//! | `g₁ && g₂` | `∩ guaranteed` | `∩ possible` |
//! | `g₁ \|\| g₂` | `∪ guaranteed` | `∪ possible` |
//! | anything else | `∅` | `V` |
//!
//! The last row is the load-bearing one, and it is why the two columns are
//! needed rather than one: a term the table does not recognize contributes no
//! guarantee and excludes no variant, so
//! `n.output.f == 'a' && size(state.xs) > 0` is guaranteed for nothing and
//! possible only for `a`. Reading "can be true for it" as coverage — the
//! reading D82 replaced — would let that guard route every variant and leave
//! the composition dead-ending on grammar 7.3 rule 7 at run time.
//!
//! **This module never reports.** Every expression it reads has already been
//! parsed and type-checked by [`cel`](crate::cel) at the edge-guard surface, so
//! a literal compared against `f` is already known to be one of `f`'s variants
//! and an expression that did not parse is already a diagnostic. What is left
//! here is a walk over shapes.

use std::collections::BTreeSet;

use ::cel::Program;
use ::cel::common::ast::operators::{EQUALS, IN, LOGICAL_AND, LOGICAL_NOT, LOGICAL_OR, NOT_EQUALS};
use ::cel::common::ast::{Expr, IdedExpr, LiteralValue};

/// What a guard says about one enum-typed field (grammar 7.3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Coverage {
    /// The variants for which the guard is true whatever else is true of the
    /// run.
    pub(crate) guaranteed: BTreeSet<String>,
    /// The variants for which the guard is not provably false.
    pub(crate) possible: BTreeSet<String>,
}

/// Parse a guard, or `None` where it does not parse — which [`cel`](crate::cel)
/// has already reported against the expression's own span.
pub(crate) fn parse(source: &str) -> Option<Program> {
    Program::compile(source).ok()
}

/// Whether the guard mentions `<node>.output.<field>` syntactically, which is
/// what puts the node in scope of the exhaustiveness check for that field
/// (grammar 7.3.1, "when the check fires").
pub(crate) fn mentions(expr: &IdedExpr, node: &str, field: &str) -> bool {
    if names_field(expr, node, field, true) {
        return true;
    }
    children(expr).into_iter().any(|c| mentions(c, node, field))
}

/// Read one guard against one enum field.
pub(crate) fn coverage(
    expr: &IdedExpr,
    node: &str,
    field: &str,
    variants: &BTreeSet<String>,
) -> Coverage {
    let unknown = || Coverage {
        guaranteed: BTreeSet::new(),
        possible: variants.clone(),
    };
    let exactly = |set: BTreeSet<String>| Coverage {
        guaranteed: set.clone(),
        possible: set,
    };
    let Expr::Call(call) = &expr.expr else {
        return unknown();
    };
    // Every operator of the table is a plain call: none of them is written on a
    // receiver, so an `args`-only reading is the whole of their shape.
    let operands = &call.args;
    match call.func_name.as_str() {
        EQUALS | NOT_EQUALS => {
            let (Some(left), Some(right)) = (operands.first(), operands.get(1)) else {
                return unknown();
            };
            // Either operand order is accepted (grammar 7.3.1).
            let literal = if is_field(left, node, field) {
                string(right)
            } else if is_field(right, node, field) {
                string(left)
            } else {
                None
            };
            let Some(literal) = literal.filter(|value| variants.contains(value)) else {
                return unknown();
            };
            if call.func_name == EQUALS {
                exactly(BTreeSet::from([literal]))
            } else {
                exactly(
                    variants
                        .iter()
                        .filter(|v| **v != literal)
                        .cloned()
                        .collect(),
                )
            }
        }
        IN => {
            let (Some(element), Some(container)) = (operands.first(), operands.get(1)) else {
                return unknown();
            };
            if !is_field(element, node, field) {
                return unknown();
            }
            let Expr::List(elements) = &container.expr else {
                return unknown();
            };
            let mut listed = BTreeSet::new();
            for element in &elements.elements {
                // One element the table cannot read makes the whole list
                // unreadable: a list holding a variable proves nothing about
                // which variants it holds.
                let Some(value) = string(element).filter(|value| variants.contains(value)) else {
                    return unknown();
                };
                listed.insert(value);
            }
            exactly(listed)
        }
        LOGICAL_NOT => {
            let Some(inner) = operands.first() else {
                return unknown();
            };
            let inner = coverage(inner, node, field, variants);
            Coverage {
                guaranteed: variants.difference(&inner.possible).cloned().collect(),
                possible: variants.difference(&inner.guaranteed).cloned().collect(),
            }
        }
        LOGICAL_AND | LOGICAL_OR => {
            let union = call.func_name == LOGICAL_OR;
            let mut folded: Option<Coverage> = None;
            for operand in operands {
                let next = coverage(operand, node, field, variants);
                folded = Some(match folded {
                    None => next,
                    Some(current) => Coverage {
                        guaranteed: combine(&current.guaranteed, &next.guaranteed, union),
                        possible: combine(&current.possible, &next.possible, union),
                    },
                });
            }
            folded.unwrap_or_else(unknown)
        }
        _ => unknown(),
    }
}

/// The two operators fold their operands the same way, in opposite directions.
fn combine(left: &BTreeSet<String>, right: &BTreeSet<String>, union: bool) -> BTreeSet<String> {
    if union {
        left.union(right).cloned().collect()
    } else {
        left.intersection(right).cloned().collect()
    }
}

/// Whether this sub-expression names `<node>.output.<field>`.
///
/// A presence test — `has(n.output.f)`, which the parser expands into a
/// `Select` carrying `test` — *mentions* the field, so it puts the node in
/// scope of the check (`presence_counts`), but it is not a shape the coverage
/// table reads: it asks whether the field is there, not what it holds, so for
/// coverage it falls to the last row and proves nothing.
fn names_field(expr: &IdedExpr, node: &str, field: &str, presence_counts: bool) -> bool {
    let Expr::Select(outer) = &expr.expr else {
        return false;
    };
    if outer.field != field || (outer.test && !presence_counts) {
        return false;
    }
    let Expr::Select(inner) = &outer.operand.expr else {
        return false;
    };
    if inner.test || inner.field != "output" {
        return false;
    }
    matches!(&inner.operand.expr, Expr::Ident(name) if name == node)
}

/// The coverage table's reading of `<node>.output.<field>`.
fn is_field(expr: &IdedExpr, node: &str, field: &str) -> bool {
    names_field(expr, node, field, false)
}

/// The string a literal holds.
fn string(expr: &IdedExpr) -> Option<String> {
    match &expr.expr {
        Expr::Literal(LiteralValue::String(value)) => Some(value.inner().to_string()),
        _ => None,
    }
}

/// Every sub-expression one node holds, for the syntactic `mentions` walk.
fn children(expr: &IdedExpr) -> Vec<&IdedExpr> {
    match &expr.expr {
        Expr::Call(call) => call
            .target
            .as_deref()
            .into_iter()
            .chain(call.args.iter())
            .collect(),
        Expr::Select(select) => vec![&select.operand],
        Expr::List(list) => list.elements.iter().collect(),
        Expr::Map(map) => map
            .entries
            .iter()
            .flat_map(|entry| match &entry.expr {
                ::cel::common::ast::EntryExpr::MapEntry(entry) => vec![&entry.key, &entry.value],
                ::cel::common::ast::EntryExpr::StructField(field) => vec![&field.value],
            })
            .collect(),
        Expr::Struct(structure) => structure
            .entries
            .iter()
            .flat_map(|entry| match &entry.expr {
                ::cel::common::ast::EntryExpr::MapEntry(entry) => vec![&entry.key, &entry.value],
                ::cel::common::ast::EntryExpr::StructField(field) => vec![&field.value],
            })
            .collect(),
        Expr::Comprehension(comprehension) => vec![
            &comprehension.iter_range,
            &comprehension.accu_init,
            &comprehension.loop_cond,
            &comprehension.loop_step,
            &comprehension.result,
        ],
        Expr::Ident(_) | Expr::Literal(_) | Expr::Unspecified => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variants() -> BTreeSet<String> {
        ["approve", "revise", "escalate"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    #[track_caller]
    fn read(source: &str) -> Coverage {
        let program = parse(source).expect("the guard parses");
        coverage(program.expression(), "review", "verdict", &variants())
    }

    fn set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn equality_guarantees_one_variant_either_way_round() {
        for source in [
            "review.output.verdict == 'approve'",
            "'approve' == review.output.verdict",
        ] {
            let found = read(source);
            assert_eq!(found.guaranteed, set(&["approve"]));
            assert_eq!(found.possible, set(&["approve"]));
        }
    }

    #[test]
    fn inequality_and_membership_name_their_complements() {
        let found = read("review.output.verdict != 'approve'");
        assert_eq!(found.guaranteed, set(&["escalate", "revise"]));
        let found = read("review.output.verdict in ['approve', 'revise']");
        assert_eq!(found.guaranteed, set(&["approve", "revise"]));
        assert_eq!(found.possible, set(&["approve", "revise"]));
    }

    /// The table's last row, and the asymmetry the two columns exist for
    /// (grammar 7.3.1's worked example).
    #[test]
    fn an_unrecognized_term_guarantees_nothing_and_excludes_nothing() {
        let found = read("size(state.feedback) > 0");
        assert!(found.guaranteed.is_empty());
        assert_eq!(found.possible, variants());

        let found = read("review.output.verdict == 'approve' && size(state.feedback) > 0");
        assert!(found.guaranteed.is_empty(), "a conjunct proves nothing");
        assert_eq!(found.possible, set(&["approve"]), "and excludes nothing");
    }

    #[test]
    fn negation_swaps_the_two_columns() {
        let found = read("!(review.output.verdict == 'approve')");
        assert_eq!(found.guaranteed, set(&["escalate", "revise"]));
        assert_eq!(found.possible, set(&["escalate", "revise"]));

        // `!(unrecognized)`: possible is `V \ ∅` and guaranteed is `V \ V`.
        let found = read("!(size(state.feedback) > 0)");
        assert!(found.guaranteed.is_empty());
        assert_eq!(found.possible, variants());
    }

    #[test]
    fn disjunction_unions_and_conjunction_intersects() {
        let found = read("review.output.verdict == 'approve' || review.output.verdict == 'revise'");
        assert_eq!(found.guaranteed, set(&["approve", "revise"]));
        let found = read("review.output.verdict != 'approve' && review.output.verdict != 'revise'");
        assert_eq!(found.guaranteed, set(&["escalate"]));
    }

    /// A literal outside the variant set is a type error the CEL front-end
    /// raises first; the table treats the shape as unreadable rather than
    /// inventing a variant.
    #[test]
    fn a_literal_that_is_not_a_variant_reads_as_nothing() {
        let found = read("review.output.verdict == 'aprove'");
        assert!(found.guaranteed.is_empty());
        assert_eq!(found.possible, variants());
    }

    #[test]
    fn a_guard_over_another_node_is_not_this_field() {
        let found = read("other.output.verdict == 'approve'");
        assert!(found.guaranteed.is_empty());
        let found = read("review.output.other == 'approve'");
        assert!(found.guaranteed.is_empty());
    }

    #[test]
    fn mention_is_syntactic_and_reaches_inside_a_call() {
        let program = parse("size(state.xs) > 0 && review.output.verdict == 'approve'")
            .expect("the guard parses");
        assert!(mentions(program.expression(), "review", "verdict"));
        assert!(!mentions(program.expression(), "review", "other"));
        let program = parse("has(review.output.verdict)").expect("the guard parses");
        assert!(
            mentions(program.expression(), "review", "verdict"),
            "a presence test still mentions the field"
        );
        assert!(
            read("has(review.output.verdict)").guaranteed.is_empty(),
            "and still proves nothing about it"
        );
    }
}
