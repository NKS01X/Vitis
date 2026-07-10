// Exclusive tests for the `vitis-core` crate: the VitisTask DAG node.

use vitis_core::VitisTask;

#[test]
fn task_new_assigns_fields() {
    let t = VitisTask::new(7, "RUN make", 2, vec![1, 3]);
    assert_eq!(t.id, 7);
    assert_eq!(t.instruction, "RUN make");
    assert_eq!(t.indent_level, 2);
    assert_eq!(t.depends_on, vec![1, 3]);
}

#[test]
fn task_new_accepts_string_and_str() {
    let from_str = VitisTask::new(0, "FROM a", 0, vec![]);
    let from_string = VitisTask::new(1, String::from("RUN b"), 2, vec![0]);
    assert_eq!(from_str.instruction, from_string.instruction.replace("RUN b", "FROM a"));
    assert_eq!(from_str.instruction, "FROM a");
}

#[test]
fn task_equality_by_value() {
    let a = VitisTask::new(0, "FROM a", 0, vec![]);
    let b = VitisTask::new(0, "FROM a", 0, vec![]);
    assert_eq!(a, b);
}

#[test]
fn task_inequality_on_instruction() {
    let a = VitisTask::new(0, "FROM a", 0, vec![]);
    let b = VitisTask::new(0, "FROM b", 0, vec![]);
    assert_ne!(a, b);
}

#[test]
fn task_inequality_on_deps() {
    let a = VitisTask::new(1, "RUN b", 2, vec![0]);
    let b = VitisTask::new(1, "RUN b", 2, vec![]);
    assert_ne!(a, b);
}

#[test]
fn task_debug_contains_fields() {
    let t = VitisTask::new(2, "RUN c", 4, vec![1]);
    let debug = format!("{:?}", t);
    assert!(debug.contains("id: 2"));
    assert!(debug.contains("RUN c"));
    assert!(debug.contains("depends_on: [1]"));
}

#[test]
fn task_clone_is_deep_copy() {
    let t = VitisTask::new(3, "COPY . .", 2, vec![0, 1]);
    let c = t.clone();
    assert_eq!(t, c);
}

#[test]
fn task_with_empty_deps() {
    let t = VitisTask::new(0, "FROM a", 0, vec![]);
    assert!(t.depends_on.is_empty());
}

#[test]
fn task_indent_levels_recorded() {
    let flat = VitisTask::new(0, "A", 0, vec![]);
    let nested = VitisTask::new(1, "B", 8, vec![0]);
    assert_eq!(flat.indent_level, 0);
    assert_eq!(nested.indent_level, 8);
}

#[test]
fn task_ordering_in_vec_preserved() {
    let tasks = vec![
        VitisTask::new(0, "a", 0, vec![]),
        VitisTask::new(1, "b", 2, vec![0]),
        VitisTask::new(2, "c", 2, vec![1]),
    ];
    assert_eq!(tasks[0].id, 0);
    assert_eq!(tasks[1].id, 1);
    assert_eq!(tasks[2].id, 2);
}

#[test]
fn task_dependency_cycle_detection_helper() {
    // Two tasks that depend on each other form a 2-cycle.
    let a = VitisTask::new(0, "A", 0, vec![1]);
    let b = VitisTask::new(1, "B", 0, vec![0]);
    let mut seen = std::collections::HashSet::new();
    let mut cur = Some(0usize);
    let mut cyclic = false;
    while let Some(id) = cur {
        if !seen.insert(id) {
            cyclic = true;
            break;
        }
        cur = match id {
            0 => a.depends_on.first().copied(),
            1 => b.depends_on.first().copied(),
            _ => None,
        };
    }
    assert!(cyclic);
}
