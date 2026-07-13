name: Pull Request

on:
  - workflow_dispatch

---

## Description
What does this PR do and why?

## Related issues
Closes #

## Checklist
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace --all-targets` passes
- [ ] Docs updated (README / `.env.example`) if behavior changed
- [ ] I agree to license my contribution under the MIT License
