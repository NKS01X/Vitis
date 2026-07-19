use criterion::{black_box, criterion_group, criterion_main, Criterion};
use vitis_parser::parse_vitisfile;

fn bench_parse_simple_sequential(c: &mut Criterion) {
    let input = "FROM ubuntu:24.04\nRUN apt-get update\nRUN apt-get install -y gcc\nCOPY . .\nRUN make\n";
    c.bench_function("parse_simple_sequential_5", |b| {
        b.iter(|| parse_vitisfile(black_box(input)))
    });
}

fn bench_parse_2branch_parallel(c: &mut Criterion) {
    let input = "\
FROM ubuntu:24.04
WORKDIR /workspace

    RUN apt-get update && apt-get install -y gcc
    COPY ./backend ./backend
    RUN make build-backend

    RUN apt-get update && apt-get install -y nodejs
    COPY ./frontend ./frontend
    RUN npm run build

RUN tar -czf release.tar.gz ./backend/dist ./frontend/dist
";
    c.bench_function("parse_2branch_parallel_9", |b| {
        b.iter(|| parse_vitisfile(black_box(input)))
    });
}

fn bench_parse_3branch_parallel(c: &mut Criterion) {
    let input = "\
FROM python:3.11-slim
RUN apt-get update

    RUN pip install pandas numpy
    COPY ./data ./data
    RUN python prepare_data.py

    RUN pip install scikit-learn torch
    COPY ./train ./train
    RUN python train/train_model.py --epochs 20

    RUN pip install matplotlib seaborn
    COPY ./evaluate ./evaluate
    RUN python evaluate/metrics.py

RUN python package.py && tar -czf ml_pipeline.tar.gz model/ metrics/ prepared_data/
";
    c.bench_function("parse_3branch_parallel_14", |b| {
        b.iter(|| parse_vitisfile(black_box(input)))
    });
}

fn bench_parse_large_100_tasks(c: &mut Criterion) {
    let mut input = String::from("FROM ubuntu:24.04\n");
    for i in 0..99 {
        input.push_str(&format!("RUN echo step_{}\n", i));
    }
    c.bench_function("parse_chain_100_tasks", |b| {
        b.iter(|| parse_vitisfile(black_box(&input)))
    });
}

fn bench_parse_deep_nesting(c: &mut Criterion) {
    let mut input = String::from("FROM base\n");
    for i in 0..10 {
        let indent = "  ".repeat(i + 1);
        input.push_str(&format!("{}RUN level_{}\n", indent, i));
    }
    c.bench_function("parse_deep_nesting_10_levels", |b| {
        b.iter(|| parse_vitisfile(black_box(&input)))
    });
}

fn bench_parse_wide_fan(c: &mut Criterion) {
    // 1 root → 20 independent branches → 1 merge
    let mut input = String::from("FROM base\n\n");
    for i in 0..20 {
        input.push_str(&format!("    RUN branch_{}\n\n", i));
    }
    input.push_str("RUN merge_all\n");
    c.bench_function("parse_wide_fan_20_branches", |b| {
        b.iter(|| parse_vitisfile(black_box(&input)))
    });
}

criterion_group!(
    benches,
    bench_parse_simple_sequential,
    bench_parse_2branch_parallel,
    bench_parse_3branch_parallel,
    bench_parse_large_100_tasks,
    bench_parse_deep_nesting,
    bench_parse_wide_fan,
);
criterion_main!(benches);
