# Issue 0010 — Disjunction operations have unbounded cartesian expansion

Status: fixed in working tree · Severity: medium (CPU/memory denial of service) · Found: 2026-06-03
Component: `crates/eval`

## Summary

Binary operations over choices and disjunction unification expand all pairwise
combinations. Repeated disjunctions can grow quadratically or exponentially,
allocating large intermediate vectors before deduplication.

## Impact

A compact schema can consume excessive CPU and memory during evaluation or
validation. Current source size and evaluation depth limits do not bound the
number of generated disjunct combinations.

## Expected behavior

Evaluation should fail deterministically with a diagnostic once disjunction
expansion exceeds a configured or conservative internal limit.

## Suggested fix

Add an expansion budget to the evaluator:

- check `left_disjuncts.len() * right_disjuncts.len()` with checked arithmetic;
- return a structured bottom value when the expansion would exceed the limit;
- cover both `evaluate_choice_binary` and `unify_disjunctions`;
- add regression tests for a deliberately oversized disjunction product.

## Resolution

Added a shared evaluator budget for disjunction cartesian expansion. Binary
operations that distribute over choices and disjunction-to-disjunction
unification now check `left_len * right_len` before allocating or iterating and
return `cue.eval.disjunction_expansion_limit` when the product exceeds the
limit. SDK coverage constructs a 65 x 65 disjunction multiplication and verifies
the bounded bottom result.
