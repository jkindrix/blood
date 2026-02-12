# Issue Triage Process

This document describes how issues are triaged and managed in the Blood project.

## Triage Workflow

### 1. Initial Triage

All new issues start with the `triage` label. Maintainers review new issues within 48 hours to:

1. **Validate**: Ensure the issue is complete and reproducible
2. **Categorize**: Apply appropriate labels
3. **Prioritize**: Assess importance and urgency
4. **Assign**: Optionally assign to a maintainer or milestone

### 2. Label System

#### Type Labels

| Label | Description |
|-------|-------------|
| `bug` | Something isn't working |
| `enhancement` | New feature or improvement |
| `documentation` | Documentation improvements |
| `question` | Questions about Blood |
| `duplicate` | Issue already exists |
| `invalid` | Not a valid issue |
| `wontfix` | Not planned to be addressed |

#### Component Labels

| Label | Description |
|-------|-------------|
| `comp:compiler` | Blood compiler (bloodc) |
| `comp:runtime` | Blood runtime |
| `comp:stdlib` | Standard library |
| `comp:lsp` | Language server |
| `comp:tooling` | Developer tools |
| `comp:docs` | Documentation |

#### Priority Labels

| Label | Description | Response Time |
|-------|-------------|---------------|
| `P0-critical` | Crash, data loss, security | Same day |
| `P1-high` | Major feature broken | Within 1 week |
| `P2-medium` | Feature partially broken | Within 1 month |
| `P3-low` | Minor issues | When available |

#### Status Labels

| Label | Description |
|-------|-------------|
| `triage` | Needs initial review |
| `needs-info` | Waiting for more information |
| `confirmed` | Bug confirmed, needs fix |
| `in-progress` | Currently being worked on |
| `blocked` | Blocked by another issue |

#### Effort Labels

| Label | Description |
|-------|-------------|
| `good-first-issue` | Good for new contributors |
| `help-wanted` | Community contributions welcome |
| `mentor-available` | A maintainer can help |

### 3. Issue Lifecycle

```
┌─────────────┐
│   Opened    │
└──────┬──────┘
       │
       v
┌─────────────┐     ┌─────────────┐
│   Triage    │────>│ Needs Info  │
└──────┬──────┘     └──────┬──────┘
       │                   │
       v                   v
┌─────────────┐     ┌─────────────┐
│  Confirmed  │<────│  Responded  │
└──────┬──────┘     └─────────────┘
       │
       v
┌─────────────┐
│ In Progress │
└──────┬──────┘
       │
       v
┌─────────────┐
│   Closed    │
└─────────────┘
```

### 4. Response Times

| Issue Type | Initial Response | Resolution Target |
|------------|------------------|-------------------|
| Security | 24 hours | ASAP |
| P0-critical | 24 hours | 1 week |
| P1-high | 48 hours | 2 weeks |
| P2-medium | 1 week | 1 month |
| P3-low | 2 weeks | When available |

### 5. Stale Issue Policy

Issues with no activity for 60 days will be marked as `stale`. After 30 more days without activity, they may be closed.

Exceptions:
- Issues with `P0-critical` or `P1-high` labels
- Issues in active milestones
- Issues with `blocked` label

### 6. Duplicate Handling

When duplicates are found:
1. Link the duplicate to the original
2. Add `duplicate` label
3. Close with comment explaining the duplicate
4. Transfer any unique information to the original

### 7. Feature Request Evaluation

Feature requests are evaluated on:

1. **Alignment**: Does it fit Blood's design philosophy?
2. **Impact**: How many users would benefit?
3. **Complexity**: How difficult is it to implement?
4. **Breaking**: Does it require breaking changes?
5. **Alternatives**: Are there existing workarounds?

Feature requests may be:
- Accepted and added to roadmap
- Deferred for future consideration
- Declined with explanation

## For Contributors

### How to Help with Triage

1. Reproduce reported bugs
2. Add missing information
3. Suggest labels
4. Link related issues
5. Test proposed fixes

### What Makes a Good Bug Report

- Clear description of expected vs actual behavior
- Minimal reproduction code
- Version and platform information
- Full error messages

### What Makes a Good Feature Request

- Clear problem statement
- Concrete use cases
- Consideration of alternatives
- Example code showing desired syntax

## For Maintainers

### Daily Triage Checklist

- [ ] Review all issues with `triage` label
- [ ] Respond to all `needs-info` issues with updates
- [ ] Check for stale issues
- [ ] Update milestone progress

### Weekly Triage Meeting

1. Review P0/P1 issues
2. Discuss blocked issues
3. Plan upcoming milestone
4. Assign issues to contributors

### Release Triage

Before each release:
1. Review all open P0/P1 issues
2. Ensure no regressions
3. Update CHANGELOG
4. Close resolved issues
