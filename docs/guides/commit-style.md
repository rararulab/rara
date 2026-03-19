# Commit Style — Conventional Commits (MANDATORY)

Every commit message MUST follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description> (#N)

<optional body>

Closes #N
```

- **Allowed types**: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`, `revert`
- **Scope** matches crate or area: `feat(kernel):`, `fix(web):`, `refactor(memory):`
- **Breaking changes** use `!`: `feat(api)!: remove deprecated endpoint`
- Include `(#N)` issue reference in commit subject
- Include `Closes #N` in commit body
- A local `commit-msg` hook (`scripts/check-conventional-commit.sh`) enforces this — do NOT bypass it
- Do NOT use free-form commit messages like `"update code"` or `"fix stuff"` — they will be rejected
