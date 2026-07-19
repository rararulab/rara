## What & why

<!-- Explain your change as a story a reviewer can follow WITHOUT reading the diff first:
  - What problem or goal prompted this? (the issue's Intent — don't just restate the title)
  - What approach did you take, and *why this one*? Name the key design decision.
  - What did you consider and reject, or what non-obvious constraint/tradeoff shaped it?
  The diff shows WHAT changed. This section must explain WHY it looks like this.
  If a reviewer would have to reverse-engineer your reasoning from the code, this section is too thin. -->

## Type of change

<!-- Check the one that applies and add the matching label to this PR -->

| Type | Label |
|------|-------|
| Bug fix | `bug` |
| New feature | `enhancement` |
| Breaking change | `breaking-change` |
| Refactor | `refactor` |
| CI / Infrastructure | `ci` |
| Maintenance | `chore` |
| Documentation | `documentation` |

## Component

<!-- Add the component label that best matches the area you changed -->
<!-- `core` · `backend` · `ui` · `extension` · `ci` -->

## Closes

<!-- REQUIRED: Link the issue this PR resolves. PR merge will auto-close the issue. -->
<!-- If no issue exists, create one first: gh issue create -->

Closes #

## How to verify

<!-- Commands you ran or steps a reviewer can repeat — evidence, not an exhaustive command dump. -->
- [ ] `just test` / relevant `cargo test -p …` passes
- [ ] `just lint` passes
- [ ] Tested locally (say how)
