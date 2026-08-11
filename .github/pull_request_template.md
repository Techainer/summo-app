## What this changes, and why

<!-- The diff shows what. Say why — the trade-off, or the bug that would come back without it. -->

## How you know it works

<!-- What you ran, and what you saw. "Tests pass" is the floor, not the answer: the interesting
     part is usually the thing you checked by hand, especially if it is visual. -->

## Checklist

- [ ] `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- [ ] `pnpm -C apps/web check`
- [ ] Anything visual: `node apps/web/e2e/shots.mjs …` and looked at the pictures
- [ ] No interface text written into a component — it goes in `src/i18n/*.json`
- [ ] A change to the shape of the system has an ADR in `docs/adr/`
