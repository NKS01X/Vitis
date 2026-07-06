# Python ML heavy-data-processing example

Machine-learning pipelines are embarrassingly parallel at the stage level. This
Vitisfile runs **data preparation**, **model training**, and **evaluation** as
three concurrent branches. Each installs only the dependencies it needs, so
there is no global environment to reconcile.

## Layout

```
.
├── data/          # raw inputs
├── train/         # training scripts
├── evaluate/      # metric scripts
└── package.py     # assembles the artifact
```

## Run it

```bash
cd examples/python-ml
vitis build --dry-run
vitis build -t ml-pipeline:1.0
```

The merge point waits for all three branches, then packages the model, metrics,
and prepared data into a single tarball.
