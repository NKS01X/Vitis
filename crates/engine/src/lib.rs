#![allow(dead_code)]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use fuser::BackgroundSession;
use tokio::sync::{mpsc, RwLock};
use vitis_core::VitisTask;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, BoxError>;

pub trait TaskExecutor: Send + Sync {
    fn execute(&self, task: &VitisTask) -> impl std::future::Future<Output = Result<()>> + Send;
}

#[derive(Clone)]
pub struct SandboxConfig {
    pub registry_url: String,
    pub image: String,
    pub tag: String,
    pub base_dir: PathBuf,
    pub cache_dir: PathBuf,
}

pub struct SandboxExecutor {
    config: Arc<SandboxConfig>,
}

impl SandboxExecutor {
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl TaskExecutor for SandboxExecutor {
    async fn execute(&self, task: &VitisTask) -> Result<()> {
        run_sandboxed_task(task, &self.config).await
    }
}

pub async fn run_worker(
    task: VitisTask,
    config: Arc<SandboxConfig>,
    tx: mpsc::Sender<Result<usize>>,
) {
    if tx.is_closed() {
        return;
    }
    let id = task.id;
    let result = run_sandboxed_task(&task, &config).await;
    let _ = tx.send(result.map(|_| id)).await;
}

#[tracing::instrument(skip(task, config), fields(task_id = task.id))]
async fn run_sandboxed_task(task: &VitisTask, config: &SandboxConfig) -> Result<()> {
    let task_dir = config.base_dir.join(format!("task_{}", task.id));
    let mount_path = task_dir.join("rootfs");
    let mount_str = mount_path.to_string_lossy().to_string();
    let cache_str = config.cache_dir.to_string_lossy().to_string();

    tokio::fs::create_dir_all(&mount_path)
        .await
        .map_err(|e| format!("failed to create task dir {}: {}", task_dir.display(), e))?;

    let _session: BackgroundSession = vitis_storage::mount_and_pull(
        &config.registry_url,
        &config.image,
        &config.tag,
        &mount_str,
        &cache_str,
    )
    .await
    .map_err(|e| format!("mount failed for task {}: {}", task.id, e))?;

    let instruction = task.instruction.clone();
    let mount_path_clone = mount_path.clone();

    let sandbox_result = tokio::task::spawn_blocking(move || {
        vitis_runtime::run_sandboxed(&instruction, &mount_path_clone)
    })
    .await
    .map_err(|e| format!("spawn_blocking panicked for task {}: {}", task.id, e))?;

    drop(_session);

    let _ = tokio::fs::remove_dir_all(&task_dir).await;

    match sandbox_result {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => {
            Err(format!("task {} exited with non-zero status: {}", task.id, status).into())
        }
        Err(e) => Err(format!("task {} sandbox error: {}", task.id, e).into()),
    }
}

pub struct SimExecutor;

impl SimExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SimExecutor {
    fn default() -> Self {
        Self
    }
}

impl TaskExecutor for SimExecutor {
    async fn execute(&self, _task: &VitisTask) -> Result<()> {
        let ms = ((_task.id as u64) % 3 + 1) * 10;
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        Ok(())
    }
}

pub struct ExecutionEngine {
    tasks: Arc<HashMap<usize, VitisTask>>,
    downstream: Arc<HashMap<usize, Vec<usize>>>,
    in_degree: Arc<RwLock<HashMap<usize, usize>>>,
    from_roots: Arc<Vec<usize>>,
}

impl ExecutionEngine {
    pub fn new(tasks: Vec<VitisTask>) -> Self {
        let mut task_map: HashMap<usize, VitisTask> = HashMap::new();
        let mut downstream: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut in_degree: HashMap<usize, usize> = HashMap::new();
        let mut from_roots: Vec<usize> = Vec::new();

        for task in &tasks {
            let id = task.id;
            task_map.insert(id, task.clone());
            downstream.entry(id).or_default();
            in_degree.entry(id).or_insert(0);
        }

        for task in &tasks {
            for dep_id in &task.depends_on {
                downstream.entry(*dep_id).or_default().push(task.id);
                *in_degree.entry(task.id).or_insert(0) += 1;
            }
            if task.instruction.to_uppercase().starts_with("FROM") {
                from_roots.push(task.id);
            }
        }

        Self {
            tasks: Arc::new(task_map),
            downstream: Arc::new(downstream),
            in_degree: Arc::new(RwLock::new(in_degree)),
            from_roots: Arc::new(from_roots),
        }
    }

