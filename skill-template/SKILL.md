---
name: my-linux-ssh
description: Connect to the user's Linux machine (my-linux-box) over SSH through a pure-HTTP Cloudflare Tunnel bridge (ssh-bridge.example.com). Use whenever the user asks to run shell commands, inspect files, review repositories, start builds/tests, or kick off background jobs on their Linux PC from Gemini Task mode.
---
# my-linux-ssh

Connect to the user's Linux host (`my-linux-box`) as user `llm` over OpenSSH using a pure HTTP POST/GET ProxyCommand (`scripts/http-ssh-proxy.py`) through `https://ssh-bridge.example.com`.

## When to Use

- The user asks to run commands, check status, read/edit files, or start builds/jobs on `my-linux-box` or their Linux PC.
- The user wants to offload a task from mobile or web Gemini Task mode to their local Linux environment.

## Steps

1. Check whether `$HOME/.local/bin/http-ssh-proxy.py` exists. If missing, fetch `scripts/http-ssh-proxy.py` via `get_skill_resources` for skill `my-linux-ssh`, write it to `$HOME/.local/bin/http-ssh-proxy.py`, and run `chmod +x`.
2. Check whether `$HOME/.local/share/my_linux_key` exists (`chmod 600`). Store the dedicated key at `$HOME/.local/share/my_linux_key` rather than `~/.ssh` so `~/.ssh` stays untouched. If missing locally, check the user's Google Drive folder `gemini/my-linux-ssh` for the saved key file (normalizing any markdown-escaped leading backslashes and blank lines when restoring to `$HOME/.local/share/my_linux_key`).
3. If you hit connection errors, verify the HTTP bridge is alive:
   ```bash
   curl -fsS -A 'Mozilla/5.0 (compatible; Google-Agent; +https://developers.google.com/crawling/docs/crawlers-fetchers/google-agent)' https://ssh-bridge.example.com/healthz
   ```
4. Execute remote commands via OpenSSH over the HTTP bridge:
   ```bash
   ssh \
     -i "$HOME/.local/share/my_linux_key" \
     -o IdentitiesOnly=yes \
     -o BatchMode=yes \
     -o ConnectTimeout=15 \
     -o StrictHostKeyChecking=no \
     -o UserKnownHostsFile=/dev/null \
     -o ProxyCommand="python3 $HOME/.local/bin/http-ssh-proxy.py https://ssh-bridge.example.com" \
     llm@ssh-bridge.example.com "<command>"
   ```
5. For any long-running job (builds, tests, coding agents, downloads), never block SSH in the foreground. Launch it detached on the Linux host (for example with `tmux new-session -d -s <name> "<cmd>"` or `nohup ... > /tmp/<name>.log 2>&1 &`), return immediately with the session/log name, and inspect progress later via `tmux capture-pane -pt <name>` or `tail`.

## Gotchas

- This setup uses pure HTTP POST/GET via `scripts/http-ssh-proxy.py` over HTTPS instead of direct TCP port 22 or WebSocket (`cloudflared access ssh`).
- Always set the exact `Google-Agent` User-Agent header on all outbound HTTP requests (already built into `scripts/http-ssh-proxy.py`).
- Always use the pre-generated dedicated key stored at `$HOME/.local/share/my_linux_key` (`chmod 600`) rather than modifying `~/.ssh` or running `ssh-keygen` on the client.
- If `/healthz` fails or returns 502/530, inform the user that `ssh-for-gemini` or the Cloudflare Tunnel on their Linux host appears offline.
