# Contributing to CCOSEL

## For AI Agents Working on Issues

When working on multiple issues simultaneously, **use git worktrees** to avoid file conflicts between parallel agents.

### Setting Up a Worktree

```bash
# Create a worktree for an issue
git worktree add ../CCOESL-issue-<number> -b fix/<descriptive-name>

# Example for issue #5 (Full screen toggle)
git worktree add ../CCOESL-issue-5 -b fix/fullscreen-toggle
```

### Workflow

1. **Create a worktree** for each issue/agent
2. **Navigate to the worktree directory** and make changes
3. **Commit and push** your changes
4. **Create a pull request** from your worktree
5. **Clean up** when merged:
   ```bash
   git worktree remove ../CCOESL-issue-<number>
   ```

### Benefits

- No file conflicts between parallel agents
- Each agent has its own working directory
- Easy to manage multiple branches simultaneously
- Clean separation of concerns

### Best Practices

- Use descriptive branch names related to the issue
- Work on separate areas of the codebase when possible
- Communicate if changes might overlap
- Rebase frequently to stay up to date with main
- Keep PRs small and focused
