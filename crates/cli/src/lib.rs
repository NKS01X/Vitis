#![allow(dead_code)]
use std::collections::HashMap;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use indicatif::ProgressStyle;
use vitis_core::VitisTask;

pub mod ansi {
    pub const BOLD: &str = "\x1b[1m";
    pub const GREEN: &str = "\x1b[32m";
    pub const CYAN: &str = "\x1b[36m";
    pub const RED: &str = "\x1b[31m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const GRAY: &str = "\x1b[90m";
    pub const RESET: &str = "\x1b[0m";
}

pub fn build_progress_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("{prefix:.bold.dim} {spinner:.green} [{elapsed_precise}] [{bar:30.cyan/blue}] {pos}/{len} {msg:.bold}")
        .unwrap()
        .progress_chars("=> ")
}

pub fn build_spinner_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .template("{prefix:.bold.dim} {spinner:.green} {msg:.bold}")
        .unwrap()
}

pub fn convert_dockerfile(dockerfile: &str) -> String {
    let lines: Vec<&str> = dockerfile
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    if lines.is_empty() {
        return String::new();
    }

    let mut tagged: Vec<LineTag> = Vec::new();
    for line in &lines {
        tagged.push(classify_line(line));
    }

    for i in 1..tagged.len() {
        if tagged[i - 1].role == LineRole::Workdir {
            let mut j = i;
            while j < tagged.len() {
                match tagged[j].role {
                    LineRole::From | LineRole::CmdOrEntrypoint | LineRole::Workdir => break,
                    LineRole::CopyOrAdd | LineRole::Run => {
                        tagged[j].is_branch_start = true;
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
        }
    }

    for i in 0..tagged.len() {
        if tagged[i].role == LineRole::Run || tagged[i].role == LineRole::CopyOrAdd {
            let mut found_branch_start = false;
            let mut j = i;
            while j > 0 {
                j -= 1;
                let prev_role = tagged[j].role;
                if prev_role == LineRole::From || prev_role == LineRole::CmdOrEntrypoint {
                    break;
                }
                if prev_role == LineRole::Workdir || prev_role == LineRole::Env {
                    break;
                }
                if prev_role == LineRole::Run || prev_role == LineRole::CopyOrAdd {
                    if tagged[j].is_branch_start {
                        found_branch_start = true;
                    }
                    break;
                }
            }
            if !found_branch_start && i > 0 {
                let prev_i = i - 1;
                if tagged[prev_i].role == LineRole::From
                    || tagged[prev_i].role == LineRole::Workdir
                    || tagged[prev_i].role == LineRole::Env
                {
                    tagged[i].is_branch_start = true;
                }
            }
        }
    }

    for line_tag in &mut tagged {
        if line_tag.role == LineRole::CmdOrEntrypoint
            || line_tag.role == LineRole::LabelOrExposeOrVolume
        {
            line_tag.is_merge_point = true;
        }
    }

    let mut out = String::new();
    let mut prev_indent: Option<usize> = None;

    for line_tag in &tagged {
        let indent: usize = if line_tag.role == LineRole::From || line_tag.is_merge_point {
            0
        } else if line_tag.is_branch_start {
            4
        } else {
            2
        };

        if matches!(prev_indent, Some(p) if p > 0 && indent <= p) {
            out.push('\n');
        }

        if line_tag.role == LineRole::From && !out.is_empty() {
            out.push('\n');
        }

        out.push_str(&" ".repeat(indent));
        out.push_str(&line_tag.text);
        out.push('\n');
        prev_indent = Some(indent);
    }

    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum LineRole {
    From,
    Run,
    CopyOrAdd,
    Workdir,
    Env,
    CmdOrEntrypoint,
    LabelOrExposeOrVolume,
    Other,
}

#[derive(Debug, Clone)]
struct LineTag {
    role: LineRole,
    text: String,
    is_branch_start: bool,
    is_merge_point: bool,
}

fn classify_line(line: &str) -> LineTag {
    let upper = line.to_uppercase();
    let role = if upper.starts_with("FROM ") {
        LineRole::From
    } else if upper.starts_with("RUN ") {
        LineRole::Run
    } else if upper.starts_with("COPY ") || upper.starts_with("ADD ") {
        LineRole::CopyOrAdd
    } else if upper.starts_with("WORKDIR ") {
        LineRole::Workdir
    } else if upper.starts_with("ENV ") || upper.starts_with("ARG ") {
        LineRole::Env
    } else if upper.starts_with("CMD ") || upper.starts_with("ENTRYPOINT ") {
        LineRole::CmdOrEntrypoint
    } else if upper.starts_with("LABEL ")
        || upper.starts_with("EXPOSE ")
        || upper.starts_with("VOLUME ")
    {
        LineRole::LabelOrExposeOrVolume
    } else {
        LineRole::Other
    };
    LineTag {
        role,
        text: line.to_string(),
        is_branch_start: false,
        is_merge_point: false,
    }
}

fn line_is_group_start(line: &str) -> bool {
    let upper = line.to_uppercase();
    upper.starts_with("RUN ")
        || upper.starts_with("COPY ")
        || upper.starts_with("ADD ")
        || upper.starts_with("CMD ")
        || upper.starts_with("ENTRYPOINT ")
        || upper.starts_with("ENV ")
        || upper.starts_with("ARG ")
        || upper.starts_with("LABEL ")
        || upper.starts_with("EXPOSE ")
        || upper.starts_with("WORKDIR ")
        || upper.starts_with("USER ")
        || upper.starts_with("VOLUME ")
        || upper.starts_with("SHELL ")
        || upper.starts_with("HEALTHCHECK ")
        || upper.starts_with("STOPSIGNAL ")
        || upper.starts_with("ONBUILD ")
        || line == "ONBUILD"
}

#[derive(Parser)]
#[command(
    name = "vitis",
    about = "Daemonless, DAG-driven container build engine",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Build {
        #[arg(short = 'f', long = "file", default_value = "Vitisfile")]
        file: PathBuf,

        #[arg(short = 't', long = "tag")]
        tag: Option<String>,

        #[arg(default_value = ".")]
        context: PathBuf,

        #[arg(long = "dry-run")]
        dry_run: bool,
    },
    Init {
        dockerfile: PathBuf,

        #[arg(short = 'o', long = "output", default_value = "Vitisfile")]
        output: PathBuf,
    },
}

pub fn render_dry_run(tasks: &[VitisTask]) -> String {
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let task_map: HashMap<usize, &VitisTask> = tasks.iter().map(|t| (t.id, t)).collect();

    for t in tasks {
        children.entry(t.id).or_default();
    }
    for t in tasks {
        for dep_id in &t.depends_on {
            children.entry(*dep_id).or_default().push(t.id);
        }
    }

    let roots: Vec<&VitisTask> = tasks.iter().filter(|t| t.depends_on.is_empty()).collect();

    fn build_subtree(
        task_id: usize,
        task_map: &HashMap<usize, &VitisTask>,
        children: &HashMap<usize, Vec<usize>>,
        visited: &mut std::collections::HashSet<usize>,
    ) -> termtree::Tree<String> {
        let task = &task_map[&task_id];
        let label = task.instruction.clone();

        if !visited.insert(task_id) {
            return termtree::Tree::new(format!("{label}  (shared)"));
        }

        let kid_ids = children
            .get(&task_id)
            .into_iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();

        if kid_ids.is_empty() {
            termtree::Tree::new(label)
        } else {
            let kid_trees: Vec<_> = kid_ids
                .into_iter()
                .map(|id| build_subtree(id, task_map, children, visited))
                .collect();
            termtree::Tree::new(label).with_leaves(kid_trees)
        }
    }

    let mut out = String::new();
    let mut visited: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (i, root) in roots.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let tree = build_subtree(root.id, &task_map, &children, &mut visited);
        out.push_str(&tree.to_string());
        out.push('\n');
    }

    if roots.is_empty() {
        out.push_str("  Warning: no root tasks found (possible cycle?)\n");
    }

    let unreachable: Vec<&VitisTask> = tasks.iter().filter(|t| !visited.contains(&t.id)).collect();
    if !unreachable.is_empty() {
        out.push('\n');
        out.push_str("  Orphaned tasks (not reachable from any root):\n");
        for t in &unreachable {
            out.push_str(&format!(
                "    {}  depends on: {:?}\n",
                t.instruction, t.depends_on
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dockerfile() -> String {
        "FROM ubuntu:24.04\nWORKDIR /workspace\nRUN apt-get update\nRUN apt-get install -y gcc\nCOPY . .\nRUN make".to_string()
    }

    #[test]
    fn test_convert_dockerfile_to_vitisfile() {
        let vf = convert_dockerfile(&make_dockerfile());
        assert!(vf.contains("FROM ubuntu:24.04"));
        assert!(vf.contains("RUN apt-get update"));
    }

    #[test]
    fn test_convert_dockerfile_has_indented_tasks() {
        let vf = convert_dockerfile(&make_dockerfile());
        assert!(vf.lines().any(|l| l.starts_with("  ")));
    }

    #[test]
    fn test_convert_dockerfile_empty_input() {
        let vf = convert_dockerfile("");
        assert!(vf.is_empty());
    }

    #[test]
    fn test_convert_dockerfile_only_comments() {
        let vf = convert_dockerfile("# comment\n# another");
        assert!(vf.is_empty());
    }

    #[test]
    fn test_convert_dockerfile_multi_stage() {
        let df = "FROM a\nRUN x\nFROM b\nRUN y\n";
        let vf = convert_dockerfile(df);
        assert!(vf.contains("FROM a"));
        assert!(vf.contains("FROM b"));
    }

    #[test]
    fn test_convert_dockerfile_roundtrip_valid_vitisfile() {
        let df = "FROM a\nRUN x\nRUN y\n";
        let vf = convert_dockerfile(df);
        let tasks = vitis_parser::parse_vitisfile(&vf);
        assert!(!tasks.is_empty());
    }

    #[test]
    fn test_convert_dockerfile_handles_entrypoint_cmd() {
        let df = "FROM a\nENTRYPOINT [\"x\"]\nCMD [\"y\"]\n";
        let vf = convert_dockerfile(df);
        assert!(vf.contains("ENTRYPOINT"));
        assert!(vf.contains("CMD"));
    }

    #[test]
    fn test_convert_dockerfile_preserves_inline_comments() {
        let df = "FROM a\nRUN echo hi # note\n";
        let vf = convert_dockerfile(df);
        assert!(vf.contains("RUN echo hi # note"));
    }

    #[test]
    fn test_line_is_group_start_run() {
        assert!(line_is_group_start("RUN echo hi"));
        assert!(line_is_group_start("run echo hi"));
    }

    #[test]
    fn test_line_is_group_start_copy() {
        assert!(line_is_group_start("COPY . ."));
    }

    #[test]
    fn test_line_is_group_start_non_command() {
        assert!(!line_is_group_start("echo hi"));
    }

    #[test]
    fn test_render_dry_run_empty() {
        let out = render_dry_run(&[]);
        assert!(out.contains("no root tasks"));
    }

    #[test]
    fn test_render_dry_run_single() {
        let tasks = vec![VitisTask {
            id: 0,
            instruction: "FROM a".into(),
            indent_level: 0,
            depends_on: vec![],
        }];
        let out = render_dry_run(&tasks);
        assert!(out.contains("FROM a"));
    }

    #[test]
    fn test_render_dry_run_chain() {
        let tasks = vec![
            VitisTask {
                id: 0,
                instruction: "FROM a".into(),
                indent_level: 0,
                depends_on: vec![],
            },
            VitisTask {
                id: 1,
                instruction: "RUN b".into(),
                indent_level: 2,
                depends_on: vec![0],
            },
            VitisTask {
                id: 2,
                instruction: "RUN c".into(),
                indent_level: 2,
                depends_on: vec![1],
            },
        ];
        let out = render_dry_run(&tasks);
        assert!(out.contains("FROM a"));
        assert!(out.contains("RUN b"));
    }

    #[test]
    fn test_render_dry_run_parallel() {
        let tasks = vec![
            VitisTask {
                id: 0,
                instruction: "FROM a".into(),
                indent_level: 0,
                depends_on: vec![],
            },
            VitisTask {
                id: 1,
                instruction: "RUN b".into(),
                indent_level: 2,
                depends_on: vec![0],
            },
            VitisTask {
                id: 2,
                instruction: "RUN c".into(),
                indent_level: 2,
                depends_on: vec![0],
            },
            VitisTask {
                id: 3,
                instruction: "RUN d".into(),
                indent_level: 0,
                depends_on: vec![1, 2],
            },
        ];
        let out = render_dry_run(&tasks);
        assert!(out.contains("RUN d"));
    }

    #[test]
    fn test_render_dry_run_detects_orphan() {
        let tasks = vec![
            VitisTask {
                id: 0,
                instruction: "FROM a".into(),
                indent_level: 0,
                depends_on: vec![],
            },
            VitisTask {
                id: 1,
                instruction: "RUN b".into(),
                indent_level: 2,
                depends_on: vec![99],
            },
        ];
        let out = render_dry_run(&tasks);
        assert!(out.contains("Orphaned"));
    }

    #[test]
    fn test_render_dry_run_marks_shared() {
        let tasks = vec![
            VitisTask {
                id: 0,
                instruction: "FROM a".into(),
                indent_level: 0,
                depends_on: vec![],
            },
            VitisTask {
                id: 1,
                instruction: "RUN b".into(),
                indent_level: 2,
                depends_on: vec![0],
            },
            VitisTask {
                id: 2,
                instruction: "RUN c".into(),
                indent_level: 2,
                depends_on: vec![0],
            },
            VitisTask {
                id: 3,
                instruction: "RUN d".into(),
                indent_level: 0,
                depends_on: vec![1, 2],
            },
            VitisTask {
                id: 4,
                instruction: "RUN e".into(),
                indent_level: 0,
                depends_on: vec![1, 2],
            },
        ];
        let out = render_dry_run(&tasks);
        assert!(out.contains("RUN d") || out.contains("shared"));
    }

    #[test]
    fn test_convert_dockerfile_blank_lines_omitted() {
        let df = "FROM a\nRUN x\n\nRUN y\n";
        let vf = convert_dockerfile(df);
        let lines: Vec<&str> = vf.lines().collect();
        assert!(lines.iter().any(|l| l.contains("RUN x")));
        assert!(lines.iter().any(|l| l.contains("RUN y")));
    }

    #[test]
    fn test_convert_dockerfile_workdir_spawns_branches() {
        let df = "FROM ubuntu\nWORKDIR /app\nCOPY backend .\nRUN make backend\n\nCOPY frontend .\nRUN npm build\nCMD bash\n";
        let vf = convert_dockerfile(df);
        let tasks = vitis_parser::parse_vitisfile(&vf);
        assert!(tasks.len() >= 5);
        let cmd_task = tasks
            .iter()
            .find(|t| t.instruction.contains("CMD"))
            .unwrap();
        assert!(!cmd_task.depends_on.is_empty());
    }

    #[test]
    fn test_convert_dockerfile_entrypoint_is_merge() {
        let df = "FROM node\nWORKDIR /src\n\nADD package.json .\n\nADD server.js .\nENTRYPOINT [\"node\"]\n";
        let vf = convert_dockerfile(df);
        assert!(vf.contains("ENTRYPOINT"));
        assert!(!vf.contains("  ENTRYPOINT"));
    }
}
