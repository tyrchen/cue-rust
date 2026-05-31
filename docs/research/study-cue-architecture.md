# Study: CUE Architecture and Rust Port Shape

Status: Done · Owner: cue-rust · Date: 2026-05-31 · Vendor pin: `vendors/cue` @ `803c837a690c75343a0f82a1029819b38e52e649`

## Why This Study

This study answers: how does `cue-lang/cue` structure parsing, loading, compiling, evaluation, validation, and export, and what architecture should a Rust implementation preserve to be semantically equivalent?

The downstream Rust work needs this because CUE is not mainly a syntax language. The core load-bearing behavior is graph unification over partially ordered values, with lexical environments, shared structure, defaults, disjunctions, closedness, and cycle handling. Upstream explicitly frames CUE as typed feature structures and graph unification, not as ordinary JSON-schema validation (`vendors/cue/doc/ref/impl.md:11`, `vendors/cue/doc/ref/impl.md:28`, `vendors/cue/doc/ref/impl.md:51`).

## Architecture Map

```text
                                   ┌──────────────────────────────┐
                                   │  Public API / CLI entry      │
                                   │  cue.Context, cue/load, cmd  │
                                   └───────────────┬──────────────┘
                                                   │
                          ┌────────────────────────▼────────────────────────┐
                          │  Source loader                                  │
                          │  cue/load + cue/build                           │
                          │  - resolves package args, modules, imports       │
                          │  - parses files into ast.File                    │
                          │  - returns build.Instance graph                  │
                          └────────────────────────┬────────────────────────┘
                                                   │ build.Instance
                 ┌─────────────────────────────────▼─────────────────────────────────┐
                 │  Runtime + compiler                                                │
                 │  internal/core/runtime + internal/core/compile                     │
                 │  - builds transitive imports                                       │
                 │  - maps AST declarations to ADT expressions                        │
                 │  - resolves identifiers into reference nodes with lexical UpCount   │
                 └─────────────────────────────────┬─────────────────────────────────┘
                                                   │ root adt.Vertex
                 ┌─────────────────────────────────▼─────────────────────────────────┐
                 │  ADT evaluator / unifier                                           │
                 │  internal/core/adt + internal/core/eval                            │
                 │  - Vertex graph, Arcs, Conjuncts, Environments                     │
                 │  - node-local scheduler for dependency/cycle handling              │
                 │  - validates concreteness, required fields, cycles                 │
                 └───────────────────────┬───────────────────────────┬───────────────┘
                                         │                           │
                  ┌──────────────────────▼─────────────┐   ┌─────────▼──────────────┐
                  │  cue.Value API                       │   │  Export / encoding     │
                  │  - immutable handle over Vertex       │   │  - AST export profiles │
                  │  - Eval, Default, Kind, Validate      │   │  - JSON/YAML/etc.      │
                  └───────────────────────────────────────┘   └──────────────────────┘
```

The public `cue.Context` is a thin wrapper over `internal/core/runtime.Runtime`; it tracks loaded instances, internal value indexes, and builtin packages (`vendors/cue/cue/context.go:33`, `vendors/cue/cue/context.go:35`). A `cue.Value` is an immutable handle containing a runtime pointer, an ADT vertex pointer, and optional parent override metadata (`vendors/cue/cue/types.go:584`, `vendors/cue/cue/types.go:589`).

## Hot Path Walkthrough

1. `cue/load.Instances` turns CLI/package arguments into one or more `build.Instance`s. It defaults empty args to `"."`, completes config, rewrites absolute paths, splits package args from file args, and wires a `build.Context` with parser and loader options (`vendors/cue/cue/load/instances.go:41`, `vendors/cue/cue/load/instances.go:47`, `vendors/cue/cue/load/instances.go:55`, `vendors/cue/cue/load/instances.go:132`, `vendors/cue/cue/load/instances.go:155`).

2. The loader materializes syntax into build instances. `loader.addFiles` iterates `BuildFiles`, applies the parser version from the module file when present, calls `getCUESyntax`, and adds parsed AST with `Instance.AddSyntax` (`vendors/cue/cue/load/loader.go:118`, `vendors/cue/cue/load/loader.go:120`, `vendors/cue/cue/load/loader.go:125`, `vendors/cue/cue/load/loader.go:129`).

