# Chainlink Issue Tracker - VS Code Extension

A simple, lean issue tracker for AI-assisted development, integrated directly into VS Code.

## Features

- **Session Management**: Start/end work sessions with handoff notes for context preservation
- **Issue Tracking**: Create, update, and manage issues without leaving your editor
- **Daemon Auto-Start**: Background daemon keeps session state fresh
- **Cross-Platform**: Works on Windows, Linux, and macOS
- **Agent-Agnostic**: Context provider script works with any AI coding assistant

## Installation

1. Install from the VS Code Extensions Marketplace (search "Chainlink Issue Tracker")
2. Open a project folder
3. Run `Chainlink: Initialize Project` from the command palette

## Commands

| Command | Description |
|---------|-------------|
| `Chainlink: Initialize Project` | Initialize chainlink in current workspace |
| `Chainlink: Start Session` | Start a new work session |
| `Chainlink: End Session` | End session with optional handoff notes |
| `Chainlink: Session Status` | Show current session info |
| `Chainlink: Start Daemon` | Manually start the background daemon |
| `Chainlink: Stop Daemon` | Stop the background daemon |
| `Chainlink: Daemon Status` | Check if daemon is running |
| `Chainlink: List Issues` | Show all open issues |
| `Chainlink: Create Issue` | Create a new issue |
| `Chainlink: Show Issue Details` | View details of a specific issue |

## Configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `chainlink.binaryPath` | `""` | Override path to chainlink binary (for development) |
| `chainlink.autoStartDaemon` | `true` | Auto-start daemon when .chainlink project detected |
| `chainlink.showOutputChannel` | `false` | Show output channel for daemon logs |

## Development

### Building the Extension

```bash
# Install dependencies
cd vscode-extension
npm install

# Compile TypeScript
npm run compile

# Build binaries for all platforms
npm run build:binaries

# Package the extension
npm run package
```

### Building Binaries

The extension bundles platform-specific binaries. To build them:

```bash
# Build all platforms (Windows native, Linux via WSL)
node scripts/build-binaries.js

# Build specific platform
node scripts/build-binaries.js --platform windows
node scripts/build-binaries.js --platform linux
```

**Requirements:**
- Windows: Visual Studio Build Tools with Rust
- Linux: WSL with Fedora 42 (or another distro with Rust installed)
- macOS: Xcode Command Line Tools with Rust

### Testing Locally

1. Open the `vscode-extension` folder in VS Code
2. Press F5 to launch Extension Development Host
3. Set `chainlink.binaryPath` to your local debug binary path

## Architecture

```
vscode-extension/
├── src/
│   ├── extension.ts    # Extension entry point, command registration
│   ├── daemon.ts       # Daemon lifecycle management
│   └── platform.ts     # Platform detection, binary resolution
├── bin/                # Platform binaries (populated by build script)
│   ├── chainlink-win.exe
│   ├── chainlink-linux
│   └── chainlink-darwin
├── scripts/
│   └── build-binaries.js  # Cross-compilation orchestration
└── package.json
```

## Daemon Behavior

The daemon runs as a background process that:
- Auto-flushes session state every 30 seconds
- Self-terminates when VS Code closes (zombie prevention via stdin monitoring)
- Writes logs to `.chainlink/daemon.log`

## Using with Any AI Agent

Chainlink includes a context provider script that works with **any** AI coding assistant, not just Claude Code.

### Context Provider

After running `Chainlink: Initialize Project`, you'll have a context provider at:
```
.chainlink/integrations/context-provider.py
```

This script generates intelligent context including:
- Current session state and handoff notes
- Open/ready issues
- Project structure
- Language-specific coding rules

### Shell Aliases

Add to your `~/.bashrc`, `~/.zshrc`, or PowerShell profile:

**Bash/Zsh:**
```bash
# Copy chainlink context to clipboard
chainlink-ctx() {
    python .chainlink/integrations/context-provider.py --clipboard
}

# Aider with chainlink context
aider-cl() {
    python .chainlink/integrations/context-provider.py --format md > /tmp/cl-ctx.md
    aider --read /tmp/cl-ctx.md "$@"
}
```

**PowerShell:**
```powershell
function chainlink-ctx {
    python .chainlink\integrations\context-provider.py | Set-Clipboard
}
```

### Usage Examples

```bash
# Full context (XML format, best for LLMs)
python .chainlink/integrations/context-provider.py

# Markdown format (human readable)
python .chainlink/integrations/context-provider.py --format md

# Just coding rules
python .chainlink/integrations/context-provider.py --rules

# Copy to clipboard for web UIs
python .chainlink/integrations/context-provider.py --clipboard

# Generate .cursorrules for Cursor
python .chainlink/integrations/context-provider.py --format md --rules > .cursorrules
```

### Agent-Specific Integration

| Agent | Method |
|-------|--------|
| **Cursor** | `python context-provider.py --format md --rules > .cursorrules` |
| **Aider** | `aider --read context.md` (generate context.md first) |
| **Continue.dev** | Add exec context provider in `.continue/config.json` |
| **Web UIs** | `--clipboard` then paste as first message |
| **Claude Code** | Built-in hooks, no setup needed |

### What Gets Injected

```xml
<chainlink-session>
Session #5 active
Working on: #12 Fix authentication bug
</chainlink-session>

<chainlink-issues>
Ready issues (unblocked):
  #12   high     Fix authentication bug
</chainlink-issues>

<coding-rules>
### Rust Best Practices
- Use `?` operator over `.unwrap()`
...
</coding-rules>
```

For full documentation, see the [main README](https://github.com/dollspace-gay/chainlink#using-chainlink-with-any-ai-agent).

## License

MIT
