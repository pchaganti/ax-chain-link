# Chainlink

A simple, lean issue tracker CLI designed for AI-assisted development. Track tasks across sessions with context preservation.

## Features

- **Local-first**: All data stored in SQLite (`.chainlink/issues.db`)
- **Session management**: Preserve context across Claude/AI sessions with handoff notes
- **Subissues**: Break large tasks into smaller, trackable pieces
- **Dependencies**: Track blocking relationships between issues
- **Labels & priorities**: Organize issues with labels and priority levels
- **Time tracking**: Start/stop timers to track time spent on issues
- **Smart recommendations**: `chainlink next` suggests what to work on based on priority and progress
- **Tree view**: Visualize issue hierarchy with `chainlink tree`
- **Claude Code hooks**: Behavioral guardrails that inject best practices into AI sessions
- **No sync complexity**: No git hooks, no auto-push, just simple local storage

## Installation

```bash
# Build from source
cargo build --release

# The binary will be at target/release/chainlink
```

## Quick Start

```bash
# Initialize in any project
chainlink init

# Start a session when you begin work
chainlink session start

# Create issues
chainlink create "Fix login bug" -p high
chainlink create "Add dark mode" -d "Support light/dark theme toggle"

# Set what you're working on
chainlink session work 1

# End session with handoff notes
chainlink session end --notes "Fixed auth bug, dark mode is next"
```

## Commands

### Issue Management

| Command | Description |
|---------|-------------|
| `chainlink create <title>` | Create a new issue |
| `chainlink create <title> -p high` | Create with priority (low/medium/high/critical) |
| `chainlink create <title> -d "desc"` | Create with description |
| `chainlink subissue <parent_id> <title>` | Create a subissue under a parent |
| `chainlink subissue <parent_id> <title> -p high` | Subissue with priority |
| `chainlink list` | List open issues |
| `chainlink list -s all` | List all issues |
| `chainlink list -s closed` | List closed issues |
| `chainlink list -l bug` | Filter by label |
| `chainlink list -p high` | Filter by priority |
| `chainlink show <id>` | Show issue details |
| `chainlink update <id> --title "New"` | Update title |
| `chainlink update <id> -d "desc"` | Update description |
| `chainlink update <id> -p critical` | Update priority |
| `chainlink close <id>` | Close an issue |
| `chainlink reopen <id>` | Reopen a closed issue |
| `chainlink delete <id>` | Delete an issue (with confirmation) |
| `chainlink delete <id> -f` | Delete without confirmation |

### Comments & Labels

| Command | Description |
|---------|-------------|
| `chainlink comment <id> "text"` | Add a comment to an issue |
| `chainlink label <id> <label>` | Add a label to an issue |
| `chainlink unlabel <id> <label>` | Remove a label from an issue |

### Dependencies

| Command | Description |
|---------|-------------|
| `chainlink block <id> <blocker_id>` | Mark issue as blocked by another |
| `chainlink unblock <id> <blocker_id>` | Remove blocking relationship |
| `chainlink blocked` | List all blocked issues |
| `chainlink ready` | List issues ready to work on (no blockers) |

### Smart Navigation

| Command | Description |
|---------|-------------|
| `chainlink next` | Recommend the next issue to work on (by priority/progress) |
| `chainlink tree` | Show all issues in a tree hierarchy |
| `chainlink tree -s open` | Show only open issues in tree view |

### Time Tracking

| Command | Description |
|---------|-------------|
| `chainlink start <id>` | Start a timer for an issue |
| `chainlink stop` | Stop the current timer |
| `chainlink timer` | Show current timer status |

### Session Management

Sessions preserve context across AI assistant restarts.

| Command | Description |
|---------|-------------|
| `chainlink session start` | Start a session, shows previous handoff notes |
| `chainlink session work <id>` | Set the issue you're currently working on |
| `chainlink session status` | Show current session info |
| `chainlink session end` | End the current session |
| `chainlink session end --notes "..."` | End with handoff notes for next session |

### Daemon (Optional)

The daemon auto-flushes session state every 30 seconds.

| Command | Description |
|---------|-------------|
| `chainlink daemon start` | Start background daemon |
| `chainlink daemon status` | Check if daemon is running |
| `chainlink daemon stop` | Stop the daemon |

## Workflow Example

```bash
$ chainlink session start
Previous session ended: 2024-01-15 09:00
Handoff notes:
  Working on auth bug. Found issue in token refresh.

Session #5 started.

$ chainlink session work 1
Now working on: #1 Fix login bug

$ chainlink comment 1 "Fixed the token refresh issue"
Added comment to issue #1

$ chainlink close 1
Closed issue #1

$ chainlink ready
Ready issues (no blockers):
  #2    medium   Add dark mode

$ chainlink session end --notes "Closed auth bug. Dark mode is next."
Session #5 ended.
Handoff notes saved.
```

## Storage

All data is stored locally in `.chainlink/issues.db` (SQLite). No external services, no network requests.

## Claude Code Hooks

Chainlink includes behavioral hooks for [Claude Code](https://claude.com/claude-code) that inject best practice reminders into AI sessions. These hooks help ensure Claude follows coding standards without requiring manual prompting.

### What the Hooks Do

The hooks are located in `.claude/hooks/` and configured in `.claude/settings.json`:

| Hook | Trigger | Purpose |
|------|---------|---------|
| `prompt-guard.py` | Every prompt | Injects language-specific best practices, reminds about error handling, security, and no stubs/dead code |
| `post-edit-check.py` | After file edits | Reminds to verify changes compile and follow project patterns |
| `session-start.py` | Session start/resume | Loads chainlink context and previous session handoff notes |

### Behavioral Guardrails

The hooks enforce these principles:

1. **No Stubs**: Implement complete, working code - no placeholder functions or TODO comments
2. **No Dead Code**: Identify incomplete features and complete them, or remove truly dead code
3. **Full Features**: Implement complete features as requested, don't stop partway
4. **Error Handling**: Proper error handling everywhere, no panics on bad input
5. **Security**: Validate input, use parameterized queries, no command injection

### Large Task Management

When code will exceed 500 lines, the hooks guide Claude to:
1. Create a parent issue for the feature
2. Break it into subissues for trackable components
3. Inform the user about the multi-part implementation
4. Work on one subissue at a time

### Language Detection

The hooks auto-detect the project language(s) and inject relevant best practices:
- **Rust**: Use `?` operator, `clippy`, parameterized SQL, avoid `.unwrap()`
- **Python**: Type hints, proper exceptions, `pathlib`, context managers
- **JavaScript/TypeScript**: `const`/`let`, async/await, strict mode, input validation
- **Go**: Check errors, use `context.Context`, `defer` for cleanup

### Installing Hooks in Other Projects

Copy the `.claude/` directory to any project to enable the hooks:

```bash
cp -r /path/to/chainlink/.claude /your/project/
```

## Development

```bash
# Run tests
cargo test

# Run with clippy
cargo clippy -- -D warnings

# Format code
cargo fmt
```

## License

MIT
