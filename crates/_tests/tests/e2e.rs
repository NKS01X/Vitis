use vitis_parser::parse_vitisfile;

#[test]
fn e2e_sample_frontend_backend() {
    let src = "FROM ubuntu:24.04\nWORKDIR /workspace\n\n  RUN apt-get update && apt-get install -y gcc\n  COPY ./backend ./backend\n  RUN make build-backend\n\n  RUN apt-get update && apt-get install -y nodejs\n  COPY ./frontend ./frontend\n  RUN npm run build\n\nRUN tar -czf release.tar.gz ./backend/dist ./frontend/dist\n";
    let tasks = parse_vitisfile(src);
    assert_eq!(tasks.len(), 9);

    assert_eq!(tasks[8].depends_on, vec![4, 7]);
}

#[test]
fn e2e_multi_language() {
    let src = "FROM ubuntu:20.04\nRUN apt-get update\nRUN apt-get install -y golang cargo\n\n  cargo build --release\n\n  go build -o app .\n\nRUN echo done\n";
    let tasks = parse_vitisfile(src);

    assert_eq!(tasks.last().unwrap().depends_on.len(), 2);
}

#[test]
fn e2e_ml_pipeline() {
    let src = "FROM python:3.11\nRUN pip install numpy\n\n  python train_a.py\n  python train_b.py\n  python train_c.py\n\nRUN evaluate.py\n";
    let tasks = parse_vitisfile(src);
    let eval = tasks.last().unwrap();

    assert_eq!(eval.depends_on.len(), 1);
}

#[test]
fn e2e_no_cycles_in_real_examples() {
    let src = "FROM a\n  RUN b\n  RUN c\nRUN d\n  RUN e\nRUN f\n";
    let tasks = parse_vitisfile(src);
    let f = tasks.last().unwrap();
    assert!(!f.depends_on.contains(&f.id));
    assert_eq!(f.depends_on.len(), 1);
}

#[test]
fn e2e_diamond_dependency() {
    let src = "FROM a\n  RUN b\n  RUN c\n    RUN d\n";
    let tasks = parse_vitisfile(src);
    assert_eq!(tasks[3].depends_on, vec![2]);
}

#[test]
fn e2e_sequential_chain_length() {
    let src = "FROM a\nRUN b\nRUN c\nRUN d\nRUN e\n";
    let tasks = parse_vitisfile(src);
    assert_eq!(tasks.len(), 5);
    assert_eq!(tasks[4].depends_on, vec![3]);
}

#[test]
fn e2e_branch_merge_single_leaf() {
    let src = "FROM a\n  RUN b\n  RUN c\n";
    let tasks = parse_vitisfile(src);
    assert_eq!(tasks[1].depends_on, vec![0]);
    assert_eq!(tasks[2].depends_on, vec![1]);
}

#[test]
fn e2e_blank_separates_independent_groups() {
    let src = "FROM a\n\n  RUN b\n\n  RUN c\n";
    let tasks = parse_vitisfile(src);
    assert_eq!(tasks[1].depends_on, vec![0]);
    assert_eq!(tasks[2].depends_on, vec![0]);
}

#[test]
fn e2e_deep_nesting_records_levels() {
    let src = "A\n  B\n    C\n      D\n        E\n";
    let tasks = parse_vitisfile(src);
    let levels: Vec<usize> = tasks.iter().map(|t| t.indent_level).collect();
    assert_eq!(levels, vec![0, 2, 4, 6, 8]);
}

#[test]
fn e2e_all_from_are_roots() {
    let src = "FROM a\nFROM b\nFROM c\n";
    let tasks = parse_vitisfile(src);
    assert!(tasks.iter().all(|t| t.depends_on.is_empty()));
}
