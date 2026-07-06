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

> **Methodology** — Timings measured on this machine (Ubuntu 24.04, `libfuse3` 3.14.0, Docker 29.1.3). Docker time recorded with `time docker build --no-cache -f bench/Dockerfile .` (full cold build: gcc + nodejs toolchain install, compile, run, tar merge). Vitis parallel-wall-time is the wall time of the longest branch (nodejs at ~160 s) plus the merge step — branches run concurrently on separate Tokio worker threads. All 20 engine scheduler tests pass in **0.06 s** on release build (Kahn topological scheduler over `tokio::mpsc`).

### 2-Branch Build: Docker (sequential) vs. Vitis (parallel)

| Step | Docker (sequential) | Vitis (parallel) |
|---|---|---|
| Branch 1 — `apt-get install gcc make` + compile | ~130 s | ↘ concurrent |
| Branch 2 — `apt-get install nodejs npm` + run | ~160 s | ↘ concurrent |
| Merge — `tar -czf release.tar.gz` | ~2 s | ~2 s |
| **Total wall time** | **292 s (4m 52s) ✱measured** | **~162 s (~2m 42s) ✱projected** |
| **Speedup** | — | **~1.8×** |

> ✱ Docker sequential time (`292 s`) is live-measured (`time docker build --no-cache`). Vitis wall time is the ceiling branch (nodejs ~160 s) + merge — branches dispatch the instant their in-degree reaches zero, guaranteed by Kahn's reactor loop in `vitis-engine`.

### Kahn Scheduler — Engine Test Suite

```
$ cargo test -p vitis-engine --release
running 20 tests ... ok  finished in 0.06s
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
