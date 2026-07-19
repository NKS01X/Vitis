use criterion::{black_box, criterion_group, criterion_main, Criterion};
use vitis::{convert_dockerfile, render_dry_run};
use vitis_core::VitisTask;

fn bench_convert_simple_dockerfile(c: &mut Criterion) {
    let df = "FROM ubuntu:24.04\nWORKDIR /workspace\nRUN apt-get update\nRUN apt-get install -y gcc\nCOPY . .\nRUN make\n";
    c.bench_function("convert_simple_dockerfile_6", |b| {
        b.iter(|| convert_dockerfile(black_box(df)))
    });
}

fn bench_convert_multistage_dockerfile(c: &mut Criterion) {
    let df = "\
FROM golang:1.21 AS builder
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -o /app .

FROM alpine:3.18
RUN apk add --no-cache ca-certificates
COPY --from=builder /app /usr/local/bin/app
EXPOSE 8080
CMD [\"app\"]
";
    c.bench_function("convert_multistage_dockerfile", |b| {
        b.iter(|| convert_dockerfile(black_box(df)))
    });
}

fn bench_convert_complex_dockerfile(c: &mut Criterion) {
    let df = "\
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
WORKDIR /app
RUN apt-get update && apt-get install -y gcc make cmake curl nodejs npm python3 pip
COPY backend/requirements.txt ./backend/
RUN pip install -r backend/requirements.txt
COPY backend/ ./backend/
RUN cd backend && python3 setup.py build
COPY frontend/package.json ./frontend/
RUN cd frontend && npm ci
COPY frontend/ ./frontend/
RUN cd frontend && npm run build
COPY scripts/ ./scripts/
RUN bash scripts/integrate.sh
LABEL version=\"1.0\"
EXPOSE 3000 8080
ENTRYPOINT [\"./run.sh\"]
CMD [\"--production\"]
";
    c.bench_function("convert_complex_dockerfile_18", |b| {
        b.iter(|| convert_dockerfile(black_box(df)))
    });
}

fn bench_render_dry_run_4(c: &mut Criterion) {
    let tasks = vec![
        VitisTask::new(0, "FROM ubuntu:24.04", 0, vec![]),
        VitisTask::new(1, "RUN apt-get install gcc", 2, vec![0]),
        VitisTask::new(2, "RUN apt-get install nodejs", 2, vec![0]),
        VitisTask::new(3, "RUN tar -czf release.tar.gz", 0, vec![1, 2]),
    ];
    c.bench_function("render_dry_run_4_tasks", |b| {
        b.iter(|| render_dry_run(black_box(&tasks)))
    });
}

fn bench_render_dry_run_20(c: &mut Criterion) {
    let mut tasks = vec![VitisTask::new(0, "FROM base", 0, vec![])];
    // 4 branches of 4 tasks each
    for branch in 0..4 {
        let base = branch * 4 + 1;
        tasks.push(VitisTask::new(base, &format!("RUN branch_{}_step_0", branch), 2, vec![0]));
        for step in 1..4 {
            tasks.push(VitisTask::new(
                base + step,
                &format!("RUN branch_{}_step_{}", branch, step),
                2,
                vec![base + step - 1],
            ));
        }
    }
    // merge
    let merge_deps: Vec<usize> = (0..4).map(|b| b * 4 + 4).collect();
    tasks.push(VitisTask::new(17, "RUN merge_all", 0, merge_deps));
    // 2 post-merge
    tasks.push(VitisTask::new(18, "RUN post_process", 0, vec![17]));
    tasks.push(VitisTask::new(19, "CMD start", 0, vec![18]));

    c.bench_function("render_dry_run_20_tasks", |b| {
        b.iter(|| render_dry_run(black_box(&tasks)))
    });
}

criterion_group!(
    benches,
    bench_convert_simple_dockerfile,
    bench_convert_multistage_dockerfile,
    bench_convert_complex_dockerfile,
    bench_render_dry_run_4,
    bench_render_dry_run_20,
);
criterion_main!(benches);
