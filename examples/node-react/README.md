# Node.js + React parallel build example

This Vitisfile builds a React frontend and a Node.js backend **concurrently**.
The two `RUN ... build` branches share the same indent under `WORKDIR`, so Vitis
runs them on separate CPU cores. The final `RUN tar` drops back to column 0 and
waits for both branches before packaging.

## Layout

```
app/
├── frontend/   # React app
└── backend/    # Node.js service
```

## Run it

```bash
cd examples/node-react
vitis build --dry-run      # preview the parallel graph
vitis build -t node-react:1.0
```

In a sequential engine this build takes `frontend_time + backend_time`. In Vitis
it takes `max(frontend_time, backend_time)`.
