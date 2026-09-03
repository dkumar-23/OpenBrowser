# Wave 1 — Scout Recon (low)

## Workspace state
- `/home/linux-user/Documents/Projects/OpenBrowser/`
- Empty project: no Cargo.toml, no src/, no Rust code yet.
- Existing files:
  - `context.md` (15122 bytes) — full system prompt / requirements
  - `architect-output.md` — synthesized design
  - `architect-todo.md` — phase tracker
  - `swarm-plan.md` — workflow plan
  - `swarm-trace.md` — execution log
  - `architect-sketch/` — candidate A + B
  - `architect-synthesis/` — synthesis C
  - `swarm-output/` — (new, contains wave outputs)

## Project state: GREENFIELD
- No Cargo workspace.
- No dependencies.
- Must be initialized as Rust workspace from scratch.
- Phase 1 starts at zero.

## Key artifacts for planning
- `context.md` = ground truth for all design decisions
- `architect-output.md` = binding design sketch (Candidate C)
- `swarm-output/wave1-architect.md` = concrete Phase 1 crate/trait contracts

## Recommendations
- Initialize Cargo workspace FIRST, then add Phase 1 crates.
- Start with `runtime-observability` since everything depends on it.
- Test infrastructure: `cargo test` for units, `cargo run --example` for integration scenarios.
- No CI/CD yet (Phase 1).
