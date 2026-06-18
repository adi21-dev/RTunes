# Contributing to RTunes

Welcome, and thank you for considering a contribution to RTunes! This is the author's first open source project, and every contribution — no matter how small — means a lot. Whether you are an experienced Rust developer or just getting started, you are welcome here.

---

## Code of Conduct

All contributors are expected to follow the [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Please read it before participating.

---

## Ways to Contribute

### Reporting Bugs

Found something broken? Open an issue using the **Bug Report** template. Before filing, search existing issues to avoid duplicates. Include as much detail as possible — see [Issue Guidelines](#issue-guidelines) below.

### Suggesting Features

Have an idea for RTunes? Open an issue using the **Feature Request** template. Explain the problem you are trying to solve and why the feature would be valuable. See [Issue Guidelines](#issue-guidelines) for what to include.

### Improving Documentation

Documentation fixes and improvements are always welcome. This includes the README, inline code comments, and files in `docs/`. No issue is required for small doc fixes — open a PR directly.

### Writing Code

Pick up an open issue, implement a fix or feature, and open a pull request. If you want to work on something that does not have an issue yet, open one first so the work can be discussed before you invest time in it.

---

## Development Setup

1. **Prerequisites** — Install the Rust stable toolchain and `cargo` via [rustup](https://rustup.rs/).

2. **Clone the repository**
   ```bash
   git clone https://github.com/TheCoder1232/RTunes.git
   cd RTunes
   ```

3. **Build the project**
   ```bash
   cargo build
   ```

4. **Format your code** — `cargo fmt` applies the standard Rust formatting rules so the codebase stays consistent.
   ```bash
   cargo fmt --all
   ```

5. **Lint your code** — `cargo clippy` catches common mistakes and non-idiomatic Rust patterns.
   ```bash
   cargo clippy -- -D warnings
   ```

6. **Run the tests** — `cargo test` compiles and runs all unit and integration tests.
   ```bash
   cargo test
   ```

---

## Branch Naming Convention

Use one of the following prefixes when creating a branch:

| Prefix | Purpose | Example |
|--------|---------|---------|
| `feat/` | New features | `feat/oscilloscope-renderer` |
| `fix/` | Bug fixes | `fix/audio-device-fallback` |
| `docs/` | Documentation changes | `docs/update-setup-steps` |
| `chore/` | Maintenance tasks | `chore/update-dependencies` |

---

## Commit Message Style

This project follows [Conventional Commits](https://www.conventionalcommits.org/). Keep the subject line under 72 characters and use the imperative mood.

```
feat: add phosphor visualizer renderer
fix: prevent panic when audio device is missing
docs: clarify development setup prerequisites
chore: bump ratatui to 0.29
refactor: simplify FFT bin aggregation logic
```

---

## Before Opening a PR

- [ ] `cargo fmt` has been run
- [ ] `cargo clippy` passes with zero warnings
- [ ] `cargo test` passes
- [ ] PR is linked to an existing issue
- [ ] PR title follows conventional commit style

---

## Pull Request Process

1. Fork the repository and create a branch using the naming convention above.
2. Make your changes, keeping commits focused and atomic.
3. Run `cargo fmt --all`, `cargo clippy -- -D warnings`, and `cargo test` — all must pass.
4. Open a pull request targeting the `dev` branch.
5. In the PR description, link the related issue using `Closes #<issue-number>`.
6. Wait for a review. Address any requested changes in follow-up commits.

> **Note:** PRs without a linked issue will not be merged. Open an issue first if one does not exist.

---

## Issue Guidelines

### Bug Reports

Before opening a bug report, search existing issues to see if it has already been reported. If not, include:

- **Operating system and architecture** (e.g. Ubuntu 24.04, x86\_64)
- **RTunes version** (output of `rtunes --version`)
- **Rust version** (output of `rustc --version`)
- **Steps to reproduce** — the exact sequence of actions that triggers the bug
- **Expected behaviour** — what you expected to happen
- **Actual behaviour** — what actually happened, including any error output

### Feature Requests

Explain the *why* behind the request, not just the *what*. Include:

- The problem or limitation you are running into
- Your proposed solution
- Any alternatives you have considered and why you ruled them out

---

## Good First Issues

New to the project or to open source? Look for issues tagged [`good first issue`](https://github.com/TheCoder1232/RTunes/labels/good%20first%20issue) — these are scoped to be approachable without deep knowledge of the codebase.

---

## Thank You

Every contribution, from a typo fix to a new feature, helps make RTunes better. Thank you for taking the time. For a broader overview of the project, see the [README.md](README.md).

