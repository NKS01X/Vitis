# Rust + Go multi-language compilation example

Two languages, one build. The Rust and Go branches are siblings at the same
indent, so Vitis compiles them at the same time. Each branch installs its own
toolchain, copies its own sources, and builds — completely isolated.

## Layout

```
.
├── rust-service/   # cargo project
└── go-service/     # go module
```

## Run it

```bash
cd examples/rust-go
vitis build --dry-run
vitis build -t rust-go:1.0
```

Because the branches are independent, adding a third language later is just
another sibling block — no restructuring required.
