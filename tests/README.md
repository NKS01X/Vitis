# Vitis Exclusive Test Suite

Standalone integration tests, one package per crate, living in a single folder
(`tests/`) so every module can be exercised in isolation. Each package imports
the real crate via a `path` dependency and asserts behaviour through the public
API only.

## Layout

```
tests/
  Cargo.toml        # workspace for the non-FUSE test packages
  core/             # vitis-core       -> VitisTask DAG node
  parser/           # vitis-parser     -> scoped Vitisfile DAG parser
  cache/            # vitis-cache      -> content-addressed cache + diff
  export/           # vitis-export     -> OCI tarball + manifest
  registry/         # vitis-registry   -> OCI registry client
  runtime/          # vitis-runtime    -> Linux-namespace sandbox
  cli/              # vitis (cli)      -> Dockerfile converter + dry-run renderer
  storage/          # vitis-storage    -> FUSE lazy-pull fs   (FUSE, standalone)
  engine/           # vitis-engine     -> Kahn topological scheduler (FUSE, standalone)
```

## Run (no FUSE required)

The seven non-FUSE packages build and run anywhere:

```bash
cd tests
cargo test
```

This runs **144 tests** across core (11), parser (29), cache (25), export (22),
registry (18), runtime (8), and cli (21).

## Run the FUSE packages (requires libfuse3-dev)

`storage` and `engine` link against the system FUSE library, so they are kept
out of the default workspace and run separately with the `fuse` feature:

```bash
sudo apt-get install -y libfuse3-dev pkg-config

cd tests/storage && cargo test --features fuse
cd tests/engine  && cargo test --features fuse
```

CI installs `libfuse3-dev` (see `.github/workflows/ci.yml`) and runs all of the
above, including the `fuse` packages.

## What is covered

- **core** — task construction, equality, cloning, dependency-cycle detection.
- **parser** — indentation semantics: sequential chains, blank-line parallel
  branches, dedent merge points, multiple `FROM` roots, comments, deep nesting.
- **cache** — deterministic SHA-256 keys, store/load round-trips, byte-range
  diff, parent-hash merging, memoization.
- **export** — SHA-256 helpers, gzipped layer compaction, OCI manifest build and
  round-trip, snapshot packaging.
- **registry** — URL building, error types, manifest/blob JSON parsing, retry
  exhaustion over an unreachable endpoint.
- **runtime** — empty-command rejection, error conversions, `Send + Sync`,
  real sandbox execution where permitted.
- **cli** — Dockerfile→Vitisfile conversion, group-start recognition, dry-run
  tree rendering, orphan detection.
- **storage** *(fuse)* — inode lookup, attribute building, mock OCI tree shape,
  chunk/TTL constants, unreachable manifest fetch.
- **engine** *(fuse)* — Kahn scheduling: chains, diamonds, parallel branches,
  failure propagation/blocking, topological ordering, end-to-end parse+run.