3. `build.Instance` is the source/package boundary object. It stores build files, parsed `*ast.File`s, package name, import path, direct imports, load errors, and resolution errors (`vendors/cue/cue/build/instance.go:35`, `vendors/cue/cue/build/instance.go:42`, `vendors/cue/cue/build/instance.go:50`, `vendors/cue/cue/build/instance.go:60`, `vendors/cue/cue/build/instance.go:64`, `vendors/cue/cue/build/instance.go:69`, `vendors/cue/cue/build/instance.go:72`, `vendors/cue/cue/build/instance.go:77`). A Rust port needs an equivalent `BuildInstance` phase rather than compiling isolated files directly.

4. Parsing is intentionally tolerant. `parser.ParseFile` accepts filename or in-memory source, records positions, returns a partial AST with `Bad*` nodes when syntax errors occur, and resolves identifiers before returning (`vendors/cue/cue/parser/interface.go:166`, `vendors/cue/cue/parser/interface.go:179`, `vendors/cue/cue/parser/interface.go:181`, `vendors/cue/cue/parser/interface.go:207`, `vendors/cue/cue/parser/interface.go:211`, `vendors/cue/cue/parser/interface.go:217`). AST categories mirror grammar roles: expressions, declarations, labels, and clauses are separate marker interfaces (`vendors/cue/cue/ast/ast.go:97`, `vendors/cue/cue/ast/ast.go:108`, `vendors/cue/cue/ast/ast.go:118`, `vendors/cue/cue/ast/ast.go:128`).

5. Public build methods call runtime compilation. `Context.BuildInstance` parses options, calls `runtime.Build`, converts errors to bottom values, and returns `Value`; `BuildFile` compiles an already parsed AST file (`vendors/cue/cue/context.go:127`, `vendors/cue/cue/context.go:131`, `vendors/cue/cue/context.go:133`, `vendors/cue/cue/context.go:140`, `vendors/cue/cue/context.go:165`, `vendors/cue/cue/context.go:169`).

6. `runtime.Build` is the recursive compile entry. It completes the instance, returns a cached vertex when one exists, builds transitive imports from file import specs, appends resolution errors, calls `compile.Instance`, injects `@extern` implementations, wraps errors as bottom vertices, and caches the instance-to-vertex mapping (`vendors/cue/internal/core/runtime/build.go:39`, `vendors/cue/internal/core/runtime/build.go:41`, `vendors/cue/internal/core/runtime/build.go:45`, `vendors/cue/internal/core/runtime/build.go:58`, `vendors/cue/internal/core/runtime/build.go:75`, `vendors/cue/internal/core/runtime/build.go:78`, `vendors/cue/internal/core/runtime/build.go:80`, `vendors/cue/internal/core/runtime/build.go:85`). Import recursion is explicit in `buildSpec` (`vendors/cue/internal/core/runtime/build.go:120`, `vendors/cue/internal/core/runtime/build.go:126`, `vendors/cue/internal/core/runtime/build.go:139`, `vendors/cue/internal/core/runtime/build.go:143`).

7. The compiler lowers AST into the ADT. `compile.Instance` creates a compiler and calls `compileFiles` (`vendors/cue/internal/core/compile/compile.go:65`, `vendors/cue/internal/core/compile/compile.go:69`, `vendors/cue/internal/core/compile/compile.go:71`). `compileFiles` precomputes package-level field scope, builds a root `Vertex`, constructs an `Environment` chain from the optional external scope, converts each file into an ADT `StructLit`, and inserts one root conjunct per file (`vendors/cue/internal/core/compile/compile.go:285`, `vendors/cue/internal/core/compile/compile.go:286`, `vendors/cue/internal/core/compile/compile.go:291`, `vendors/cue/internal/core/compile/compile.go:317`, `vendors/cue/internal/core/compile/compile.go:321`, `vendors/cue/internal/core/compile/compile.go:330`, `vendors/cue/internal/core/compile/compile.go:334`, `vendors/cue/internal/core/compile/compile.go:336`).

