## What and why

<!-- One topic per PR. Link the backlog issue: Refs #123 -->

## Checklist

- [ ] `just check` passes (the pre-push hook runs it before every push; turn it on with `just install-hooks`)
- [ ] A test was added that fails without this change
- [ ] Contract changes (proto, schemas, commands) are in their own PR, merged first
- [ ] New user-facing errors have catalog codes
- [ ] New external dependencies are in the §27 register with a reason and a replacement path
- [ ] Code follows the house style: simple functions, plain types, constants at the top, readable names
- [ ] Every commit is signed off (`git commit -s`)
- [ ] Links its backlog issue (Refs #n, never Closes) and stays within one task
- [ ] Level claimed: none / L0 / L1 / L2 for <feature>, proven by <tests> (and the row in docs/plan section 7.3 is updated)
- [ ] Every new name is in contracts.md (in this PR, or in the contract PR that landed first), and schema changes carry their contracts update (check-names, check-schema-diff)
- [ ] A reviewer agent that is not the author reviewed it (naming and schema, style, tests); every finding is fixed or answered
