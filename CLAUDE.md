# runtime-matters

Stuart owns what and why. Claude owns how.

## Invariants

- Validate assumptions before acting.
- Keep files under 700 lines.
- Keep functions around 150 lines or less.
- Keep shared behavior DRY.
- For CLI shape, think `kubectl`: stable verbs, positional targets, and flags for modifiers.
- Prove changes with `just check && just build && just test` before commit.