8. Identifier resolution is compiled, not re-derived during evaluation. The compiler converts unresolved identifiers into `FieldReference`, import identifiers into `ImportReference`, label aliases into `LabelReference`/`DynamicReference`, alias references into `ValueReference`, and let references into `LetReference` (`vendors/cue/internal/core/compile/compile.go:423`, `vendors/cue/internal/core/compile/compile.go:434`, `vendors/cue/internal/core/compile/compile.go:463`, `vendors/cue/internal/core/compile/compile.go:521`, `vendors/cue/internal/core/compile/compile.go:560`, `vendors/cue/internal/core/compile/compile.go:568`, `vendors/cue/internal/core/compile/compile.go:614`). The `UpCount` recorded here is the key to preserving lexical scoping in Rust.

9. Evaluation is demand driven. `eval.Evaluate` creates an `adt.OpContext` and finalizes the root vertex (`vendors/cue/internal/core/eval/eval.go:23`, `vendors/cue/internal/core/eval/eval.go:24`, `vendors/cue/internal/core/eval/eval.go:28`). `Value.Kind`, JSON marshaling, validation, export, and reference resolution all force enough of the vertex graph to answer their question (`vendors/cue/cue/types.go:801`, `vendors/cue/cue/types.go:824`, `vendors/cue/cue/types.go:838`, `vendors/cue/cue/types.go:843`, `vendors/cue/cue/types.go:886`, `vendors/cue/cue/types.go:939`).

## Key Data Structures

### `ast.File`, `ast.StructLit`, and Syntax Nodes

The AST is deliberately close to source. `ast.File` keeps filename, top-level declarations, unresolved identifiers, language version, and comments (`vendors/cue/cue/ast/ast.go:1108`, `vendors/cue/cue/ast/ast.go:1110`, `vendors/cue/cue/ast/ast.go:1113`, `vendors/cue/cue/ast/ast.go:1120`). `ast.StructLit` is a declaration list with source positions and comments (`vendors/cue/cue/ast/ast.go:595`, `vendors/cue/cue/ast/ast.go:597`, `vendors/cue/cue/ast/ast.go:600`). For Rust, keep source-preserving AST separate from the semantic ADT. Do not try to evaluate from parser nodes directly.

### `Runtime` and `index`

`Runtime` owns reusable evaluation state: an index, a loaded-instance map, extern injections, evaluator version, and debug flags (`vendors/cue/internal/core/runtime/runtime.go:25`, `vendors/cue/internal/core/runtime/runtime.go:26`, `vendors/cue/internal/core/runtime/runtime.go:29`, `vendors/cue/internal/core/runtime/runtime.go:31`, `vendors/cue/internal/core/runtime/runtime.go:35`, `vendors/cue/internal/core/runtime/runtime.go:37`). Its index stores builtin package registries, instance-to-vertex caches, a unique-id counter, and a Go-type cache (`vendors/cue/internal/core/runtime/imports.go:77`, `vendors/cue/internal/core/runtime/imports.go:80`, `vendors/cue/internal/core/runtime/imports.go:83`, `vendors/cue/internal/core/runtime/imports.go:85`, `vendors/cue/internal/core/runtime/imports.go:88`, `vendors/cue/internal/core/runtime/imports.go:89`).

String labels are globally interned through a mutex-protected `labelMap`/`labels` pair, with label 0 reserved for `_` (`vendors/cue/internal/core/runtime/index.go:61`, `vendors/cue/internal/core/runtime/index.go:63`, `vendors/cue/internal/core/runtime/index.go:68`, `vendors/cue/internal/core/runtime/index.go:73`, `vendors/cue/internal/core/runtime/index.go:80`, `vendors/cue/internal/core/runtime/index.go:86`). A Rust implementation should use an interner with stable numeric symbols, but should avoid making the interner a hidden global unless compatibility or memory sharing requires it.

### `Feature`

`Feature` is a compact encoded label containing an integer/string index plus a label type (`vendors/cue/internal/core/adt/feature.go:28`, `vendors/cue/internal/core/adt/feature.go:30`). The `StringIndexer` contract requires stable bidirectional mapping and unique ids (`vendors/cue/internal/core/adt/feature.go:51`, `vendors/cue/internal/core/adt/feature.go:58`, `vendors/cue/internal/core/adt/feature.go:61`, `vendors/cue/internal/core/adt/feature.go:63`). Rust should model this as a small copyable label newtype, not as `String` keys on every vertex.

### `Vertex`

