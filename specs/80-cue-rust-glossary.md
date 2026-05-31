# cue-rust Glossary

Status: Draft
Last updated: 2026-05-31

## Terms

ADT: Abstract data type layer used by CUE to represent semantic values, references, vertices, conjuncts, environments, and constraints.

Arc: A labeled edge from a vertex to a child vertex.

Bottom: The invalid value in the CUE lattice. It can represent conflict, incomplete value, failed validation, or infrastructure-derived semantic failure.

BuildInstance: Loader output describing one package or file set before runtime compilation.

Closedness: Constraint controlling which fields may appear in a struct at a given level or recursively.

Conjunct: A semantic contribution to a vertex, usually an expression paired with the environment in which it should be evaluated.

Context: Public SDK entry point that owns runtime state and builds values.

Default: Preferred disjunction alternative selected by defaulting operations such as concrete export.

Diagnostic: Structured error or warning with code, message, spans, and optional path.

Disjunction: CUE `|` value representing alternatives.

Environment: Lexical scope chain used to resolve references without copying values.

Feature: Compact interned label used for fields, definitions, hidden fields, list indices, and special labels.

Loader: Subsystem that resolves package arguments, files, modules, overlays, tags, stdin, imports, and data files into build instances.

OpContext: Operation-local evaluator state. It is intentionally mutable and not shared across operations.

Source AST: Source-preserving syntax tree produced by the parser.

Unification: CUE operation that computes the greatest lower bound of values in the subsumption order.

Value: Public immutable handle to a semantic CUE value.

Vertex: Semantic graph node representing a CUE value with arcs, conjuncts, status, closedness, and errors.
