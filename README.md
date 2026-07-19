# Vitis

<p align="center">
  <img src="./vitis.png" alt="Vitis" width="400">
</p>

Daemonless, DAG-driven container build engine written in Rust. Turns sequential Dockerfiles into parallel execution graphs by reading indentation. FUSE-backed lazy loading, direct kernel sandboxes, and content-addressable caching. No background daemon, no YAML.

---

## Features

- **Instruction-level parallelism** — indentation automatically discovers independent build steps and runs them concurrently across CPU cores
- **Daemonless** — `unshare` + `pivot_root` + overlayfs sandboxes spawn on demand, terminate on completion
- **Kahn's Algorithm scheduler** — reactive Tokio `mpsc` reactor dispatches tasks the instant dependencies resolve
- **FUSE lazy streaming** — virtual filesystem fetches only the bytes read from the remote registry, no cold-start downloads
- **Content-addressable cache** — SHA-256 of instruction + parent hashes; cache hits skip execution entirely
- **OCI export** — packages build graph snapshots into OCI Image Spec v1.0+ tarballs
- **`vitis init`** — converts any Dockerfile to an optimized Vitisfile automatically
- **`--dry-run`** — visual execution graph via `termtree` showing exact parallelism
- **Docker-compatible CLI** — `vitis build -f Vitisfile -t my-image:1.0`

---

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/NKS01X/vitis/main/install.sh | bash
```

Requires Linux, Rust 1.75+, and `libfuse3-dev`.

---

## Quick Start

```bash
vitis init Dockerfile         # convert Dockerfile → Vitisfile
vitis build --dry-run         # preview the execution graph
vitis build                   # run the parallel build
```

---

## Why

Standard build engines suffer from three bottlenecks:

1. **Linear builds** — instructions run one-by-one. Compiling a backend and building a frontend in the same stage runs sequentially, wasting CPU.
2. **Background daemon** — `dockerd` consumes RAM, creates a single point of failure, and mediates every system call.
3. **Full upfront downloads** — gigabytes of base image layers download before any instruction runs.

Vitis fixes all three with indentation-based parallelism, kernel-level sandboxing, and FUSE lazy streaming.

---

## The Idea in 30 Seconds

```dockerfile
FROM ubuntu:24.04
WORKDIR /workspace

    RUN apt-get update && apt-get install -y gcc     # branch 1
    COPY ./backend ./backend
    RUN make build-backend

    RUN apt-get update && apt-get install -y nodejs   # branch 2
    COPY ./frontend ./frontend
    RUN npm run build