`Vertex` is the semantic graph node. It may be a leaf or composite, has arcs for evaluated struct/list members, retains source conjuncts, tracks evaluation status, closedness flags, optionality, dynamic/shared/internal state, pattern constraints, child errors, and contributing struct literals (`vendors/cue/internal/core/adt/composite.go:144`, `vendors/cue/internal/core/adt/composite.go:147`, `vendors/cue/internal/core/adt/composite.go:150`, `vendors/cue/internal/core/adt/composite.go:152`, `vendors/cue/internal/core/adt/composite.go:158`, `vendors/cue/internal/core/adt/composite.go:172`, `vendors/cue/internal/core/adt/composite.go:180`, `vendors/cue/internal/core/adt/composite.go:193`, `vendors/cue/internal/core/adt/composite.go:202`, `vendors/cue/internal/core/adt/composite.go:229`, `vendors/cue/internal/core/adt/composite.go:234`, `vendors/cue/internal/core/adt/composite.go:237`, `vendors/cue/internal/core/adt/composite.go:247`, `vendors/cue/internal/core/adt/composite.go:249`, `vendors/cue/internal/core/adt/composite.go:256`, `vendors/cue/internal/core/adt/composite.go:267`).

Rust implication: this wants arena allocation with stable node ids or `Arc`-backed immutable handles plus interior evaluator state. Plain owned trees will fail because CUE references can share substructure and evaluation mutates node state.

### `Environment` and `Conjunct`

An `Environment` links lexical parent scopes to a composite node and also carries dynamic labels, comprehension ids, and per-environment caches (`vendors/cue/internal/core/adt/composite.go:83`, `vendors/cue/internal/core/adt/composite.go:86`, `vendors/cue/internal/core/adt/composite.go:96`, `vendors/cue/internal/core/adt/composite.go:100`, `vendors/cue/internal/core/adt/composite.go:111`). Upstream’s ADT docs explain why: CUE reference semantics behave like copying the referenced value, but the implementation avoids actual copies by resolving references through environments (`vendors/cue/internal/core/adt/doc.go:41`, `vendors/cue/internal/core/adt/doc.go:42`, `vendors/cue/internal/core/adt/doc.go:59`, `vendors/cue/internal/core/adt/doc.go:60`, `vendors/cue/internal/core/adt/doc.go:63`).

A `Conjunct` is the pair of an environment and expression/node, plus close information (`vendors/cue/internal/core/adt/composite.go:1471`, `vendors/cue/internal/core/adt/composite.go:1473`, `vendors/cue/internal/core/adt/composite.go:1477`). Duplicate fields are not overwritten; they become multiple conjuncts that unify. This matches the formal implementation note that duplicate labels are represented as unification `&(a, b)` (`vendors/cue/doc/ref/impl.md:153`, `vendors/cue/doc/ref/impl.md:155`, `vendors/cue/doc/ref/impl.md:156`).

### `OpContext`

`OpContext` is per-operation evaluator state. It amortizes allocations, records current vertex/parents, stores errors and stats, and is explicitly not goroutine-safe (`vendors/cue/internal/core/adt/context.go:100`, `vendors/cue/internal/core/adt/context.go:101`, `vendors/cue/internal/core/adt/context.go:105`, `vendors/cue/internal/core/adt/context.go:108`, `vendors/cue/internal/core/adt/context.go:111`, `vendors/cue/internal/core/adt/context.go:114`, `vendors/cue/internal/core/adt/context.go:123`). Rust should make this non-`Sync` by construction, and keep public values immutable while using operation-local mutable contexts for evaluation.

### References

