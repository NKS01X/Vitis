use vitis_core::VitisTask;

pub fn parse_vitisfile(content: &str) -> Vec<VitisTask> {
    let mut tasks: Vec<VitisTask> = Vec::new();
    let mut scope_stack: Vec<(usize, usize)> = Vec::new();
    let mut last_in_segment: Option<usize> = None;
    let mut branch_leaves: Vec<usize> = Vec::new();
    let mut current_id: usize = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            if let Some(leaf) = last_in_segment {
                let leaf_indent = tasks[leaf].indent_level;

                if leaf_indent > 0 && !branch_leaves.contains(&leaf) {
                    branch_leaves.push(leaf);
                }
            }
            last_in_segment = None;
            continue;
        }

        if trimmed.starts_with('#') {
            continue;
        }

        let indent_level = line.len() - line.trim_start().len();
        let is_root = trimmed.starts_with("FROM");

        let mut merging = false;
        while let Some(&(stack_indent, _)) = scope_stack.last() {
            if stack_indent > indent_level {
                scope_stack.pop();
                merging = true;
            } else {
                break;
            }
        }

        if !merging && last_in_segment.is_none() {
            while let Some(&(stack_indent, _)) = scope_stack.last() {
                if stack_indent >= indent_level {
                    scope_stack.pop();
                } else {
                    break;
                }
            }
        }

        let mut depends_on = Vec::new();

        if is_root {
        } else if merging {
            depends_on.extend(branch_leaves.iter().cloned());
            if let Some(leaf) = last_in_segment {
                if !depends_on.contains(&leaf) {
                    depends_on.push(leaf);
                }
            }
            branch_leaves.clear();
        } else if let Some(prev_id) = last_in_segment {
            depends_on.push(prev_id);
        } else if let Some(&(_, parent_id)) = scope_stack.last() {
            depends_on.push(parent_id);
        }

        tasks.push(VitisTask {
            id: current_id,
            instruction: trimmed.to_string(),
            indent_level,
            depends_on,
        });

        scope_stack.push((indent_level, current_id));
        last_in_segment = Some(current_id);
        current_id += 1;
    }

    tasks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(content: &str, id: usize) -> Vec<usize> {
        parse_vitisfile(content)
            .into_iter()
            .find(|t| t.id == id)
            .map(|t| t.depends_on)
            .unwrap_or_default()
    }

    #[test]
    fn sequential_chain() {
        let v = parse_vitisfile("FROM a\nRUN b\nRUN c\n");
        assert_eq!(v[0].depends_on.len(), 0);
        assert_eq!(v[1].depends_on, vec![0]);
        assert_eq!(v[2].depends_on, vec![1]);
    }

    #[test]
    fn parallel_branches_merge() {
        let src = "FROM a\n\n    RUN b\n\n    RUN c\n\nRUN d\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[3].depends_on, vec![1, 2]);
    }

    #[test]
    fn sequential_chain_within_branch() {
        let src = "FROM a\n\n    RUN b\n    RUN c\n\nRUN d\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[1].depends_on, vec![0]);
        assert_eq!(v[2].depends_on, vec![1]);
        assert_eq!(v[3].depends_on, vec![2]);
    }

    #[test]
    fn comments_and_blanks_ignored() {
        let src = "# header\nFROM a\n\n  # indented comment\n  RUN b\n";
        let v = parse_vitisfile(src);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].instruction, "FROM a");
        assert_eq!(v[1].instruction, "RUN b");
    }

    #[test]
    fn two_roots_no_deps() {
        let v = parse_vitisfile("FROM a\nFROM b\n");
        assert_eq!(v[0].depends_on.len(), 0);
        assert_eq!(v[1].depends_on.len(), 0);
    }

    #[test]
    fn nested_dependency() {
        let src = "FROM a\n  RUN b\n    RUN c\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[1].depends_on, vec![0]);
        assert_eq!(v[2].depends_on, vec![1]);
    }

    #[test]
    fn diamond_shape() {
        let src = "FROM a\n  RUN b\n  RUN c\n    RUN d\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[1].depends_on, vec![0]);
        assert_eq!(v[2].depends_on, vec![1]);
        assert_eq!(v[3].depends_on, vec![2]);
    }

    #[test]
    fn empty_input() {
        assert!(parse_vitisfile("").is_empty());
        assert!(parse_vitisfile("# only comments\n\n").is_empty());
    }

    #[test]
    fn preserved_inline_comment() {
        let v = parse_vitisfile("RUN echo hi # note\n");
        assert_eq!(v[0].instruction, "RUN echo hi # note");
    }

    #[test]
    fn branch_leaf_buffering() {
        let src = "FROM a\n\n  RUN b\n  RUN c\n  RUN e\n\nRUN f\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[4].depends_on, vec![3]);
    }

    #[test]
    fn multiple_branch_leaves_on_merge() {
        let src = "FROM a\n\n  RUN b\n\n  RUN c\n\n  RUN e\n\nRUN f\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[4].depends_on, vec![1, 2, 3]);
    }

    #[test]
    fn deep_nesting_chain() {
        let src = "A\n  B\n    C\n      D\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[3].depends_on, vec![2]);
    }

    #[test]
    fn multiple_independent_branches() {
        let src = "FROM a\n\n  RUN b\n\n  RUN c\n\n  RUN d\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[1].depends_on, vec![0]);
        assert_eq!(v[2].depends_on, vec![0]);
        assert_eq!(v[3].depends_on, vec![0]);
    }

    #[test]
    fn ids_are_sequential() {
        let v = parse_vitisfile("A\nB\nC\nD\n");
        let ids: Vec<usize> = v.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn indent_level_recorded() {
        let v = parse_vitisfile("A\n  B\n    C\n");
        assert_eq!(v[0].indent_level, 0);
        assert_eq!(v[1].indent_level, 2);
        assert_eq!(v[2].indent_level, 4);
    }

    #[test]
    fn tabs_and_spaces_mixed() {
        let src = "FROM a\n\tRUN b\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[1].indent_level, 1);
        assert_eq!(v[1].depends_on, vec![0]);
    }

    #[test]
    fn merge_after_two_levels() {
        let src = "FROM a\n  RUN b\n    RUN c\n  RUN d\n";
        let v = parse_vitisfile(src);
        assert_eq!(v[3].depends_on, vec![2]);
    }

    #[test]
    fn no_cyclic_self_dep() {
        let v = parse_vitisfile("RUN a\nRUN b\n");
        assert_ne!(v[1].depends_on, vec![1]);
    }

    #[test]
    fn whitespace_only_lines_ignored() {
        let src = "FROM a\n   \nRUN b\n";
        let v = parse_vitisfile(src);
        assert_eq!(v.len(), 2);
    }
}