    #[tracing::instrument(skip(self, executor))]
    pub async fn run<F>(&self, executor: Arc<F>) -> Result<()>
    where
        F: TaskExecutor + 'static,
    {
        let total = self.tasks.len();
        if total == 0 {
            return Ok(());
        }

        let (tx, mut rx) = mpsc::channel::<Result<usize>>(total.max(1));
        let mut pending = total;
        let mut in_flight: usize = 0;

        {
            let guard = self.in_degree.read().await;
            for (&id, &deg) in guard.iter() {
                if deg == 0 {
                    spawn_one(id, self.tasks.clone(), executor.clone(), tx.clone());
                    in_flight += 1;
                }
            }
        }

        if in_flight == 0 && pending > 0 {
            return Err(
                "deadlock detected: no runnable tasks, yet tasks remain (cyclic dependency?)"
                    .into(),
            );
        }

        while let Some(result) = rx.recv().await {
            match result {
                Ok(id) => {
                    pending -= 1;
                    in_flight -= 1;

                    if pending == 0 {
                        return Ok(());
                    }

                    let mut fresh: Vec<usize> = Vec::new();
                    {
                        let mut guard = self.in_degree.write().await;
                        if let Some(dependents) = self.downstream.get(&id) {
                            for &dep_id in dependents {
                                let deg = guard.get_mut(&dep_id).unwrap();
                                *deg = deg.saturating_sub(1);
                                if *deg == 0 {
                                    fresh.push(dep_id);
                                }
                            }
                        }
                    }

                    for next_id in fresh {
                        spawn_one(next_id, self.tasks.clone(), executor.clone(), tx.clone());
                        in_flight += 1;
                    }
                }
                Err(e) => {
                    return Err(e);
                }
            }

            if pending > 0 && in_flight == 0 {
                return Err("deadlock detected: runnable queue emptied before all tasks completed (cyclic dependency?)".into());
            }
        }

        Ok(())
    }

    pub async fn run_sandboxed(&self, config: SandboxConfig) -> Result<()> {
        let total = self.tasks.len();
        if total == 0 {
            return Ok(());
        }

        let (tx, mut rx) = mpsc::channel::<Result<usize>>(total.max(1));
        let mut pending = total;
        let mut in_flight: usize = 0;
        let config = Arc::new(config);

        {
            let guard = self.in_degree.read().await;
            for (&id, &deg) in guard.iter() {
                if deg == 0 && self.from_roots.contains(&id) {
                    let task = self.tasks.get(&id).cloned().unwrap();
                    let cfg = config.clone();
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        run_worker(task, cfg, tx_clone).await;
                    });
                    in_flight += 1;
                }
            }
        }

        if in_flight == 0 && pending > 0 {
            return Err("deadlock detected: no runnable FROM roots, yet tasks remain".into());
        }

        while let Some(result) = rx.recv().await {
            match result {
                Ok(id) => {
                    pending -= 1;
                    in_flight -= 1;

                    if pending == 0 {
                        return Ok(());
                    }

                    let mut fresh: Vec<usize> = Vec::new();
                    {
                        let mut guard = self.in_degree.write().await;
                        if let Some(dependents) = self.downstream.get(&id) {
                            for &dep_id in dependents {
                                let deg = guard.get_mut(&dep_id).unwrap();
                                *deg = deg.saturating_sub(1);
                                if *deg == 0 {
                                    fresh.push(dep_id);
                                }
                            }
                        }
                    }

                    for next_id in fresh {
                        let task = self.tasks.get(&next_id).cloned().unwrap();
                        let cfg = config.clone();
                        let tx_clone = tx.clone();
                        tokio::spawn(async move {
                            run_worker(task, cfg, tx_clone).await;
                        });
                        in_flight += 1;
                    }
                }
                Err(e) => {
                    return Err(e);
                }
            }

            if pending > 0 && in_flight == 0 {
                return Err(
                    "deadlock detected: runnable queue emptied before all tasks completed".into(),
                );
            }
        }

        Ok(())
    }
}