References are ADT nodes with different behavior. `FieldReference` looks up a label in a relative environment (`vendors/cue/internal/core/adt/expr.go:566`, `vendors/cue/internal/core/adt/expr.go:567`, `vendors/cue/internal/core/adt/expr.go:576`). `ValueReference` returns the current or ancestor vertex (`vendors/cue/internal/core/adt/expr.go:593`, `vendors/cue/internal/core/adt/expr.go:611`). `LabelReference` evaluates the current label to a value (`vendors/cue/internal/core/adt/expr.go:619`, `vendors/cue/internal/core/adt/expr.go:638`, `vendors/cue/internal/core/adt/expr.go:648`). `DynamicReference` evaluates an expression to compute a label before lookup (`vendors/cue/internal/core/adt/expr.go:651`, `vendors/cue/internal/core/adt/expr.go:679`, `vendors/cue/internal/core/adt/expr.go:691`). `ImportReference` loads either an instance or builtin package (`vendors/cue/internal/core/adt/expr.go:704`, `vendors/cue/internal/core/adt/expr.go:727`, `vendors/cue/internal/core/adt/expr.go:729`, `vendors/cue/internal/core/adt/expr.go:732`). `LetReference` is cached per environment/expression/arc to avoid exponential behavior (`vendors/cue/internal/core/adt/expr.go:741`, `vendors/cue/internal/core/adt/expr.go:760`, `vendors/cue/internal/core/adt/expr.go:802`, `vendors/cue/internal/core/adt/expr.go:816`, `vendors/cue/internal/core/adt/expr.go:824`, `vendors/cue/internal/core/adt/expr.go:841`).

## Key Algorithms

### Unification as Greatest Lower Bound

The formal semantics define unification as the greatest lower bound in the subsumption order (`vendors/cue/doc/ref/impl.md:186`, `vendors/cue/doc/ref/impl.md:192`). Evaluation is the process of making the typed feature structure well formed (`vendors/cue/doc/ref/impl.md:213`, `vendors/cue/doc/ref/impl.md:244`). In implementation, `adt.Unify` creates a fresh vertex, adds conjuncts from both operands, finalizes it, and returns that vertex (`vendors/cue/internal/core/adt/composite.go:929`, `vendors/cue/internal/core/adt/composite.go:934`, `vendors/cue/internal/core/adt/composite.go:944`, `vendors/cue/internal/core/adt/composite.go:945`, `vendors/cue/internal/core/adt/composite.go:956`).

Rust rule: implement unification over ADT values first, before building APIs or encoders. Every higher-level feature depends on it.

### Lazy Finalization and Node Scheduler

`Vertex.getState` initializes a node context and schedules conjuncts only once (`vendors/cue/internal/core/adt/unify.go:47`, `vendors/cue/internal/core/adt/unify.go:62`, `vendors/cue/internal/core/adt/unify.go:64`, `vendors/cue/internal/core/adt/unify.go:65`). Scheduling sets a cycle placeholder, pushes the vertex onto the operation context, and schedules each conjunct (`vendors/cue/internal/core/adt/unify.go:91`, `vendors/cue/internal/core/adt/unify.go:97`, `vendors/cue/internal/core/adt/unify.go:99`, `vendors/cue/internal/core/adt/unify.go:107`, `vendors/cue/internal/core/adt/unify.go:109`, `vendors/cue/internal/core/adt/unify.go:111`, `vendors/cue/internal/core/adt/unify.go:115`).

The scheduler is not a general async runtime. It is a node-local dependency engine. Tasks depend on properties such as field existence, scalar value, conjunct set, subfields, or recursive value; blocked cycles are frozen and unblocked in phases (`vendors/cue/internal/core/adt/sched.go:21`, `vendors/cue/internal/core/adt/sched.go:23`, `vendors/cue/internal/core/adt/sched.go:40`, `vendors/cue/internal/core/adt/sched.go:43`, `vendors/cue/internal/core/adt/sched.go:49`, `vendors/cue/internal/core/adt/sched.go:53`, `vendors/cue/internal/core/adt/sched.go:55`). Scheduler state is compact bitsets/counters plus task queues (`vendors/cue/internal/core/adt/sched.go:246`, `vendors/cue/internal/core/adt/sched.go:253`, `vendors/cue/internal/core/adt/sched.go:275`, `vendors/cue/internal/core/adt/sched.go:280`, `vendors/cue/internal/core/adt/sched.go:292`). Rust should copy this as a deterministic single-threaded evaluator first; parallelism can be layered later if the graph semantics are proven.

### Defaults and Disjunctions

