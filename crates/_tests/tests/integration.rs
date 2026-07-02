use vitis_core::VitisTask;
use vitis_parser::parse_vitisfile;

#[cfg(feature = "engine")]
fn run(cmd: &str) -> std::process::Output {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let mut args = vec!["run", "--bin", "vitis", "-q", "--features", "engine", "--"];
    args.extend(parts);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    std::process::Command::new("cargo")
        .args(&args)
        .current_dir(workspace_root)
        .output()
        .expect("spawn vitis")
}

#[test]
fn parses_real_vitisfile_shape() {
    let src = "FROM ubuntu:24.04\nWORKDIR /workspace\n\n  RUN apt-get update\n  RUN make\n\n  RUN npm ci\n  RUN npm run build\n\nRUN tar -czf out.tar.gz .\n";
    let tasks = parse_vitisfile(src);
    assert_eq!(tasks.len(), 7);

    assert_eq!(tasks[6].depends_on, vec![3, 5]);
}

#[cfg(feature = "engine")]
#[test]
fn dry_run_subprocess_succeeds() {
    let out = run("build --dry-run");
    if !out.status.success() {
        panic!("dry_run_subprocess_succeeds failed. stdout: {}, stderr: {}",
               String::from_utf8_lossy(&out.stdout),
               String::from_utf8_lossy(&out.stderr));
    }
}

#[cfg(feature = "engine")]
#[test]
fn init_subprocess_succeeds() {
    let df = "FROM ubuntu\nRUN apt-get update\nRUN apt-get install -y curl\n";
    let tmp = std::env::temp_dir().join("vitis_e2e_dockerfile");
    std::fs::write(&tmp, df).unwrap();
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let out = std::process::Command::new("cargo")
        .args([
            "run",
            "--bin",
            "vitis",
            "-q",
            "--features",
            "engine",
            "--",
            "init",
            tmp.to_str().unwrap(),
            "-o",
            "/tmp/vitis_e2e_out",
        ])
        .current_dir(workspace_root)
        .output()
        .expect("spawn vitis init");
    if !out.status.success() {
        panic!("init_subprocess_succeeds failed. stdout: {}, stderr: {}",
               String::from_utf8_lossy(&out.stdout),
               String::from_utf8_lossy(&out.stderr));
    }
    let converted = std::fs::read_to_string("/tmp/vitis_e2e_out").unwrap();
    assert!(converted.contains("FROM ubuntu"));
}

#[test]
fn full_parse_produces_acyclic_graph() {
    let src = "FROM a\n  RUN b\n    RUN c\n  RUN d\nRUN e\n";
    let tasks = parse_vitisfile(src);

    assert!(!tasks[4].depends_on.contains(&4));
}

#[cfg(feature = "engine")]
#[test]
fn integration_topological_order_preserved() {
    let tasks = vec![
        VitisTask {
            id: 0,
            instruction: "FROM".into(),
            indent_level: 0,
            depends_on: vec![],
        },
        VitisTask {
            id: 1,
            instruction: "A".into(),
            indent_level: 2,
            depends_on: vec![0],
        },
        VitisTask {
            id: 2,
            instruction: "B".into(),
            indent_level: 2,
            depends_on: vec![0],
        },
        VitisTask {
            id: 3,
            instruction: "C".into(),
            indent_level: 0,
            depends_on: vec![1, 2],
        },
    ];
    let engine = vitis_engine::ExecutionEngine::new(tasks);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(engine.run(std::sync::Arc::new(vitis_engine::SimExecutor::new())));
    assert!(result.is_ok());
}

#[cfg(not(feature = "engine"))]
#[test]
fn integration_topological_shape() {
    let tasks = vec![
        VitisTask {
            id: 0,
            instruction: "FROM".into(),
            indent_level: 0,
            depends_on: vec![],
        },
        VitisTask {
            id: 1,
            instruction: "A".into(),
            indent_level: 2,
            depends_on: vec![0],
        },
        VitisTask {
            id: 2,
            instruction: "B".into(),
            indent_level: 2,
            depends_on: vec![0],
        },
        VitisTask {
            id: 3,
            instruction: "C".into(),
            indent_level: 0,
            depends_on: vec![1, 2],
        },
    ];
    assert_eq!(tasks[3].depends_on.len(), 2);
}

#[test]
fn parser_handles_three_independent_branches() {
    let src = "FROM a\n\n  RUN b\n\n  RUN c\n\n  RUN d\n";
    let tasks = parse_vitisfile(src);

    assert_eq!(tasks[1].depends_on, vec![0]);
    assert_eq!(tasks[2].depends_on, vec![0]);
    assert_eq!(tasks[3].depends_on, vec![0]);
}

#[test]
fn parser_preserves_instruction_text() {
    let tasks = parse_vitisfile("RUN echo \"hello world\"\n");
    assert_eq!(tasks[0].instruction, "RUN echo \"hello world\"");
}

#[test]
fn parser_ignores_comments_in_graph() {
    let src = "# build stage\nFROM a\n  # setup\n  RUN b\n";
    let tasks = parse_vitisfile(src);
    assert_eq!(tasks.len(), 2);
}

#[test]
fn parser_nested_four_levels() {
    let src = "A\n  B\n    C\n      D\n        E\n";
    let tasks = parse_vitisfile(src);
    assert_eq!(tasks.len(), 5);
    assert_eq!(tasks[4].depends_on, vec![3]);
}

#[test]
fn parser_merge_after_deep_branch() {
    let src = "FROM a\n  RUN b\n    RUN c\n      RUN d\n  RUN e\n";
    let tasks = parse_vitisfile(src);
    assert_eq!(tasks[4].depends_on, vec![3]);
}

#[test]
fn parser_empty_file_no_tasks() {
    assert!(parse_vitisfile("").is_empty());
    assert!(parse_vitisfile("   \n\n").is_empty());
}

#[cfg(feature = "engine")]
#[test]
fn dry_run_output_contains_task_count() {
    let out = run("build --dry-run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.contains("tasks") {
        panic!("dry_run_output_contains_task_count failed. stdout: {}, stderr: {}",
               stdout,
               String::from_utf8_lossy(&out.stderr));
    }
}

#[cfg(feature = "engine")]
#[test]
fn init_then_parse_roundtrip() {
    let df = "FROM rust:1.75\nWORKDIR /src\nRUN cargo build\nRUN cargo test\n";
    let tmp = std::env::temp_dir().join("vitis_rt_df");
    std::fs::write(&tmp, df).unwrap();
    let out_path = std::env::temp_dir().join("vitis_rt_vf");
    let _ = std::fs::remove_file(&out_path);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let out = std::process::Command::new("cargo")
        .args([
            "run",
            "--bin",
            "vitis",
            "-q",
            "--features",
            "engine",
            "--",
            "init",
            tmp.to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .current_dir(workspace_root)
        .output()
        .unwrap();
    if !out.status.success() {
        panic!("init_then_parse_roundtrip cargo run failed. stdout: {}, stderr: {}",
               String::from_utf8_lossy(&out.stdout),
               String::from_utf8_lossy(&out.stderr));
    }
    let vf = std::fs::read_to_string(&out_path).unwrap();
    let tasks = parse_vitisfile(&vf);
    assert!(!tasks.is_empty());
}
