# Notifications

cmux provides a notification panel for AI agents like Claude Code, Codex, and OpenCode. Notifications appear in a dedicated panel and trigger desktop notifications (via `notify-send`).

## Quick Start

```bash
# Send a notification (if cmux is available)
command -v cmux &>/dev/null && cmux notify --title "Done" --body "Task complete"
```

## Detection

Check if the `cmux` CLI is available before using it:

```bash
# Shell
if command -v cmux &>/dev/null; then
    cmux notify --title "Hello"
fi
```

```python
# Python
import shutil
import subprocess

def notify(title: str, body: str = ""):
    if shutil.which("cmux"):
        subprocess.run(["cmux", "notify", "--title", title, "--body", body])
```

## CLI Usage

```bash
# Simple notification
cmux notify --title "Build Complete"

# With subtitle and body
cmux notify --title "Claude Code" --subtitle "Permission" --body "Approval needed"

# Notify specific tab/panel
cmux notify --title "Done" --tab 0 --panel 1
```

## Integration Examples

### Claude Code Hooks

Add to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "Notification": [
      {
        "matcher": "idle_prompt",
        "hooks": [
          {
            "type": "command",
            "command": "command -v cmux &>/dev/null && cmux notify --title 'Claude Code' --body 'Waiting for input'"
          }
        ]
      },
      {
        "matcher": "permission_prompt",
        "hooks": [
          {
            "type": "command",
            "command": "command -v cmux &>/dev/null && cmux notify --title 'Claude Code' --subtitle 'Permission' --body 'Approval needed'"
          }
        ]
      }
    ]
  }
}
```

### OpenAI Codex

Add to `~/.codex/config.toml`:

```toml
notify = ["bash", "-c", "command -v cmux &>/dev/null && cmux notify --title Codex --body \"$(echo $1 | jq -r '.\"last-assistant-message\" // \"Turn complete\"' 2>/dev/null | head -c 100)\"", "--"]
```

Or create a simple script `~/.local/bin/codex-notify.sh`:

```bash
#!/bin/bash
MSG=$(echo "$1" | jq -r '."last-assistant-message" // "Turn complete"' 2>/dev/null | head -c 100)
command -v cmux &>/dev/null && cmux notify --title "Codex" --body "$MSG"
```

Then use:
```toml
notify = ["bash", "~/.local/bin/codex-notify.sh"]
```

### OpenCode Plugin

Create `.opencode/plugins/cmux-notify.js`:

```javascript
export const CmuxNotificationPlugin = async ({ $, }) => {
  const notify = async (title, body) => {
    try {
      await $`command -v cmux && cmux notify --title ${title} --body ${body}`;
    } catch {
      // cmux is not installed or not running
    }
  };

  return {
    event: async ({ event }) => {
      if (event.type === "session.idle") {
        await notify("OpenCode", "Session idle");
      }
    },
  };
};
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `CMUX_SOCKET` | Overrides the control socket path. Default: `$XDG_RUNTIME_DIR/cmux/cmux.sock` (falls back to `/run/user/$UID/cmux/cmux.sock`) |

## CLI Commands

```
cmux notify --title <text> [--subtitle <text>] [--body <text>] [--tab <id|index>] [--panel <id|index>]
cmux list-notifications
cmux clear-notification <id>
cmux ping
```

## Best Practices

1. **Always check availability first** - Use `command -v cmux` before calling
2. **Keep notifications concise** - Title should be brief, use body for details
