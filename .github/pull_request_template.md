Closes #

## What and why

<!-- What changed, and the reason. Call out anything that departs from the ticket. -->

## Checklist

- [ ] `cargo xtask ci` passes locally
- [ ] New behaviour has tests
- [ ] New or changed app: still at or above the line-coverage bar, with tests in `src/tests.rs`
- [ ] New app: registered in `crates/ccosel-shell/src/registry.rs` and in the `guests` list in `build_web()` (`xtask/src/main.rs`)
