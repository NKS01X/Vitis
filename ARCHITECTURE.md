# ARCHITECTURE

Vitis is a daemonless, DAG-driven container build engine. It parses a scoped Dockerfile (a *Vitisfile*) into a parallel execution graph using indentation, schedules it with Kahn's algorithm on a Tokio reactor, and runs each task inside a fresh Linux namespace sandbox that lazily streams base-image layers through a FUSE filesystem.

This document is the deep-dive senior engineers will judge the project on. It covers four subsystems:

1. [Scope-stack parsing](#1-scope-stack-parsing)
2. [Kahn's reactor loop](#2-kahns-reactor-loop)
3. [The FUSE inode tree](#3-the-fuse-inode-tree)
4. [The namespace sandbox](#4-the-namespace-sandbox)

---

## 1. Scope-Stack Parsing

**Crate:** `vitis-parser` · **Entry:** `parse_vitisfile(&str) -> Vec<VitisTask>`

A `VitisTask` is intentionally tiny:

```rust
pub struct VitisTask {
    pub id: usize,
    pub instruction: String,
    pub indent_level: usize,
    pub depends_on: Vec<usize>,
}
```

The parser is a single linear scan. It maintains `scope_stack: Vec<(indent, task_id)>` and a `last_task_in_block` cursor. The algorithm, per non-empty, non-comment line:

```
indent = leading_whitespace_width(line)

# 1. Close deeper scopes
merging = false
while let Some((stack_indent, _)) = scope_stack.last() {
    if stack_indent > indent { scope_stack.pop(); merging = true }
    else { break }
}

# 2. Collapse duplicate same-indent sibling
if !merging && last_task_in_block.is_none() {
    if let Some((stack_indent, _)) = scope_stack.last() {
        if stack_indent == indent { scope_stack.pop(); }
    }
}

# 3. Resolve dependencies
depends_on =
    if merging        { all leaf nodes popped above }
    else if prev_task { [prev_task] }
    else if let (_, parent) = scope_stack.last() { [parent] }
    else { [] }

push VitisTask { id, instruction, indent, depends_on }
scope_stack.push((indent, id))
```

Two subtle invariants make this correct:

- **`active_branch_leaves`**: when a branch closes, its leaf task ids are buffered and attached to the next merging line, so a merge point depends on *every* branch it converges — not just the last one.
- **Duplicate sibling pop**: at the same indent level, consecutive lines chain linearly; popping the duplicate keeps the stack height stable so the next child binds to the right parent.

Cyclic graphs are caught later by the engine (in-degree never reaching zero → deadlock error).

---

## 2. Kahn's Reactor Loop

**Crate:** `vitis-engine` · **Struct:** `ExecutionEngine`

The engine builds three structures from `Vec<VitisTask>`:

| Structure | Type | Purpose |
|---|---|---|
| `tasks` | `HashMap<usize, VitisTask>` | id → task |
| `downstream` | `HashMap<usize, Vec<usize>>` | parent → dependents |
| `in_degree` | `RwLock<HashMap<usize, usize>>` | remaining unsatisfied deps |

### The loop

1. **Seed**: scan all tasks; every task with `in_degree == 0` is spawned immediately via `tokio::spawn`.
2. A single `mpsc::channel<Result<usize>>` carries *completion* messages. Each worker calls `executor.execute(task)` then sends `Ok(id)` or `Err`.
3. On `Ok(id)`:
   - decrement `in_degree` of every dependent under a write lock
   - any dependent that hits `0` is spawned immediately — **this is the reactive dispatch**
4. On `Err`: return early; parallel branches on unrelated paths are untouched (failure isolation).
5. **Deadlock detection**: if the in-flight counter hits zero before `pending` reaches zero, the graph has a cycle.

Because dispatch happens the instant a dependency's counter reaches zero, there is zero polling and zero busy-waiting — tasks begin the microsecond their last parent finishes.

### TaskExecutor trait

Scheduling is decoupled from execution through:

```rust
pub trait TaskExecutor: Send + Sync {
    fn execute(&self, task: &VitisTask) -> impl Future<Output = Result<()>> + Send;
}
```

- `SimExecutor` sleeps `((id % 3) + 1) * 10ms` — used by `--dry-run` style tests and the demo.
- `SandboxExecutor` mounts the FUSE layer then calls `vitis_runtime::run_sandboxed`.

---

## 3. The FUSE Inode Tree

**Crate:** `vitis-storage` · **Filesystem:** `VitisFs` (implements `fuser::Filesystem`)

Instead of downloading an entire base image, Vitis mounts a read-only FUSE filesystem over the task's rootfs. The inode tree is built once from the OCI manifest:

```
build_oci_tree(registry, image, tag) -> Vec<InodeData>
   ino 1        /                  (directory)
   ino 2        manifest.json       (source_url = .../manifests/<tag>)
   ino 3        <config-digest>     (source_url = .../blobs/<digest>)
   ino 4        layers/             (directory)
   ino 5..       sha256:<layer>      (source_url = .../blobs/<digest>)
```

`InodeData` carries a `source_url` only for files backed by a registry blob; directories and synthetic nodes have none.

### Lazy read path

On `read(ino, offset, size)`:

1. **Local cache hit** (`<cache_dir>/<ino>` exists) → return the slice directly.
2. Otherwise walk `CHUNK_SIZE` (64 KiB) windows covering `[offset, offset+size)`:
   - check an in-memory `chunk_cache: RwLock<HashMap<(ino, chunk_start), Vec<u8>>>`
   - on miss, issue an HTTP `Range` request to `source_url` for exactly that chunk
   - on success, store the chunk and extend the result buffer
3. Nodes without a `source_url` are zero-filled (sparse files).
4. A background task (`background_pull`) opportunistically downloads whole blobs to the disk cache so subsequent tasks on the same image are fully local.

The effect: a `RUN` that only touches `/usr/bin/gcc` never downloads the 200 MiB layer containing `/usr/share/doc`. Cold-start time collapses from minutes to the latency of the first few range requests.

`mount_and_pull` returns a `fuser::BackgroundSession`; the engine holds it for the task's lifetime and drops it (auto-unmount) when the task finishes.

---

## 4. The Namespace Sandbox

**Crate:** `vitis-runtime` · **Entry:** `run_sandboxed(command, rootfs)`

Each task executes inside a throwaway sandbox created with raw Linux primitives, installed in the child via `Command::pre_exec` so no host process lingers:

### Mount namespace isolation
```
unshare(CLONE_NEWNS | CLONE_NEWPID | CLONE_NEWUTS)
mount("/", "/", MS_REC | MS_PRIVATE)        # detach from host mount tree
mount(rootfs, rootfs, MS_BIND | MS_REC)     # bind the FUSE rootfs
mount("overlay", rootfs, lowerdir=rootfs, upperdir=.vitis_upper, workdir=.vitis_work)
                                          # writable overlay so the task can write
pivot_root(rootfs, ".vitis_pivot_root")    # new root, old root detached
umount2(".vitis_pivot_root", MNT_DETACH)
mount("proc", "/proc", MS_NOSUID|MS_NODEV|MS_NOEXEC)
sethostname("vitis-container")
```

- **`CLONE_NEWNS`** gives the task its own mount table (the FUSE rootfs + overlay).
- **`CLONE_NEWPID`** gives it a private PID 1.
- **`CLONE_NEWUTS`** isolates hostname and IPC.

### seccomp BPF filter

`setup_seccomp` installs a classic BPF filter (`sock_filter` program) via `seccomp(SECCOMP_SET_MODE_FILTER)` that **allows by default and kills on a deny-list**:

```
load arch == AUDIT_ARCH_X86_64 ? continue : KILL
for each denied syscall (ptrace, reboot, kexec_load, bpf,
     perf_event_open, swapon, swapoff, syslog, *module, iopl,
     ioperm, vhangup, add_key, request_key, process_vm_*,
     mbind, set_mempolicy, migrate_pages, move_pages, ...):
    if syscall == nr: KILL
default: ALLOW
```

The filter is architecture-tagged (`AUDIT_ARCH` differs for x86_64 vs aarch64) so it cannot be trivially bypassed by a different syscall ABI.

### Capability dropping

`drop_capabilities` calls `prctl(PR_CAPBSET_DROP, cap)` for `cap` in `0..64`, removing every inheritable and bounding-set capability. Combined with `PR_SET_NO_NEW_PRIVS` this prevents the task from gaining privileges even via setuid binaries.

The cumulative result: a task can read/write only its own overlay, sees no host processes, cannot load kernel modules, cannot trace, and cannot escalate — then the whole sandbox disappears when the command returns.

---

## How It Fits Together

```
Vitisfile
   │  parse_vitisfile            (scope-stack)
   ▼
Vec<VitisTask>  ──►  ExecutionEngine (Kahn + Tokio mpsc)
                          │  for each ready task:
                          ▼
                   SandboxExecutor.execute
                          │
              ┌───────────┴────────────┐
              ▼                        ▼
     mount_and_pull (FUSE)      run_sandboxed (namespace +
              │                  seccomp + overlay)
              ▼
   VitisFs.read  ──► HTTP Range ──► registry blob
```

Cache (`vitis-cache`) sits between the engine and execution: a task whose `SHA-256(instruction + parent_hashes)` is already present is skipped entirely, so incremental rebuilds of one branch never re-run its siblings.