Defaults are resolved by narrowing disjunctions to default alternatives and by closing open lists when computing default values (`vendors/cue/internal/core/adt/default.go:21`, `vendors/cue/internal/core/adt/default.go:33`, `vendors/cue/internal/core/adt/default.go:48`, `vendors/cue/internal/core/adt/default.go:57`, `vendors/cue/internal/core/adt/default.go:63`, `vendors/cue/internal/core/adt/default.go:73`, `vendors/cue/internal/core/adt/default.go:92`, `vendors/cue/internal/core/adt/default.go:97`). `cue.Value.Default` exposes this as a value plus whether it changed (`vendors/cue/cue/types.go:774`, `vendors/cue/cue/types.go:781`, `vendors/cue/cue/types.go:782`, `vendors/cue/cue/types.go:787`).

Rust rule: do not desugar defaults in the parser. Defaults are semantic choices that interact with disjunctions, validation, export, and JSON marshaling.

### Validation

Validation is a separate pass over an already evaluated vertex. Options control concrete requirement, final required-field checking, cycle disallowance, incomplete reporting, and all-errors traversal (`vendors/cue/internal/core/adt/validate.go:17`, `vendors/cue/internal/core/adt/validate.go:18`, `vendors/cue/internal/core/adt/validate.go:21`, `vendors/cue/internal/core/adt/validate.go:24`, `vendors/cue/internal/core/adt/validate.go:27`, `vendors/cue/internal/core/adt/validate.go:31`, `vendors/cue/internal/core/adt/validate.go:37`). The validator treats definitions differently, dereferences shared values carefully, reports required fields only when final validation is requested, and skips lets/non-defined arcs (`vendors/cue/internal/core/adt/validate.go:95`, `vendors/cue/internal/core/adt/validate.go:113`, `vendors/cue/internal/core/adt/validate.go:118`, `vendors/cue/internal/core/adt/validate.go:129`, `vendors/cue/internal/core/adt/validate.go:148`, `vendors/cue/internal/core/adt/validate.go:167`, `vendors/cue/internal/core/adt/validate.go:180`, `vendors/cue/internal/core/adt/validate.go:188`).

Rust rule: expose validation policy as explicit options. Do not fold validation into evaluation or JSON encoding.

### Export and Encoding

Export is profile-driven. Profiles choose whether to simplify, take defaults, require finality, include optional/definitions/hidden/docs/attrs, inline imports, or expand references (`vendors/cue/internal/core/export/export.go:34`, `vendors/cue/internal/core/export/export.go:37`, `vendors/cue/internal/core/export/export.go:40`, `vendors/cue/internal/core/export/export.go:43`, `vendors/cue/internal/core/export/export.go:62`, `vendors/cue/internal/core/export/export.go:73`, `vendors/cue/internal/core/export/export.go:76`). `Value.Syntax` constructs an export profile and chooses `Vertex` export for concrete/final mode or `Def` export for schema mode (`vendors/cue/cue/types.go:886`, `vendors/cue/cue/types.go:897`, `vendors/cue/cue/types.go:939`, `vendors/cue/cue/types.go:945`, `vendors/cue/cue/types.go:946`).

JSON marshaling takes defaults, evaluates enough of the value, rejects unresolved or non-concrete values, and then emits concrete scalar/list/struct forms (`vendors/cue/cue/types.go:824`, `vendors/cue/cue/types.go:833`, `vendors/cue/cue/types.go:834`, `vendors/cue/cue/types.go:838`, `vendors/cue/cue/types.go:840`, `vendors/cue/cue/types.go:843`, `vendors/cue/cue/types.go:848`). Rust should keep CUE AST export, JSON emission, YAML emission, and schema export as clients of the same evaluated ADT, not as separate evaluators.

## What We Will Adopt

1. Use a three-layer model: source AST, compiled ADT, public immutable value handle. Upstream keeps parser AST (`vendors/cue/cue/ast/ast.go:1108`), compiled ADT (`vendors/cue/internal/core/adt/doc.go:15`), and `cue.Value` wrapper (`vendors/cue/cue/types.go:584`) distinct.

2. Model values as a graph of vertices and arcs, not as a tree. Upstream relies on structure sharing and graph treatment for cycles (`vendors/cue/doc/ref/impl.md:28`, `vendors/cue/doc/ref/impl.md:29`, `vendors/cue/doc/ref/impl.md:30`), and `Vertex` directly encodes parent, arcs, shared status, and conjunct provenance (`vendors/cue/internal/core/adt/composite.go:152`, `vendors/cue/internal/core/adt/composite.go:247`, `vendors/cue/internal/core/adt/composite.go:256`).

