# mstlo Documentation

Background and reference material for mstlo. For installation and a quick start, see the [main README](../README.md).

<!-- TOC -->

- [mstlo Documentation](#mstlo-documentation)
  - [Contents](#contents)
  - [API Documentation](#api-documentation)

<!-- /TOC -->

## Contents

- [Signal Temporal Logic and Evaluation Semantics](signal_temporal_logic.md): the STL syntax supported by mstlo and the formal definitions of the four online semantics (delayed qualitative, delayed quantitative, RoSI and eager qualitative).
- [Implementation](implementation.md): the evaluation algorithm, the signal model (zero-order hold and linear interpolation), how input is consumed and when verdicts are emitted, and settings that can hurt performance.

## API Documentation

- Rust API: [docs.rs/mstlo](https://docs.rs/mstlo)
- Python API: [INTO-CPS-Association.github.io/mstlo](https://INTO-CPS-Association.github.io/mstlo/)
