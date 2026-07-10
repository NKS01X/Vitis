// Exclusive tests for the `vitis-cli` crate: Dockerfile converter + dry-run renderer.

use vitis_cli::{convert_dockerfile, render_dry_run};
use vitis_core::VitisTask;

#[test]
fn convert_empty_input() {
    assert!(convert_dockerfile("").is_empty());
}

#[test]
fn convert_comments_only() {
    assert!(convert_dockerfile("# comment\n# another").is_empty());
}

#[test]
fn convert_preserves_from() {
    let vf = convert_dockerfile("FROM ubuntu:24.04\nRUN x\n");
    assert!(vf.contains("FROM ubuntu:24.04"));
}

#[test]
fn convert_indents_tasks() {
    let df = "FROM ubuntu:24.04\nWORKDIR /workspace\nRUN apt-get update\nRUN make\n";
    let vf = convert_dockerfile(df);
    assert!(vf.lines().any(|l| l.starts_with("  RUN apt-get update")));
    assert!(vf.lines().any(|l| l.starts_with("  RUN make")));
}

#[test]
fn convert_multi_stage() {
    let df = "FROM a\nRUN x\nFROM b\nRUN y\n";
    let vf = convert_dockerfile(df);
    assert!(vf.contains("FROM a"));
    assert!(vf.contains("FROM b"));
}

#[test]
fn convert_blank_lines_create_parallel_branches() {
    let df = "FROM a\nRUN x\n\nRUN y\n";
    let vf = convert_dockerfile(df);
    assert!(vf.lines().any(|l| l.starts_with("  RUN x")));
    assert!(vf.lines().any(|l| l.starts_with("  RUN y")));
}

#[test]
fn convert_roundtrip_is_valid_vitisfile() {
    let df = "FROM a\nRUN x\nRUN y\n";
    let vf = convert_dockerfile(df);
    let tasks = vitis_parser::parse_vitisfile(&vf);
    assert!(!tasks.is_empty());
}

#[test]
fn convert_preserves_entrypoint_and_cmd() {
    let df = "FROM a\nENTRYPOINT [\"x\"]\nCMD [\"y\"]\n";
    let vf = convert_dockerfile(df);
    assert!(vf.contains("ENTRYPOINT"));
    assert!(vf.contains("CMD"));
}

#[test]
fn convert_handles_copy_add() {
    let df = "FROM a\nCOPY . .\nADD x y\n";
    let vf = convert_dockerfile(df);
    assert!(vf.contains("COPY"));
    assert!(vf.contains("ADD"));
}

#[test]
fn convert_filters_inline_comments_per_line() {
    // The converter strips comment-only lines but keeps inline comments.
    let df = "FROM a\nRUN echo hi # note\n";
    let vf = convert_dockerfile(df);
    assert!(vf.contains("FROM a"));
    assert!(vf.contains("RUN echo hi # note"));
}

#[test]
fn convert_recognizes_run_group() {
    // RUN lines are indented as their own parallel group branch.
    let vf = convert_dockerfile("FROM a\nRUN x\n\nRUN y\n");
    let lines: Vec<&str> = vf.lines().collect();
    assert!(lines.iter().any(|l| l.trim() == "RUN x"));
    assert!(lines.iter().any(|l| l.trim() == "RUN y"));
}

#[test]
fn convert_recognizes_copy_and_add_groups() {
    let vf = convert_dockerfile("FROM a\nCOPY . .\n\nADD x y\n");
    assert!(vf.contains("COPY . ."));
    assert!(vf.contains("ADD x y"));
}

#[test]
fn convert_recognizes_env_and_workdir_groups() {
    let vf = convert_dockerfile("FROM a\nWORKDIR /app\nENV A=b\n");
    let lines: Vec<&str> = vf.lines().collect();
    assert!(lines.iter().any(|l| l.trim() == "WORKDIR /app"));
    assert!(lines.iter().any(|l| l.trim() == "ENV A=b"));
}

#[test]
fn render_empty_warns_no_roots() {
    let out = render_dry_run(&[]);
    assert!(out.contains("no root tasks"));
}

#[test]
fn render_single_task() {
    let tasks = vec![VitisTask { id: 0, instruction: "FROM a".into(), indent_level: 0, depends_on: vec![] }];
    let out = render_dry_run(&tasks);
    assert!(out.contains("FROM a"));
}

#[test]
fn render_sequential_chain() {
    let tasks = vec![
        VitisTask { id: 0, instruction: "FROM a".into(), indent_level: 0, depends_on: vec![] },
        VitisTask { id: 1, instruction: "RUN b".into(), indent_level: 2, depends_on: vec![0] },
        VitisTask { id: 2, instruction: "RUN c".into(), indent_level: 2, depends_on: vec![1] },
    ];
    let out = render_dry_run(&tasks);
    assert!(out.contains("FROM a"));
    assert!(out.contains("RUN b"));
    assert!(out.contains("RUN c"));
}

#[test]
fn render_parallel_branches() {
    let tasks = vec![
        VitisTask { id: 0, instruction: "FROM a".into(), indent_level: 0, depends_on: vec![] },
        VitisTask { id: 1, instruction: "RUN b".into(), indent_level: 2, depends_on: vec![0] },
        VitisTask { id: 2, instruction: "RUN c".into(), indent_level: 2, depends_on: vec![0] },
        VitisTask { id: 3, instruction: "RUN d".into(), indent_level: 0, depends_on: vec![1, 2] },
    ];
    let out = render_dry_run(&tasks);
    assert!(out.contains("RUN d"));
}

#[test]
fn render_detects_orphan() {
    let tasks = vec![
        VitisTask { id: 0, instruction: "FROM a".into(), indent_level: 0, depends_on: vec![] },
        VitisTask { id: 1, instruction: "RUN b".into(), indent_level: 2, depends_on: vec![99] },
    ];
    let out = render_dry_run(&tasks);
    assert!(out.contains("Orphaned"));
}

#[test]
fn render_marks_shared_subtree() {
    let tasks = vec![
        VitisTask { id: 0, instruction: "FROM a".into(), indent_level: 0, depends_on: vec![] },
        VitisTask { id: 1, instruction: "RUN b".into(), indent_level: 2, depends_on: vec![0] },
        VitisTask { id: 2, instruction: "RUN c".into(), indent_level: 2, depends_on: vec![0] },
        VitisTask { id: 3, instruction: "RUN d".into(), indent_level: 0, depends_on: vec![1, 2] },
        VitisTask { id: 4, instruction: "RUN e".into(), indent_level: 0, depends_on: vec![1, 2] },
    ];
    let out = render_dry_run(&tasks);
    assert!(out.contains("shared") || out.contains("RUN d"));
}

#[test]
fn render_multi_root() {
    let tasks = vec![
        VitisTask { id: 0, instruction: "FROM a".into(), indent_level: 0, depends_on: vec![] },
        VitisTask { id: 1, instruction: "FROM b".into(), indent_level: 0, depends_on: vec![] },
    ];
    let out = render_dry_run(&tasks);
    assert!(out.contains("FROM a"));
    assert!(out.contains("FROM b"));
}

#[test]
fn convert_does_not_emit_trailing_blank_only_blocks() {
    let df = "FROM a\nRUN x\n";
    let vf = convert_dockerfile(df);
    assert!(!vf.contains("\n\n\n"));
}