3. Intern labels into compact `Feature`-like ids. Field lookup and ordering are too central to use repeated strings (`vendors/cue/internal/core/adt/feature.go:28`, `vendors/cue/internal/core/adt/feature.go:51`).

4. Compile references into explicit reference nodes with lexical `UpCount`. This is the core trick that avoids runtime copying while preserving CUE reference semantics (`vendors/cue/internal/core/adt/doc.go:59`, `vendors/cue/internal/core/adt/doc.go:63`, `vendors/cue/internal/core/compile/compile.go:463`, `vendors/cue/internal/core/compile/compile.go:560`, `vendors/cue/internal/core/compile/compile.go:614`).

5. Use operation-local evaluation contexts. Public values can be immutable and callable concurrently, while `OpContext` is intentionally single-goroutine and per operation (`vendors/cue/cue/types.go:588`, `vendors/cue/internal/core/adt/context.go:108`).

6. Preserve tolerant parsing and rich errors. Parser returns partial ASTs and sanitized sorted error lists, instead of failing with no tree (`vendors/cue/cue/parser/interface.go:179`, `vendors/cue/cue/parser/interface.go:181`, `vendors/cue/cue/parser/interface.go:207`).

7. Treat loading/package resolution as a real subsystem. CUE modules, imports, package qualifiers, tag injection, file source handling, and extern injection all happen before or around compile (`vendors/cue/cue/load/instances.go:102`, `vendors/cue/cue/load/instances.go:132`, `vendors/cue/cue/load/instances.go:193`, `vendors/cue/internal/core/runtime/extern.go:72`).

8. Make validation and export profile-driven. Upstream has explicit config structs for both (`vendors/cue/internal/core/adt/validate.go:17`, `vendors/cue/internal/core/export/export.go:34`).

## What We Will Avoid

1. Avoid a direct AST interpreter. It will duplicate identifier resolution, miss `Environment` sharing semantics, and make cycles/defaults harder. Upstream’s compiler centralizes scope and reference lowering before evaluation (`vendors/cue/internal/core/compile/compile.go:418`, `vendors/cue/internal/core/compile/compile.go:423`).

2. Avoid representing duplicate fields as map overwrites. Duplicate labels are unification, not replacement (`vendors/cue/doc/ref/impl.md:153`, `vendors/cue/doc/ref/impl.md:156`).

3. Avoid eager full evaluation on load. Upstream often finalizes on demand, and `Value.Kind` distinguishes concrete kind from incomplete kind (`vendors/cue/cue/types.go:801`, `vendors/cue/cue/types.go:816`).

4. Avoid exposing evaluator mutation through public handles. Upstream says `Value` is immutable and methods may be called concurrently, while the internal operation context is not goroutine-safe (`vendors/cue/cue/types.go:588`, `vendors/cue/internal/core/adt/context.go:108`).

5. Avoid making builtin packages special cases in the parser. Upstream registers builtins in runtime package registries and resolves imports through `ImportReference` at evaluation (`vendors/cue/internal/core/runtime/imports.go:28`, `vendors/cue/internal/core/runtime/imports.go:32`, `vendors/cue/internal/core/adt/expr.go:727`).

## Open Questions

1. `spike-rust-arena-vertex.md`: should Rust vertices be represented by generational arena ids, `slotmap`, or `Arc` nodes with operation-local mutable state? This affects cycle detection, sharing, and API concurrency.

2. `spike-rust-scheduler-port.md`: can the upstream node scheduler be ported almost mechanically with bitflags and task queues, or should Rust start with a simpler recursive evaluator and add the scheduler after semantic tests are available?

3. `spike-cue-test-corpus.md`: which upstream conformance/testdata directories should be vendored or mirrored into Rust integration tests first, and what golden format should we use?

4. `spike-rust-parser-stack.md`: choose parser technology for CUE grammar in Rust. The project guidance prefers `winnow` for grammars, but CUE’s semicolon/comma insertion, interpolation, comments, partial AST recovery, and source positions need a focused parser spike.

5. `spike-decimal-regex-semantics.md`: identify Rust crates and compatibility gaps for arbitrary precision decimals, regex behavior, Unicode/string literal handling, and JSON number output.
