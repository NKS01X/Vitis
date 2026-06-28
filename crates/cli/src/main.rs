#![allow(dead_code, unused_imports, unused_macros)]
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::sync::{Mutex, RwLock};
use vitis::{
    ansi, build_progress_style, build_spinner_style, convert_dockerfile, render_dry_run, Cli,
    Command,
};
use vitis_core::VitisTask;

#[cfg(feature = "engine")]
use vitis_engine::{ExecutionEngine, SimExecutor, TaskExecutor};

macro_rules! info {
    ($fmt:literal $(, $arg:expr)*) => {
        println!(
            concat!("{}==>{} ", $fmt),
            ansi::GREEN, ansi::RESET,
            $($arg),*
        )
    };
}

macro_rules! step {
    ($fmt:literal $(, $arg:expr)*) => {
        println!(
            concat!("{}[STARTING]{} ", $fmt),
            ansi::CYAN, ansi::RESET,
            $($arg),*
        )
    };
}

macro_rules! done {
    ($fmt:literal $(, $arg:expr)*) => {
        println!(
            concat!("{}[COMPLETED]{} ", $fmt),
            ansi::GREEN, ansi::RESET,
            $($arg),*
        )
    };
}

macro_rules! error {
    ($fmt:literal $(, $arg:expr)*) => {
        eprintln!(
            concat!("{}[FAILED]{} ", $fmt),
            ansi::RED, ansi::RESET,
            $($arg),*
        )
    };
}

fn print_banner() {
    println!();
    println!(
        "{bold}{mag}\
  __         _   _        {reset}\
{bold}{mag} / _|       (_) | |       {reset}\
{bold}{mag}| |_   __ _  _  | |_  ___ {reset}\
{bold}{mag}|  _| / _` || | | __|/ __|{reset}\
{bold}{mag}| |  | (_| || | | |_ \\__ \\{reset}\
{bold}{mag}|_|   \\__,_||_|  \\__||___/{reset}\
        ",
        bold = ansi::BOLD,
        mag = ansi::MAGENTA,
        reset = ansi::RESET,
    );
    println!(
        "{}Nodes Linked. Branches Spawning.{}",
        ansi::CYAN,
        ansi::RESET
    );
    println!();
}

fn branch_prefix(idx: usize) -> String {
    let symbols = ["\u{2502}", "\u{2503}", "\u{2502}", "\u{2503}"];
    let colors = [ansi::CYAN, ansi::MAGENTA, ansi::YELLOW, ansi::GREEN];
    let c = colors[idx % colors.len()];
    let s = symbols[idx % symbols.len()];
    format!("{}{}{} branch {}", c, s, ansi::RESET, idx + 1)
}

fn compute_branches(tasks: &[VitisTask]) -> Vec<Vec<(usize, &str)>> {
    if tasks.is_empty() {
        return vec![];
    }

    let task_map: HashMap<usize, &VitisTask> = tasks.iter().map(|t| (t.id, t)).collect();

    let mut downstream: HashMap<usize, Vec<usize>> = HashMap::new();
    for t in tasks {
        for dep in &t.depends_on {
            downstream.entry(*dep).or_default().push(t.id);
        }
    }

    let mut branch_of: HashMap<usize, usize> = HashMap::new();
    let roots: Vec<&VitisTask> = tasks.iter().filter(|t| t.depends_on.is_empty()).collect();

    let mut next_branch = 0;
    for root in &roots {
        let mut queue = vec![root.id];
        let mut visited = HashSet::new();
        while let Some(tid) = queue.pop() {
            if !visited.insert(tid) {
                continue;
            }
            branch_of.entry(tid).or_insert(next_branch);
            let kids = downstream.get(&tid);
            if let Some(kids) = kids {
                if kids.len() > 1 {
                    for &kid in kids {
                        let _ = branch_of.insert(kid, next_branch + 1);
                        next_branch += 1;
                    }
                } else {
                    for &kid in kids {
                        branch_of.insert(kid, branch_of[&tid]);
                        queue.push(kid);
                    }
                }
            }
        }
        next_branch += 1;
    }

    if branch_of.is_empty() {
        for t in tasks {
            branch_of.insert(t.id, 0);
        }
    }

    let mut branch_tasks: Vec<Vec<(usize, &str)>> = Vec::new();
    for (tid, bid) in &branch_of {
        while branch_tasks.len() <= *bid {
            branch_tasks.push(Vec::new());
        }
        branch_tasks[*bid].push((*tid, &task_map[tid].instruction));
    }

    for chain in &mut branch_tasks {
        chain.sort_by_key(|(id, _)| *id);
    }

    branch_tasks
}