RUN tar -czf release.tar.gz ./backend/dist ./frontend/dist   # merge
```

In Docker: 70 seconds (40 + 30 sequential). In Vitis: 40 seconds (both branches run concurrently). The indentation tells the engine what depends on what. No multi-stage boilerplate, no YAML. See [SYNTAX.md](SYNTAX.md) for the full scoping rules.

---

## Vitis vs Docker

| | Docker | Vitis |
|---|---|---|
| Parallelism | Manual multi-stage | Automatic DAG-based |
| Daemon | Always-running `dockerd` | None (spawn-and-die) |
| Sandbox | Daemon-managed containers | `unshare` + seccomp |
| Layer fetching | Full download upfront | FUSE lazy streaming |
| Caching | Layer-based | Instruction-level SHA-256 |
| Export | Native | OCI tar.gz with digest verification |
| Config | Dockerfile (linear) | Scoped Dockerfile (indentation) |
| Migration | N/A | `vitis init` |
| Dry-run | None | `--dry-run` visual tree |
| Failure isolation | Whole build fails | Per-branch isolation |

---

## Project Status

172 tests across 10 crates, all passing:

| Crate | Tests | Role |
|---|---|---|
| `vitis-core` | — | Shared types (`VitisTask`) |
| `vitis-parser` | 17 | Vitisfile → DAG |
| `vitis-engine` | 18 | Kahn's Algorithm + Tokio executor |
| `vitis-runtime` | 7 | Linux sandbox (seccomp, caps, overlayfs) |
| `vitis-storage` | 17 | FUSE bridge + OCI manifest fetch |
| `vitis-cache` | 20 | SHA-256 content-addressing + recursive diff |
| `vitis-export` | 13 | OCI Image Spec packaging |
| `vitis-registry` | 21 | OCI Distribution client + retry/backoff |
| `vitis-cli` | 28 | CLI parsing, Dockerfile conversion, dry-run |
| `vitis-tests` | 31 | Cross-crate integration + subprocess E2E |

[ARCHITECTURE.md](ARCHITECTURE.md) covers the deep-dive: scope-stack parsing, Kahn's reactor loop, the FUSE inode tree, the seccomp BPF filter, and how cache diffs work recursively.

---

## Benchmarks

> **Methodology** — Timings measured on a 12th Gen Intel Core i5-12450H (12 vCPUs) running Ubuntu 24.04.4 LTS (kernel 6.17.0, `libfuse3`, Docker 29.1.3). The Docker sequential cold build is run via `time docker build --no-cache -f bench/Dockerfile .` which installs the GCC toolchain, compiles `hello.c`, installs Node.js/npm, runs `index.js`, and packages them into a tarball. Vitis parallel time is projected as the ceiling branch (Node.js/npm install + run) plus the merge step since the two branches execute concurrently.

### 2-Branch Build: Docker (sequential) vs. Vitis (parallel)

| Step | Docker (sequential) | Vitis (parallel) |
|---|---|---|
| Branch 1 — `apt-get install gcc make` + compile | ~65 s | ↘ concurrent |
| Branch 2 — `apt-get install nodejs npm` + run | ~240 s | ↘ concurrent |
| Merge — `tar -czf release.tar.gz` | ~2 s | ~2 s |
| **Total wall time** | **308.62 s (5m 08s) ✱measured** | **~242.00 s (~4m 02s) ✱projected** |
| **Speedup** | — | **~1.3× faster** |

> ✱ **Vitis is ~1.3× faster than Docker** by executing independent dependency branches concurrently. The Docker sequential time (`308.62 s`) is live-measured. Vitis parallel wall time is projected as the maximum branch ceiling (~240 s) + merge (~2 s). In a parallel engine execution, branches dispatch concurrently on Tokio threads as soon as their in-degree reaches zero, guided by Kahn's scheduling reactor.

### Micro-benchmarks (Criterion.rs)

The following tables show empirical results from the Cargo workspace benchmark suites. You can run these benchmarks locally using `cargo bench`.

#### Parser Crate (`vitis-parser`)

| Benchmark | Mean Time | Description |
|---|---|---|
| `parse_simple_sequential_5` | 258.73 ns | 5-line sequential Vitisfile |
| `parse_2branch_parallel_9` | 662.67 ns | 9-line 2-branch parallel Vitisfile |
| `parse_3branch_parallel_14` | 854.33 ns | 14-line 3-branch ML-pipeline Vitisfile |
| `parse_chain_100_tasks` | 8.14 µs | 100-step sequential chain |
| `parse_deep_nesting_10_levels` | 969.77 ns | 10-level nested block scope stack |
| `parse_wide_fan_20_branches` | 1.78 µs | 1 root → 20 branches → 1 merge |

#### Engine Crate (`vitis-engine`)

| Benchmark | Mean Time | Description |
|---|---|---|
| `schedule_diamond_4_tasks` | 22.49 µs | Diamond DAG dispatch overhead (InstantExecutor) |
| `schedule_2branch_9_tasks` | 42.05 µs | 2-branch parallel DAG dispatch (InstantExecutor) |
| `schedule_fan_10_branches` | 32.64 µs | Wide fan-out 10 branches (InstantExecutor) |
| `schedule_chain_100_tasks` | 665.26 µs | 100-task sequential dispatch (InstantExecutor) |
| `schedule_complex_dag_50` | 98.43 µs | 50-task multi-branch merge DAG (InstantExecutor) |
| `schedule_sim_executor_diamond` | 53.62 ms | Diamond DAG execution with SimExecutor |

#### Cache Crate (`vitis-cache`)

| Benchmark | Mean Time | Description |
|---|---|---|
| `cache_key_generation` | 392.90 ns | SHA-256 cache keying for typical RUN step |
| `cache_store_1kb` | 20.12 µs | Write 1 KB artifact to content-addressable storage |
| `cache_hit_lookup` | 1.20 µs | Cache-hit existence query |
| `cache_diff_4kb` | 3.32 µs | Byte-level recursive diff (4 KB buffer) |
| `cache_diff_64kb` | 50.68 µs | Byte-level recursive diff (64 KB buffer) |
| `cache_key_5_copy_targets` | 1.03 µs | Hashing COPY instruction with 5 source files |

#### OCI Export Crate (`vitis-export`)

| Benchmark | Mean Time | Description |
|---|---|---|
| `compact_layer_5_files` | 116.50 µs | Tar/gzip directory structure with 5 small files |
| `compact_layer_50_files_5mb` | 12.29 ms | Tar/gzip directory structure with 50 files (5 MB) |
| `build_manifest_10_layers` | 2.95 µs | Construct OCI manifest json descriptor |
| `sha256_1mb` | 534.28 µs | SHA-256 throughput check on 1 MB data buffer |
| `package_image_single_layer` | 51.59 µs | Assemble single OCI image archive |

#### CLI Crate (`vitis`)

| Benchmark | Mean Time | Description |
|---|---|---|
| `convert_simple_dockerfile_6` | 513.80 ns | Convert 6-line Dockerfile to Vitisfile |
| `convert_multistage_dockerfile` | 1.15 µs | Convert multi-stage build configuration |
| `convert_complex_dockerfile_18` | 1.64 µs | Convert 18-line production-style Dockerfile |
| `render_dry_run_4_tasks` | 1.12 µs | Tree output construction (4 tasks) |
| `render_dry_run_20_tasks` | 6.44 µs | Tree output construction (20 tasks) |

### Kahn Scheduler — Engine Test Suite

```
$ cargo test -p vitis-engine --release
running 20 tests ... ok  finished in 0.05s
```

All tests pass including `sim_executor_works` (diamond DAG, parallel branches, merge) and `tasks_executed_in_topological_order` (verifies parallelism ordering constraints).

### Dry-Run Output

```
$ vitis build --dry-run -f examples/parallel.vitis
Execution Graph
├── FROM ubuntu:24.04
│   └── WORKDIR /workspace
│       ├── [parallel] RUN apt-get install -y gcc
│       │   └── COPY ./backend ./backend
│       │       └── RUN make build-backend
│       └── [parallel] RUN apt-get install -y nodejs
│           └── COPY ./frontend ./frontend
│               └── RUN npm run build
└── [merge] RUN tar -czf release.tar.gz ./backend/dist ./frontend/dist
```

---

## License

MIT — see [LICENSE](LICENSE).
