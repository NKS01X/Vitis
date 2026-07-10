// Exclusive tests for the `vitis-parser` crate: the scoped Vitisfile DAG parser.

use vitis_parser::parse_vitisfile;

fn deps(src: &str, id: usize) -> Vec<usize> {
    parse_vitisfile(src)
        .into_iter()
        .find(|t| t.id == id)
        .map(|t| t.depends_on)
        .unwrap_or_default()
}

#[test]
fn empty_input_yields_no_tasks() {
    assert!(parse_vitisfile("").is_empty());
    assert!(parse_vitisfile("   \n\n").is_empty());
    assert!(parse_vitisfile("# only comments\n\n").is_empty());
}

#[test]
fn single_from_is_root() {
    let v = parse_vitisfile("FROM ubuntu:24.04\n");
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].depends_on, vec![]);
    assert_eq!(v[0].indent_level, 0);
}

#[test]
fn sequential_chain_dependencies() {
    let v = parse_vitisfile("FROM a\nRUN b\nRUN c\nRUN d\n");
    assert_eq!(v[0].depends_on, vec![]);
    assert_eq!(v[1].depends_on, vec![0]);
    assert_eq!(v[2].depends_on, vec![1]);
    assert_eq!(v[3].depends_on, vec![2]);
}

#[test]
fn ids_are_zero_based_sequential() {
    let v = parse_vitisfile("FROM a\nRUN b\nRUN c\n");
    let ids: Vec<usize> = v.iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![0, 1, 2]);
}

#[test]
fn indent_level_recorded_per_line() {
    let v = parse_vitisfile("A\n  B\n    C\n      D\n");
    assert_eq!(v[0].indent_level, 0);
    assert_eq!(v[1].indent_level, 2);
    assert_eq!(v[2].indent_level, 4);
    assert_eq!(v[3].indent_level, 6);
}

#[test]
fn nested_dependency_chain() {
    let v = parse_vitisfile("FROM a\n  RUN b\n    RUN c\n      RUN d\n");
    assert_eq!(v[1].depends_on, vec![0]);
    assert_eq!(v[2].depends_on, vec![1]);
    assert_eq!(v[3].depends_on, vec![2]);
}

#[test]
fn parallel_branches_via_blank_lines() {
    // blank-separated branches at the same indent are independent siblings.
    let src = "FROM a\n\n  RUN b\n\n  RUN c\n\n  RUN d\n";
    let v = parse_vitisfile(src);
    assert_eq!(v[1].depends_on, vec![0]);
    assert_eq!(v[2].depends_on, vec![0]);
    assert_eq!(v[3].depends_on, vec![0]);
}

#[test]
fn merge_point_depends_on_all_branch_leaves() {
    // three blank-separated branches; the dedent merges all three leaves.
    let src = "FROM a\n\n  RUN b\n\n  RUN c\n\n  RUN e\n\nRUN f\n";
    let v = parse_vitisfile(src);
    assert_eq!(v[4].depends_on, vec![1, 2, 3]);
}

#[test]
fn merge_of_two_branches() {
    let src = "FROM a\n\n  RUN b\n\n  RUN c\n\nRUN d\n";
    let v = parse_vitisfile(src);
    assert_eq!(v[3].depends_on, vec![1, 2]);
}

#[test]
fn single_branch_is_sequential_not_parallel() {
    let src = "FROM a\n\n  RUN b\n  RUN c\n  RUN e\n\nRUN f\n";
    let v = parse_vitisfile(src);
    // b->c->e is one sequential branch; f depends only on the leaf e.
    assert_eq!(v[1].depends_on, vec![0]);
    assert_eq!(v[2].depends_on, vec![1]);
    assert_eq!(v[3].depends_on, vec![2]);
    assert_eq!(v[4].depends_on, vec![3]);
}

#[test]
fn multiple_from_are_independent_roots() {
    let v = parse_vitisfile("FROM a\nFROM b\nFROM c\n");
    assert_eq!(v[0].depends_on, vec![]);
    assert_eq!(v[1].depends_on, vec![]);
    assert_eq!(v[2].depends_on, vec![]);
}

#[test]
fn comments_are_ignored() {
    let src = "# header\nFROM a\n\n  # indented comment\n  RUN b\n";
    let v = parse_vitisfile(src);
    assert_eq!(v.len(), 2);
    assert_eq!(v[0].instruction, "FROM a");
    assert_eq!(v[1].instruction, "RUN b");
}

#[test]
fn inline_comment_preserved_in_instruction() {
    let v = parse_vitisfile("RUN echo hi # note\n");
    assert_eq!(v[0].instruction, "RUN echo hi # note");
}