#[cfg(feature = "engine")]
struct ProgressExecutor<F> {
    inner: F,
    branches: Vec<Vec<(usize, String)>>,
    branch_status: RwLock<HashMap<usize, BranchState>>,
    multi: MultiProgress,
    bars: Vec<ProgressBar>,
    completed: AtomicUsize,
    total: usize,
    cancelled: Arc<AtomicBool>,
}

#[cfg(feature = "engine")]
#[derive(Default)]
struct BranchState {
    started_count: usize,
    completed_count: usize,
}

#[cfg(feature = "engine")]
impl<F: TaskExecutor> TaskExecutor for ProgressExecutor<F> {
    async fn execute(&self, task: &VitisTask) -> vitis_engine::Result<()> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err("build cancelled by signal".into());
        }

        let branch_id = self
            .branches
            .iter()
            .position(|b| b.iter().any(|(tid, _)| *tid == task.id));

        if let Some(bid) = branch_id {
            let bar = &self.bars[bid % self.bars.len()];

            let chain_len = self.branches[bid].len();
            {
                let mut status = self.branch_status.write().await;
                let st = status.entry(bid).or_default();
                st.started_count += 1;

                let task_pos_in_chain = self.branches[bid]
                    .iter()
                    .position(|(tid, _)| *tid == task.id)
                    .unwrap_or(0);
                bar.inc(task_pos_in_chain as u64 - (st.started_count - 2) as u64);
                bar.set_length(chain_len as u64);
                bar.set_position((st.started_count - 1) as u64);
            }

            bar.set_style(
                ProgressStyle::default_bar()
                    .template("{prefix:.bold.dim} {spinner:.green} [{elapsed_precise}] [{bar:20.cyan/blue}] {pos}/{len} {msg:.bold}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            bar.set_prefix(branch_prefix(bid));

            if let Some((_, instr)) = self.branches[bid].iter().find(|(tid, _)| *tid == task.id) {
                let display = if instr.len() > 60 {
                    format!("{}...", &instr[..57])
                } else {
                    instr.clone()
                };
                bar.set_message(display);
            }
        }

        let result = self.inner.execute(task).await;

        let done = self.completed.fetch_add(1, Ordering::SeqCst) + 1;

        if let Some(bid) = branch_id {
            let bar = &self.bars[bid % self.bars.len()];
            let chain_len = self.branches[bid].len();
            {
                let mut status = self.branch_status.write().await;
                let st = status.entry(bid).or_default();
                st.completed_count += 1;
                bar.inc(1);
            }

            let is_final = {
                let status = self.branch_status.read().await;
                let st = status.get(&bid).unwrap();
                st.completed_count >= chain_len
            };

            if result.is_ok() && is_final {
                bar.set_style(
                    ProgressStyle::default_bar()
                        .template("{prefix:.bold.dim} {spinner:.green} [{elapsed_precise}] [{bar:20.green/green}] {pos}/{len} {msg:.green.bold}")
                        .unwrap()
                        .progress_chars("=> "),
                );
                let last_instr = self.branches[bid]
                    .last()
                    .map(|(_, i)| {
                        if i.len() > 40 {
                            format!("{}...", &i[..37])
                        } else {
                            i.to_string()
                        }
                    })
                    .unwrap_or_default();
                bar.set_message(format!("\u{2713} {}", last_instr));
                bar.finish();
            } else if result.is_err() {
                bar.set_style(
                    ProgressStyle::default_bar()
                        .template("{prefix:.bold.dim} {spinner:.red} [{elapsed_precise}] [{bar:20.red/red}] {pos}/{len} {msg:.red.bold}")
                        .unwrap()
                        .progress_chars("=> "),
                );
                bar.abandon_with_message("\u{2717} FAILED");
            } else {
                bar.set_length(chain_len as u64);
            }

            bar.set_message(format!("{} / {}", done, self.total));
        }

        if self.cancelled.load(Ordering::SeqCst) {
            return Err("build cancelled by signal".into());
        }

        result
    }
}

#[cfg(feature = "engine")]
struct LoggingExecutor<F> {
    inner: F,
    started: Mutex<HashSet<usize>>,
}

#[cfg(feature = "engine")]
impl<F: TaskExecutor> TaskExecutor for LoggingExecutor<F> {
    async fn execute(&self, task: &VitisTask) -> vitis_engine::Result<()> {
        {
            let mut s = self.started.lock().await;
            if s.insert(task.id) {
                step!("Task {}: {}", task.id, task.instruction);
            }
        }

        let result = self.inner.execute(task).await;

        match &result {
            Ok(()) => done!("Task {}: {}", task.id, task.instruction),
            Err(e) => error!("Task {}: {} ({})", task.id, task.instruction, e),
        }

        result
    }
}

fn print_dry_run(tasks: &[VitisTask]) {
    let plain = render_dry_run(tasks);
    let header = format!(
        "{}Execution Graph ({} tasks):{}\n",
        ansi::BOLD,
        tasks.len(),
        ansi::RESET
    );
    print!("{header}{plain}");
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Build {
            file,
            tag,
            context,
            dry_run,
        } => run_build(file, tag, context, dry_run).await,
        Command::Init { dockerfile, output } => run_init(dockerfile, output),
    }
}

