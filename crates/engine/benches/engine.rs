use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::Arc;
use vitis_core::VitisTask;
use vitis_engine::{ExecutionEngine, SimExecutor, TaskExecutor};

struct InstantExecutor;
impl TaskExecutor for InstantExecutor {
    async fn execute(&self, _task: &VitisTask) -> vitis_engine::Result<()> {
        Ok(())
    }
}

fn make_task(id: usize, deps: Vec<usize>) -> VitisTask {
    VitisTask {
        id,
        instruction: format!("task_{}", id),
        indent_level: 0,
        depends_on: deps,
    }
}

fn bench_schedule_diamond_4(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("schedule_diamond_4_tasks", |b| {
        b.iter(|| {
            let tasks = vec![
                make_task(0, vec![]),
                make_task(1, vec![0]),
                make_task(2, vec![0]),
                make_task(3, vec![1, 2]),
            ];
            let engine = ExecutionEngine::new(tasks);
            rt.block_on(engine.run(Arc::new(InstantExecutor))).unwrap();
        })
    });
}

fn bench_schedule_2branch_9(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("schedule_2branch_9_tasks", |b| {
        b.iter(|| {
            let tasks = vec![
                make_task(0, vec![]),    // FROM
                make_task(1, vec![0]),   // WORKDIR
                make_task(2, vec![1]),   // branch1: install gcc
                make_task(3, vec![2]),   // branch1: COPY
                make_task(4, vec![3]),   // branch1: make
                make_task(5, vec![1]),   // branch2: install nodejs
                make_task(6, vec![5]),   // branch2: COPY
                make_task(7, vec![6]),   // branch2: npm build
                make_task(8, vec![4, 7]), // merge: tar
            ];
            let engine = ExecutionEngine::new(tasks);
            rt.block_on(engine.run(Arc::new(InstantExecutor))).unwrap();
        })
    });
}

fn bench_schedule_wide_fan_10(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("schedule_fan_10_branches", |b| {
        b.iter(|| {
            let mut tasks = vec![make_task(0, vec![])];
            for i in 1..=10 {
                tasks.push(make_task(i, vec![0]));
            }
            let merge_deps: Vec<usize> = (1..=10).collect();
            tasks.push(make_task(11, merge_deps));
            let engine = ExecutionEngine::new(tasks);
            rt.block_on(engine.run(Arc::new(InstantExecutor))).unwrap();
        })
    });
}

fn bench_schedule_chain_100(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("schedule_chain_100_tasks", |b| {
        b.iter(|| {
            let mut tasks = vec![make_task(0, vec![])];
            for i in 1..100 {
                tasks.push(make_task(i, vec![i - 1]));
            }
            let engine = ExecutionEngine::new(tasks);
            rt.block_on(engine.run(Arc::new(InstantExecutor))).unwrap();
        })
    });
}

fn bench_schedule_complex_50(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("schedule_complex_dag_50", |b| {
        b.iter(|| {
            let mut tasks = vec![make_task(0, vec![])];
            // 5 branches of 9 tasks each, all merging
            for branch in 0..5 {
                let base = branch * 9 + 1;
                tasks.push(make_task(base, vec![0]));
                for j in 1..9 {
                    tasks.push(make_task(base + j, vec![base + j - 1]));
                }
            }
            // merge point depends on last task of each branch
            let merge_deps: Vec<usize> = (0..5).map(|b| b * 9 + 9).collect();
            tasks.push(make_task(46, merge_deps));
            let engine = ExecutionEngine::new(tasks);
            rt.block_on(engine.run(Arc::new(InstantExecutor))).unwrap();
        })
    });
}

fn bench_schedule_sim_executor(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("schedule_sim_executor_diamond", |b| {
        b.iter(|| {
            let tasks = vec![
                make_task(0, vec![]),
                make_task(1, vec![0]),
                make_task(2, vec![0]),
                make_task(3, vec![1, 2]),
            ];
            let engine = ExecutionEngine::new(tasks);
            rt.block_on(engine.run(Arc::new(SimExecutor::new()))).unwrap();
        })
    });
}

criterion_group!(
    benches,
    bench_schedule_diamond_4,
    bench_schedule_2branch_9,
    bench_schedule_wide_fan_10,
    bench_schedule_chain_100,
    bench_schedule_complex_50,
    bench_schedule_sim_executor,
);
criterion_main!(benches);
