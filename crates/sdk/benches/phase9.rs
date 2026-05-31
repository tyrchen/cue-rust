//! Phase 9 smoke benchmarks.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use cue_rust::{Context, EncodeOptions, Encoding, encode_value};

const SOURCE: &str = r#"
package bench

service: {
    name: "api"
    replicas: 3
    enabled: true
}
"#;

fn bench_parser(criterion: &mut Criterion) {
    let context = Context::new();
    criterion.bench_function("parser/basic", |bencher| {
        bencher.iter(|| {
            let parsed = context.parse_source("bench.cue", SOURCE);
            black_box(parsed.diagnostics().has_errors());
        });
    });
}

fn bench_compiler(criterion: &mut Criterion) {
    let context = Context::new();
    criterion.bench_function("compiler/basic", |bencher| {
        bencher.iter(|| {
            let value = context.compile_source("bench.cue", SOURCE);
            black_box(value.is_ok());
        });
    });
}

fn bench_evaluator(criterion: &mut Criterion) {
    let context = Context::new();
    criterion.bench_function("evaluator/basic", |bencher| {
        bencher.iter(|| {
            if let Ok(value) = context.compile_source("bench.cue", SOURCE) {
                black_box(value.evaluate().is_ok());
            }
        });
    });
}

fn bench_export(criterion: &mut Criterion) {
    let context = Context::new();
    criterion.bench_function("export/json", |bencher| {
        bencher.iter(|| {
            if let Ok(value) = context.compile_source("bench.cue", SOURCE) {
                let mut options = EncodeOptions::default();
                options.encoding = Encoding::Json;
                black_box(encode_value(&value, options).is_ok());
            }
        });
    });
}

criterion_group!(
    benches,
    bench_parser,
    bench_compiler,
    bench_evaluator,
    bench_export
);
criterion_main!(benches);