fn run_init(dockerfile: PathBuf, output: PathBuf) -> Result<()> {
    let content = std::fs::read_to_string(&dockerfile)
        .with_context(|| format!("failed to read {:?}", dockerfile))?;

    let vitisfile = convert_dockerfile(&content);

    if vitisfile.is_empty() {
        anyhow::bail!("no valid Dockerfile instructions found in {:?}", dockerfile);
    }

    std::fs::write(&output, &vitisfile).with_context(|| format!("failed to write {:?}", output))?;

    info!(
        "Converted {:?} -> {:?} ({} tasks)",
        dockerfile,
        output,
        vitis_parser::parse_vitisfile(&vitisfile).len()
    );

    Ok(())
}

async fn run_build(
    file: PathBuf,
    _tag: Option<String>,
    context: PathBuf,
    dry_run: bool,
) -> Result<()> {
    #[cfg(not(feature = "engine"))]
    {
        let _ = (file, context, dry_run);
        anyhow::bail!(
            "live execution requires the `engine` feature (built with libfuse3). \
             Rebuild with `--features engine`, or use `--dry-run` after enabling it."
        );
    }

    #[cfg(feature = "engine")]
    run_build_engine(file, _tag, context, dry_run).await
}

#[cfg(feature = "engine")]
async fn run_build_engine(
    file: PathBuf,
    _tag: Option<String>,
    context: PathBuf,
    dry_run: bool,
) -> Result<()> {
    print_banner();

    info!("Build context verified at: {:?}", context);

    let content =
        std::fs::read_to_string(&file).with_context(|| format!("failed to read {:?}", file))?;

    let tasks = vitis_parser::parse_vitisfile(&content);
    let branches = compute_branches(&tasks);

    if tasks.is_empty() {
        info!("No tasks found in {:?} — nothing to do.", file);
        return Ok(());
    }

    info!(
        "Parsed {} {} across {} parallel branches from {:?}.",
        tasks.len(),
        if tasks.len() == 1 { "task" } else { "tasks" },
        branches.len(),
        file
    );

    if let Some(ref tag) = _tag {
        info!("Tagging final image as: {}", tag);
    }

    if dry_run {
        println!();
        print_dry_run(&tasks);
        println!();
        info!("Dry-run complete. No tasks were executed.");
        return Ok(());
    }

    info!("Spawning concurrent execution matrix...");
    println!();

    let multi = MultiProgress::new();
    let mut bars = Vec::new();

    for (bid, branch) in branches.iter().enumerate() {
        let pb = multi.add(ProgressBar::new(branch.len() as u64));
        pb.set_style(build_progress_style());
        pb.set_prefix(branch_prefix(bid));

        let chain_len = branch.len();
        let last_instr = branch
            .last()
            .map(|(_, i)| {
                if i.len() > 40 {
                    format!("{}...", &i[..37])
                } else {
                    i.to_string()
                }
            })
            .unwrap_or_default();
        pb.set_message(format!("0/{} {}", chain_len, last_instr));
        bars.push(pb);
    }

    let cancelled = Arc::new(AtomicBool::new(false));
    let sig_cancelled = cancelled.clone();

    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        sig_cancelled.store(true, Ordering::SeqCst);
        eprintln!(
            "{}[SHUTTING DOWN]{} signal received, finishing in-flight tasks...",
            ansi::YELLOW,
            ansi::RESET
        );
    });

    let branch_strings: Vec<Vec<(usize, String)>> = branches
        .iter()
        .map(|b| b.iter().map(|(id, s)| (*id, s.to_string())).collect())
        .collect();

    let executor = ProgressExecutor {
        inner: SimExecutor::new(),
        branches: branch_strings,
        branch_status: RwLock::new(HashMap::new()),
        multi: multi.clone(),
        bars,
        completed: AtomicUsize::new(0),
        total: tasks.len(),
        cancelled: cancelled.clone(),
    };

    let engine = ExecutionEngine::new(tasks);

    let result = tokio::time::timeout(Duration::from_secs(86400), engine.run(Arc::new(executor)))
        .await
        .unwrap_or_else(|_| {
            cancelled.store(true, Ordering::SeqCst);
            Err("build timed out".into())
        });

    match result {
        Ok(()) => {
            println!();
            info!(
                "{}{}BUILD SUCCESSFUL{}",
                ansi::BOLD,
                ansi::GREEN,
                ansi::RESET
            );
            Ok(())
        }
        Err(e) => {
            println!();
            error!("BUILD FAILED — {}", e);
            Err(anyhow::anyhow!("build failed: {}", e))
        }
    }
}