fn spawn_one<F>(
    task_id: usize,
    tasks: Arc<HashMap<usize, VitisTask>>,
    executor: Arc<F>,
    tx: mpsc::Sender<Result<usize>>,
) where
    F: TaskExecutor + 'static,
{
    let task = match tasks.get(&task_id) {
        Some(t) => t.clone(),
        None => {
            let _ = tx.try_send(Err(format!("task {task_id} not found").into()));
            return;
        }
    };

    tokio::spawn(async move {
        let result = executor.execute(&task).await.map(|_| task_id);
        let _ = tx.send(result).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct OKExecutor;
    impl TaskExecutor for OKExecutor {
        async fn execute(&self, _task: &VitisTask) -> Result<()> {
            Ok(())
        }
    }

    struct FailOn {
        id: usize,
    }
    impl TaskExecutor for FailOn {
        async fn execute(&self, task: &VitisTask) -> Result<()> {
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
        async fn execute(&self, task: &VitisTask) -> Result<()> {
            self.ran.lock().unwrap().push(task.id);
            Ok(())
        }
    }

    struct Recording<F> {
        inner: F,
        ran: Arc<std::sync::Mutex<Vec<usize>>>,
    }
    impl<F: TaskExecutor> TaskExecutor for Recording<F> {
        async fn execute(&self, task: &VitisTask) -> Result<()> {
            self.ran.lock().unwrap().push(task.id);
            self.inner.execute(task).await
        }
    }

    fn make_task(id: usize, deps: Vec<usize>) -> VitisTask {
        VitisTask {
            id,
            instruction: format!("task_{id}"),
            indent_level: 0,
            depends_on: deps,
        }
    }

    fn make_task_with_instruction(id: usize, instruction: &str, deps: Vec<usize>) -> VitisTask {
        VitisTask {
            id,
            instruction: instruction.into(),
            indent_level: 0,
            depends_on: deps,
        }
    }

    #[tokio::test]
    async fn empty_task_list() {
        let engine = ExecutionEngine::new(vec![]);
        assert!(engine.run(Arc::new(OKExecutor)).await.is_ok());
    }

    #[tokio::test]
    async fn single_task() {
        let engine = ExecutionEngine::new(vec![make_task(0, vec![])]);
        assert!(engine.run(Arc::new(OKExecutor)).await.is_ok());
    }

    #[tokio::test]
    async fn sequential_chain() {
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![1]),
        ];
        assert!(ExecutionEngine::new(tasks)
            .run(Arc::new(OKExecutor))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn parallel_tasks() {
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![0]),
            make_task(3, vec![1, 2]),
        ];
        assert!(ExecutionEngine::new(tasks)
            .run(Arc::new(OKExecutor))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn diamond_dag() {
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![0]),
            make_task(3, vec![1, 2]),
            make_task(4, vec![3]),
        ];
        assert!(ExecutionEngine::new(tasks)
            .run(Arc::new(OKExecutor))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn task_failure_propagates() {
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![1]),
        ];
        let r = ExecutionEngine::new(tasks)
            .run(Arc::new(FailOn { id: 1 }))
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("task 1 failed"));
    }

    #[tokio::test]
    async fn failure_does_not_spawn_downstream() {
        let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![1]),
        ];
        let executor = Recording {
            inner: FailOn { id: 1 },
            ran: ran.clone(),
        };
        let _ = ExecutionEngine::new(tasks).run(Arc::new(executor)).await;
        let executed = ran.lock().unwrap().clone();
        assert!(executed.contains(&0));
        assert!(executed.contains(&1));
        assert!(!executed.contains(&2));
    }

    #[tokio::test]
    async fn failure_parallel_branch() {
        let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![0]),
            make_task(3, vec![1, 2]),
        ];
        let executor = Recording {
            inner: FailOn { id: 1 },
            ran: ran.clone(),
        };
        let r = ExecutionEngine::new(tasks).run(Arc::new(executor)).await;
        assert!(r.is_err());
        let executed = ran.lock().unwrap().clone();
        assert!(executed.contains(&0));
        assert!(executed.contains(&2));
        assert!(!executed.contains(&3));
    }

    #[tokio::test]
    async fn sim_executor_works() {
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![0]),
            make_task(3, vec![1, 2]),
        ];
        assert!(ExecutionEngine::new(tasks)
            .run(Arc::new(SimExecutor::new()))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn complex_dag() {
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![0]),
            make_task(3, vec![1]),
            make_task(4, vec![2]),
            make_task(5, vec![3, 4]),
        ];
        assert!(ExecutionEngine::new(tasks)
            .run(Arc::new(OKExecutor))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn tasks_executed_in_topological_order() {
        let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
        let executor = TrackedExecutor { ran: ran.clone() };
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![0]),
            make_task(3, vec![1, 2]),
        ];
        assert!(ExecutionEngine::new(tasks)
            .run(Arc::new(executor))
            .await
            .is_ok());
        let executed = ran.lock().unwrap().clone();
        assert_eq!(executed[0], 0);
        assert_eq!(*executed.last().unwrap(), 3);
        let p1 = executed.iter().position(|&x| x == 1).unwrap();
        let p2 = executed.iter().position(|&x| x == 2).unwrap();
        let p3 = executed.iter().position(|&x| x == 3).unwrap();
        assert!(p1 < p3);
        assert!(p2 < p3);
    }

    #[tokio::test]
    async fn large_dag_performance() {
        let mut tasks = vec![make_task(0, vec![])];
        for i in 1..100 {
            tasks.push(make_task(i, vec![i - 1]));
        }
        assert!(ExecutionEngine::new(tasks)
            .run(Arc::new(OKExecutor))
            .await
            .is_ok());
    }

    #[test]
    fn engine_construction_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<ExecutionEngine>();
    }

    #[test]
    fn sandbox_config_creation() {
        let c = SandboxConfig {
            registry_url: "https://registry.example.com".into(),
            image: "ubuntu".into(),
            tag: "24.04".into(),
            base_dir: PathBuf::from("/tmp/vitis"),
            cache_dir: PathBuf::from("/tmp/vitis/cache"),
        };
        assert_eq!(c.image, "ubuntu");
    }

    #[test]
    fn sandbox_config_clone() {
        let a = SandboxConfig {
            registry_url: "url".into(),
            image: "img".into(),
            tag: "tag".into(),
            base_dir: PathBuf::from("/a"),
            cache_dir: PathBuf::from("/b"),
        };
        let b = a.clone();
        assert_eq!(a.registry_url, b.registry_url);
    }

    #[test]
    fn sandbox_executor_creation() {
        let c = SandboxConfig {
            registry_url: "url".into(),
            image: "img".into(),
            tag: "tag".into(),
            base_dir: PathBuf::from("/tmp"),
            cache_dir: PathBuf::from("/tmp/cache"),
        };
        let _: Arc<SandboxExecutor> = Arc::new(SandboxExecutor::new(c));
    }

    #[tokio::test]
    async fn run_worker_closed_channel() {
        let config = Arc::new(SandboxConfig {
            registry_url: "url".into(),
            image: "img".into(),
            tag: "tag".into(),
            base_dir: PathBuf::from("/tmp"),
            cache_dir: PathBuf::from("/tmp/cache"),
        });
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        run_worker(make_task(0, vec![]), config, tx).await;
    }

    #[tokio::test]
    async fn run_sandboxed_only_seeds_from_roots() {
        let tasks = vec![
            make_task_with_instruction(0, "FROM ubuntu", vec![]),
            make_task_with_instruction(1, "RUN echo hello", vec![0]),
        ];
        let engine = ExecutionEngine::new(tasks);
        assert_eq!(engine.from_roots.len(), 1);
        assert!(engine.from_roots.contains(&0));
        assert!(!engine.from_roots.contains(&1));
    }

    #[tokio::test]
    async fn sim_executor_sleeps_correct_duration() {
        let e = SimExecutor::new();
        let task = make_task(0, vec![]);
        let start = std::time::Instant::now();
        e.execute(&task).await.unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed >= std::time::Duration::from_millis(10));
        assert!(elapsed < std::time::Duration::from_millis(500));
    }

    #[tokio::test]
    async fn failure_halts_all_downstream_execution() {
        let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tasks = vec![
            make_task(0, vec![]),
            make_task(1, vec![0]),
            make_task(2, vec![1]),
            make_task(3, vec![1]),
            make_task(4, vec![2, 3]),
        ];
        let executor = Recording {
            inner: FailOn { id: 1 },
            ran: ran.clone(),
        };
        let r = ExecutionEngine::new(tasks).run(Arc::new(executor)).await;
        assert!(r.is_err());
        let executed = ran.lock().unwrap().clone();
        assert!(executed.contains(&0));
        assert!(executed.contains(&1));
        assert!(!executed.contains(&2));
        assert!(!executed.contains(&3));
        assert!(!executed.contains(&4));
    }
}
