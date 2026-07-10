// Exclusive tests for the `vitis-engine` crate: the Kahn topological scheduler.
//
// Requires the `fuse` feature (engine links the FUSE storage crate). Run with:
//     cargo test -p vitis-test-engine --features fuse
//
// Without the feature this compiles to nothing.

#![cfg(feature = "fuse")]

use std::sync::Arc;

use vitis_core::VitisTask;
use vitis_engine::{ExecutionEngine, SimExecutor, TaskExecutor};

struct OKExecutor;
impl TaskExecutor for OKExecutor {
    async fn execute(&self, _task: &VitisTask) -> vitis_engine::Result<()> {
        Ok(())
    }
}

struct FailOn {
    id: usize,
}
impl TaskExecutor for FailOn {
    async fn execute(&self, task: &VitisTask) -> vitis_engine::Result<()> {
        if task.id == self.id {
            Err(format!("task {} failed as requested", task.id).into())
        } else {
            Ok(())
        }
    }
}

struct TrackedExecutor {
    ran: Arc<std::sync::Mutex<Vec<usize>>>,
}
impl TaskExecutor for TrackedExecutor {
    async fn execute(&self, task: &VitisTask) -> vitis_engine::Result<()> {
        self.ran.lock().unwrap().push(task.id);
        Ok(())
    }
}

struct Recording<F> {
    inner: F,
    ran: Arc<std::sync::Mutex<Vec<usize>>>,
}
impl<F: TaskExecutor> TaskExecutor for Recording<F> {
    async fn execute(&self, task: &VitisTask) -> vitis_engine::Result<()> {
        self.ran.lock().unwrap().push(task.id);
        self.inner.execute(task).await
    }
}

fn task(id: usize, deps: Vec<usize>) -> VitisTask {
    VitisTask { id, instruction: format!("task_{id}"), indent_level: 0, depends_on: deps }
}

#[tokio::test]
async fn empty_task_list_succeeds() {
    let e = ExecutionEngine::new(vec![]);
    assert!(e.run(Arc::new(OKExecutor)).await.is_ok());
}

#[tokio::test]
async fn single_task() {
    let e = ExecutionEngine::new(vec![task(0, vec![])]);
    assert!(e.run(Arc::new(OKExecutor)).await.is_ok());
}

#[tokio::test]
async fn sequential_chain() {
    let tasks = vec![task(0, vec![]), task(1, vec![0]), task(2, vec![1])];
    assert!(ExecutionEngine::new(tasks).run(Arc::new(OKExecutor)).await.is_ok());
}

#[tokio::test]
async fn parallel_tasks() {
    let tasks = vec![task(0, vec![]), task(1, vec![0]), task(2, vec![0]), task(3, vec![1, 2])];
    assert!(ExecutionEngine::new(tasks).run(Arc::new(OKExecutor)).await.is_ok());
}

#[tokio::test]
async fn diamond_dag() {
    let tasks = vec![
        task(0, vec![]), task(1, vec![0]), task(2, vec![0]),
        task(3, vec![1, 2]), task(4, vec![3]),
    ];
    assert!(ExecutionEngine::new(tasks).run(Arc::new(OKExecutor)).await.is_ok());
}

#[tokio::test]
async fn failure_propagates() {
    let tasks = vec![task(0, vec![]), task(1, vec![0]), task(2, vec![1])];
    let r = ExecutionEngine::new(tasks).run(Arc::new(FailOn { id: 1 })).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("task 1 failed"));
}

#[tokio::test]
async fn failure_blocks_downstream() {
    let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tasks = vec![task(0, vec![]), task(1, vec![0]), task(2, vec![1])];
    let exec = Recording { inner: FailOn { id: 1 }, ran: ran.clone() };
    let _ = ExecutionEngine::new(tasks).run(Arc::new(exec)).await;
    let executed = ran.lock().unwrap().clone();
    assert!(executed.contains(&0));
    assert!(executed.contains(&1));
    assert!(!executed.contains(&2));
}

#[tokio::test]
async fn failure_in_one_branch_blocks_merge() {
    let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tasks = vec![task(0, vec![]), task(1, vec![0]), task(2, vec![0]), task(3, vec![1, 2])];
    let exec = Recording { inner: FailOn { id: 1 }, ran: ran.clone() };
    let r = ExecutionEngine::new(tasks).run(Arc::new(exec)).await;
    assert!(r.is_err());
    let executed = ran.lock().unwrap().clone();
    assert!(executed.contains(&0));
    assert!(executed.contains(&2));
    assert!(!executed.contains(&3));
}

#[tokio::test]
async fn sim_executor_works() {
    let tasks = vec![task(0, vec![]), task(1, vec![0]), task(2, vec![0]), task(3, vec![1, 2])];
    assert!(ExecutionEngine::new(tasks).run(Arc::new(SimExecutor::new())).await.is_ok());
}

#[tokio::test]
async fn complex_dag() {
    let tasks = vec![
        task(0, vec![]), task(1, vec![0]), task(2, vec![0]),
        task(3, vec![1]), task(4, vec![2]), task(5, vec![3, 4]),
    ];
    assert!(ExecutionEngine::new(tasks).run(Arc::new(OKExecutor)).await.is_ok());
}

#[tokio::test]
async fn topological_order_enforced() {
    let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
    let exec = TrackedExecutor { ran: ran.clone() };
    let tasks = vec![task(0, vec![]), task(1, vec![0]), task(2, vec![0]), task(3, vec![1, 2])];
    assert!(ExecutionEngine::new(tasks).run(Arc::new(exec)).await.is_ok());
    let executed = ran.lock().unwrap().clone();
    assert_eq!(executed[0], 0);
    assert_eq!(*executed.last().unwrap(), 3);
    assert!(executed.iter().position(|&x| x == 1).unwrap() < executed.iter().position(|&x| x == 3).unwrap());
    assert!(executed.iter().position(|&x| x == 2).unwrap() < executed.iter().position(|&x| x == 3).unwrap());
}

#[tokio::test]
async fn large_chain() {
    let mut tasks = vec![task(0, vec![])];
    for i in 1..100 {
        tasks.push(task(i, vec![i - 1]));
    }
    assert!(ExecutionEngine::new(tasks).run(Arc::new(OKExecutor)).await.is_ok());
}

#[tokio::test]
async fn parse_then_run_end_to_end() {
    let src = "FROM a\n  RUN b\n  RUN c\nRUN d\n";
    let tasks = vitis_parser::parse_vitisfile(src);
    assert!(ExecutionEngine::new(tasks).run(Arc::new(SimExecutor::new())).await.is_ok());
}

#[test]
fn engine_is_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<ExecutionEngine>();
}

#[test]
fn sim_executor_sleeps() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let e = SimExecutor::new();
        let start = std::time::Instant::now();
        e.execute(&task(0, vec![])).await.unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed >= std::time::Duration::from_millis(10));
        assert!(elapsed < std::time::Duration::from_millis(500));
    });
}
