# SYNTAX GUIDE

The Vitisfile uses indentation to express dependency and parallelism. It is a minimal syntax that converts any sequential Dockerfile into a parallel DAG. Read this in two minutes and you will understand the entire scoping model.

## The One Rule

```
Same indent  → runs in parallel (siblings)
Deeper indent → depends on the nearest shallower ancestor
Shallower     → merge point: waits for the leaves of the branch it closes
```

## 30-Second Example

```dockerfile
FROM ubuntu:24.04
WORKDIR /workspace

    RUN apt-get update && apt-get install -y gcc   # branch 1
    COPY ./backend ./backend
    RUN make build-backend

    RUN apt-get update && apt-get install -y nodejs # branch 2
    COPY ./frontend ./frontend
    RUN npm run build

RUN tar -czf release.tar.gz ./backend/dist ./frontend/dist  # merge
```

- `branch 1` and `branch 2` are indented equally under `WORKDIR`, so they run concurrently.
- The final `RUN tar` drops back to column 0, so it waits for **both** branches before running.

## Three Core Behaviors

### 1. Sequence (same indent, no blank line)
Consecutive lines at the same indent run one after another. Each depends only on the line immediately above it.

```dockerfile
RUN apt-get update
RUN apt-get install -y curl     # runs after the line above
```

### 2. Branch (blank line + deeper indent)
A blank line followed by deeper indentation spawns a new parallel branch. The parser walks the scope stack backward to the first line with strictly less indent and binds to it.

```dockerfile
RUN apt-get update

    RUN make server      # new branch, parent = apt-get update
    RUN make client      # parallel sibling of "make server"
```

### 3. Merge (shallower indent)
When indentation drops, the engine collects the leaf nodes of every branch just closed and makes the new line depend on all of them.

```dockerfile
    RUN make server
    RUN make client

RUN ./test-all           # depends on both make server and make client
```

## Scope-Stack Algorithm

The parser (`vitis-parser`) keeps a `scope_stack` of `(indent, task_id)` pairs and processes the file line by line:

1. **Pop** every stack entry whose indent is greater than the current line's indent. If anything was popped, this is a "merging" line.
2. If not merging and the previous line was at the same indent, pop the duplicate so siblings chain correctly.
3. Compute `depends_on`:
   - **merging** → all leaf nodes collected during the pop (the branch heads)
   - **otherwise** → the last task at the same indent, or the parent on the stack
4. Push `(indent, id)` onto the stack.

This produces a `Vec<VitisTask>` where every task carries its `depends_on` vector — the DAG handed to the scheduler.

## Comments and Blank Lines

- Lines starting with `#` are ignored entirely (including indented comments).
- Blank lines separate parallel branches; they do **not** create tasks.
- Inline comments (`RUN foo # note`) are preserved as part of the instruction string.

## Migration from Docker

`vitis init Dockerfile` rewrites a linear Dockerfile into a scoped Vitisfile automatically: every instruction after a `FROM` is indented, and blank lines are inserted between independent groups so the engine parallelizes them.

See `examples/` for ready-to-run Vitisfiles (frontend/backend, multi-language, and ML pipelines).