#[test]
fn whitespace_only_lines_ignored() {
    let v = parse_vitisfile("FROM a\n   \nRUN b\n");
    assert_eq!(v.len(), 2);
}

#[test]
fn tabs_count_as_one_indent_column() {
    let v = parse_vitisfile("FROM a\n\tRUN b\n");
    assert_eq!(v[1].indent_level, 1);
    assert_eq!(v[1].depends_on, vec![0]);
}

#[test]
fn diamond_shape_dependencies() {
    let src = "FROM a\n  RUN b\n  RUN c\n    RUN d\n";
    let v = parse_vitisfile(src);
    assert_eq!(v[1].depends_on, vec![0]);
    assert_eq!(v[2].depends_on, vec![1]);
    assert_eq!(v[3].depends_on, vec![2]);
}

#[test]
fn no_self_dependency_ever() {
    let v = parse_vitisfile("RUN a\nRUN b\nRUN c\n");
    for t in &v {
        assert!(!t.depends_on.contains(&t.id));
    }
}

#[test]
fn merge_after_deep_branch() {
    let src = "FROM a\n  RUN b\n    RUN c\n      RUN d\n  RUN e\n";
    let v = parse_vitisfile(src);
    assert_eq!(v[4].depends_on, vec![3]);
}

#[test]
fn real_shaped_vitisfile() {
    let src = "FROM ubuntu:24.04\nWORKDIR /workspace\n\n  RUN apt-get update\n  RUN make\n\n  RUN npm ci\n  RUN npm run build\n\nRUN tar -czf out.tar.gz .\n";
    let v = parse_vitisfile(src);
    assert_eq!(v.len(), 7);
    assert_eq!(v[6].depends_on, vec![3, 5]);
}

#[test]
fn branch_leaf_buffering_single_branch() {
    let src = "FROM a\n\n  RUN b\n  RUN c\n  RUN e\n\nRUN f\n";
    let v = parse_vitisfile(src);
    assert_eq!(v[4].depends_on, vec![3]);
}

#[test]
fn deep_nesting_four_levels() {
    let v = parse_vitisfile("A\n  B\n    C\n      D\n        E\n");
    let levels: Vec<usize> = v.iter().map(|t| t.indent_level).collect();
    assert_eq!(levels, vec![0, 2, 4, 6, 8]);
}

#[test]
fn three_independent_blank_branches() {
    let src = "FROM a\n\n  RUN b\n\n  RUN c\n\n  RUN d\n";
    let v = parse_vitisfile(src);
    assert_eq!(v[1].depends_on, vec![0]);
    assert_eq!(v[2].depends_on, vec![0]);
    assert_eq!(v[3].depends_on, vec![0]);
}

#[test]
fn nested_dependency_preserved() {
    let v = parse_vitisfile("FROM a\n  RUN b\n    RUN c\n");
    assert_eq!(v[1].depends_on, vec![0]);
    assert_eq!(v[2].depends_on, vec![1]);
}

#[test]
fn instruction_text_preserved() {
    let v = parse_vitisfile("RUN echo \"hello world\"\n");
    assert_eq!(v[0].instruction, "RUN echo \"hello world\"");
}

#[test]
fn blank_separates_independent_groups() {
    let v = parse_vitisfile("FROM a\n\n  RUN b\n\n  RUN c\n");
    assert_eq!(v[1].depends_on, vec![0]);
    assert_eq!(v[2].depends_on, vec![0]);
}

#[test]
fn complex_multi_level_merge() {
    let src = "FROM a\n  RUN b\n    RUN c\n  RUN d\nRUN e\n";
    let v = parse_vitisfile(src);
    // e merges only the leaf of the last branch (d), which is sequential from b.
    assert_eq!(v[4].depends_on, vec![3]);
}

#[test]
fn all_from_are_roots_in_parallel_stages() {
    let v = parse_vitisfile("FROM a\nFROM b\nFROM c\n");
    assert!(v.iter().all(|t| t.depends_on.is_empty()));
}

#[test]
fn every_task_has_valid_id() {
    let v = parse_vitisfile("FROM a\n  RUN b\n  RUN c\nRUN d\n");
    for (i, t) in v.iter().enumerate() {
        assert_eq!(t.id, i);
    }
}

#[test]
fn dependencies_reference_existing_ids() {
    let v = parse_vitisfile("FROM a\n  RUN b\n  RUN c\nRUN d\n  RUN e\nRUN f\n");
    let ids: std::collections::HashSet<usize> = v.iter().map(|t| t.id).collect();
    for t in &v {
        for &d in &t.depends_on {
            assert!(ids.contains(&d), "task {} depends on missing id {}", t.id, d);
        }
    }
}
