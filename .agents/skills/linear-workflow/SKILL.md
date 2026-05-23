---
name: linear-workflow
description: Linear issue workflow automation and best practices. Use when working with Linear issues — automatically moves issues through states (Backlog → In Progress → Needs Review), tags with "agent", and follows Linear conventions for issue management, comments, and project organization.
version: 1.0.0
author: automatic
tags:
  - linear
  - project-management
  - workflow
  - automation
---

# Linear Workflow

Standardized workflow patterns for managing Linear issues with AI agents.

## Core Principles

1. **Always transition states** — Never leave an issue in the wrong state
2. **Tag agent work** — All agent-touched issues get the "agent" label
3. **Communicate progress** — Use comments to track decisions and blockers
4. **Link related work** — Connect issues, PRs, and commits

## Standard Workflow

### When Starting Work on an Issue

1. **Read the issue** to understand context, requirements, and acceptance criteria
2. **Check current state** — if not already "In Progress", transition it
3. **Add the "agent" label** to mark agent involvement
4. **Add a comment** noting what you're starting and your approach (optional for trivial issues)

Example:
```
Starting work on this issue.

Plan:
- Update API endpoint to accept new parameter
- Add validation for email format
- Update tests

Estimated completion: 20 minutes
```

### During Work

- **Update comments** if you hit blockers, change approach, or discover new requirements
- **Link related issues** if you discover dependencies
- **Ask questions** in comments rather than making assumptions

### When Completing Work

1. **Transition to "Needs Review"** (or equivalent review state)
2. **Add a completion comment** summarizing what was done
3. **Link the PR** if code was generated
4. **Note any follow-up items** discovered during implementation

Example:
```
✅ Implementation complete

Changes:
- Added email validation to User.create()
- Updated API endpoint POST /users to accept optional email field
- Added 3 new test cases covering valid/invalid/missing email

PR: #123

Follow-up:
- Consider adding email verification flow (created issue #456)
```

## State Transitions

| From State | To State | When |
|------------|----------|------|
| Backlog, Todo, Ready | In Progress | Starting work |
| In Progress | Needs Review | Work complete, needs human review |
| In Progress | Blocked | Cannot proceed without external input |
| Needs Review | Done | After human approval (human action only) |
| Needs Review | In Progress | Review requested changes |

**Never transition directly to "Done"** — always go through review first.

## Labeling Strategy

### Required Labels

- **agent** — Applied to all issues touched by an agent

### Suggested Labels

- **needs-testing** — Implementation done but needs test coverage
- **breaking-change** — Changes that affect existing APIs or behavior
- **documentation** — Requires docs update
- **security** — Security-related changes
- **performance** — Performance optimization or concern

## Comments Best Practices

### Good Comment Patterns

✅ **Progress updates**
```
Updated User model to include email validation.
Next: API endpoint changes.
```

✅ **Blocker reporting**
```
⚠️ Blocked: Need clarification on password reset flow.
Should we send email immediately or queue it?
```

✅ **Decision documentation**
```
Decision: Using bcrypt for password hashing (industry standard, OWASP recommended).
Rejected argon2 due to deployment environment constraints.
```

### Avoid

❌ Verbose play-by-play of every minor action  
❌ Restating what's already in the code/PR  
❌ Uncertainty without asking for clarification ("I think maybe...")

## Working with Projects

- **Always assign issues to the active project/cycle** if one exists
- **Update project status** if the issue affects milestones
- **Check project dependencies** before closing issues

## Working with Teams

- **Respect team ownership** — don't reassign issues across teams without confirmation
- **Follow team conventions** — check team-specific labels, workflows, or templates
- **Tag team members** when their input is needed

## Integration with Git

When creating commits or PRs related to Linear issues:

- **Reference issue IDs** in commit messages: `feat: add email validation (LIN-123)`
- **Link PRs** in Linear comments
- **Use Linear's Git integration** — if configured, commits with issue IDs auto-link

## Error Handling

If you cannot complete an issue:

1. **Document the blocker** in a comment
2. **Transition to "Blocked"** state (if available)
3. **Tag relevant people** who can unblock
4. **Create follow-up issues** for discovered work
5. **Never leave the issue in "In Progress"** if you've stopped working on it

## Tools Usage

### Starting Work
```javascript
// 1. Get the issue
const issue = await linear_get_issue({ issueId: "LIN-123" });

// 2. Check if "agent" label exists, create if needed
const labels = await linear_list_labels({ teamId: issue.team.id });
const agentLabel = labels.find(l => l.name === "agent");
if (!agentLabel) {
  await linear_create_label({
    teamId: issue.team.id,
    name: "agent",
    color: "#5E6AD2" // Linear purple
  });
}

// 3. Transition to In Progress + add label
await linear_update_issue({
  issueId: "LIN-123",
  stateId: "<in-progress-state-id>",
  labelIds: [...issue.labelIds, agentLabel.id]
});

// 4. Add starting comment (optional)
await linear_create_comment({
  issueId: "LIN-123",
  body: "Starting work on this issue.\n\nPlan:\n- ..."
});
```

### Completing Work
```javascript
// 1. Transition to Needs Review
await linear_update_issue({
  issueId: "LIN-123",
  stateId: "<needs-review-state-id>"
});

// 2. Add completion comment
await linear_create_comment({
  issueId: "LIN-123",
  body: "✅ Implementation complete\n\nChanges:\n- ...\n\nPR: #123"
});
```

## Quick Reference

**Every time you work on a Linear issue:**

1. ✅ Read the issue first
2. ✅ Transition to "In Progress"
3. ✅ Add "agent" label
4. ✅ Do the work
5. ✅ Transition to "Needs Review"
6. ✅ Add completion comment with summary

**Never:**

- ❌ Leave issues in wrong state
- ❌ Skip the review state
- ❌ Make assumptions without documenting them
- ❌ Close issues without human review
