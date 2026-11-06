# CLAUDE.md

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) for every commit:

```
<type>(<optional scope>): <short imperative summary>

<optional body>

<optional footer(s)>
```

- Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`.
- Keep the summary lowercase, imperative, and under ~72 characters, with no trailing period.
- Mark breaking changes with `!` after the type/scope (e.g. `feat(api)!: ...`) and/or a `BREAKING CHANGE:` footer.
