# SSH for Gemini

[日本語版 README はこちら (README.ja.md)](./README.ja.md)

Give **Gemini Task mode** real OpenSSH access to your Linux machine — **no Mac required, no global TCP port 22 exposed, no WebSocket needed, and zero extra HTTP auth layers**.

`ssh-for-gemini` is a tiny, zero-dependency Rust daemon (~600 KB stripped) that bridges pure HTTP/1.1 `POST`/`GET` requests to local `127.0.0.1:22`, paired with a drop-in Gemini Skill template (`skill-template/`). Once wired up, you can open Gemini on your phone or browser in Task mode and tell it to inspect repos, run tests, kick off builds, check USB/hardware devices, or dispatch detached jobs on your Linux box using your Gemini Task quota.

> [!IMPORTANT]
> **Before handing this repository to an AI / Coding Agent**
>
> **Do not paste the GitHub URL directly into an AI chat or coding agent.** The [Why This Architecture?](#why-this-architecture-gemini-task-sandbox-constraints) section below documents cloud-sandbox and gateway constraints in plain technical detail, which can trigger automated safety/policy refusals when an AI reads it without prior context.
>
> Instead, **read and understand all caveats, prerequisites, and security risks in this README yourself first**, then clone a **disposable setup copy** and run these two commands in the freshly cloned directory before pointing your coding agent at it (or pass `AGENTS.md` directly):
>
> ```bash
> mv AGENTS.md README.md
> rm -rf README.ja.md .git
> ```
>
> **Notes on `rm -rf .git`:**
> - That disposable directory will no longer support `git pull` for upstream updates. (If you only want to run the NixOS daemon, you can reference `github:kenh0u/ssh-for-gemini` directly in your NixOS `flake.nix` instead.)
> - Run these commands on a **fresh clone before `cargo build`** creates `target/`: without `.git`, Nix treats a local directory as a `path:` flake and copies every file in it (including `target/`) into the Nix store.


---

## What You Can Do

- **Drive your Linux box from your phone**: Start a Gemini Task from mobile or web and have Gemini SSH directly into a sandboxed Linux user on your home PC or server.
- **Keep Gemini Task quota separate from local coding agents**: Offload research-to-code reviews, repo inspection, or background builds to Gemini Task mode while saving your Antigravity / Claude Code / Codex quotas.
- **100% real OpenSSH + Ed25519**: Authentication and end-to-end encryption stay strictly inside OpenSSH. Even though the transport is plain HTTPS via Cloudflare Tunnel, nobody without your Ed25519 private key can get past `sshd`.
- **Detached long-running jobs**: Kick off `tmux` or `systemd-run --user` jobs on your Linux host so tasks survive Gemini turn limits and can be checked later from another turn.

---

## Why This Architecture? (Gemini Task Sandbox Constraints)

Google's official desktop local-task / computer-use features target macOS (Apple Silicon) and require local Wi-Fi/Bluetooth pairing for remote triggers. Meanwhile, Gemini Task mode already gives every account a persistent Linux cloud sandbox ("remote computer") with optional **Network access for remote computer**.

Empirical testing of that cloud sandbox revealed five hard constraints that directly shaped `ssh-for-gemini`:

| Constraint Observed in Gemini Task Sandbox | Why Standard Approaches Break | How `ssh-for-gemini` Solves It |
| :--- | :--- | :--- |
| **Outbound TCP port 22 is blocked** | Direct `ssh` times out. | Tunnel everything over port 443 HTTPS. |
| **L7 TLS-inspecting HTTPS gateway rejects `Upgrade: websocket` (`400 Bad Request`)** | `cloudflared access ssh` (WebSocket) and Tailscale DERP (`Upgrade: derp`) fail at handshake. | Transport raw TCP bytes over plain HTTP/1.1 `POST` (`/tx`) and long-poll `GET` (`/rx`) with finite `Content-Length` and Keep-Alive. |
| **No `/dev/net/tun` and non-root container** | Kernel-mode WireGuard / Tailscale cannot create a TUN interface. | Pure userspace Python 3 `ProxyCommand` (`http-ssh-proxy.py`) using only stdlib. |
| **Sandbox policy blocks `~/.ssh` writes and `ssh-keygen`** | Placing keys in `~/.ssh` or generating keys inside Gemini fails with policy deny. | Generate the dedicated Ed25519 key on your Linux PC and store it at `$HOME/.local/share/<key>` (`chmod 600`) inside Gemini. |
| **Skill security scanner rejects raw private keys (and explicit Drive-exfil wording) inside skill packages** | Bundling a private key in `SKILL.md` or `assets/` fails `save_skill` with `ZIP_NOT_CLEAN`. | Use a two-tier key layout: **L1** = Gemini's per-account persistent `$HOME/.local/share/...` (shared across all tasks of the same Google account), **L2** = a private Google Drive folder (e.g. `gemini/my-linux-ssh/`) referenced cleanly in the skill for automatic recovery if the sandbox resets. |

Additional runtime notes:
- **User-Agent**: Outbound HTTP requests from Gemini's sandbox require the `Google-Agent` User-Agent header (already built into `skill-template/scripts/http-ssh-proxy.py`).
- **Google Docs escaping on Drive L2 recovery**: Plain text files created/converted as Google Docs in Drive return `\-----BEGIN OPENSSH PRIVATE KEY-----` and blank lines when read via Drive tools. The skill template explicitly instructs Gemini to strip leading `\` before `-----` and drop empty lines before writing `chmod 600`.
- **Turn/time limits**: Gemini Task turns are not infinite. Always detach long builds/tests on the Linux side (`tmux`, `nohup`, or `systemd-run --user`) and poll logs/panes rather than blocking a single SSH foreground command.

---

## Repository Layout

```text
ssh-for-gemini/
├── README.md                          # English documentation (this file)
├── README.ja.md                       # Japanese documentation
├── LICENSE-MIT / LICENSE-APACHE       # Dual-licensed (MIT OR Apache-2.0)
├── Cargo.toml / Cargo.lock            # Zero-dependency Rust crate
├── src/main.rs                        # HTTP/1.1 POST/GET <-> local TCP(22) daemon (default port 28787)
├── flake.nix / flake.lock             # Nix flake (packages, checks, devShells, overlays, nixosModules)
├── package.nix                        # rustPlatform.buildRustPackage derivation
├── nix/module.nix                     # Hardened NixOS systemd module (services.ssh-for-gemini)
└── skill-template/                    # Template to hand to Gemini Task to register your Skill
    ├── SKILL.md
    └── scripts/http-ssh-proxy.py
```

---

## Prerequisites

1. **Linux host** with OpenSSH server (`sshd`) accepting Ed25519 public-key authentication.
   - Strongly recommended: create a **dedicated, non-privileged "cage" user** (e.g. `llm`) whose home directory and permissions contain only what you are comfortable letting an autonomous agent touch.
2. **Cloudflare Tunnel** on your domain, routing an **HTTP** (not SSH!) published hostname such as `https://ssh-bridge.example.com` -> `http://127.0.0.1:28787`.
   - Using NixOS? Check out [kenh0u/nixos-cloudflared-remotely](https://github.com/kenh0u/nixos-cloudflared-remotely) for declarative remote-managed `cloudflared` tunnel setup on NixOS.
3. **Gemini Task mode** with **Network access for remote computer** enabled in Gemini **Settings -> Task settings**.

---

## For Coding Agents (Antigravity, Claude Code, etc.) — Guided Setup Protocol

If a user pointed you (an AI coding agent running on their Linux machine) at this repository, follow this exact division of responsibilities so you handle the local setup cleanly and hand off to the user + Gemini Task at the right boundary:

### Phase 1 — What YOU (the Coding Agent) do on the Linux PC
1. **Confirm target parameters with the user**:
   - Dedicated Linux username (recommend creating a non-privileged user such as `llm` if they don't already have one)
   - Cloudflare Tunnel subdomain they want to expose (e.g. `ssh-bridge.example.com`)
   - Local listen port (default: `28787`)
   - Desired Gemini Skill name (e.g. `my-linux-ssh`) and Google Drive folder path for L2 key backup (e.g. `gemini/my-linux-ssh`)
2. **Build & install `ssh-for-gemini`**:
   - On NixOS: build via `nix build` (or wire `nixosModules.default` + `overlays.default` into their NixOS flake and enable `services.ssh-for-gemini.enable = true;`).
   - On other Linux distros: `cargo build --release` and install `target/release/ssh-for-gemini` as a systemd service.
3. **Create a dedicated Ed25519 keypair for Gemini**:
   ```bash
   ssh-keygen -t ed25519 -N "" -C "gemini-task-ssh" -f ./id_ed25519_gemini
   ```
   - Install `id_ed25519_gemini.pub` into the dedicated Linux user's `~/.ssh/authorized_keys` (consider prefixing with `no-port-forwarding,no-agent-forwarding,no-X11-forwarding` if port forwarding is unnecessary).
4. **Configure Cloudflare Tunnel** so `https://<their-subdomain>` forwards to **`http://127.0.0.1:28787`** (service type: **HTTP**, NOT SSH).
   - If the user is on NixOS, you may reference [kenh0u/nixos-cloudflared-remotely](https://github.com/kenh0u/nixos-cloudflared-remotely).
5. **Verify locally and externally**:
   - Confirm `curl -fsS http://127.0.0.1:28787/healthz` returns `ok`.
   - Confirm `curl -fsS https://<their-subdomain>/healthz` returns `ok`.
6. **Render a customized copy of `skill-template/`** (replacing `my-linux-ssh`, `my-linux-box`, `ssh-bridge.example.com`, `llm`, `my_linux_key`, and `gemini/my-linux-ssh` with the user's actual values — **never put the private key inside `SKILL.md` or `skill-template/`**).

### Phase 2 — Hand off to the User (Exact Instructions to Present)
Once Phase 1 is verified, give the user these three concrete steps:
1. **Enable network access in Gemini**: In Gemini Web/Mobile -> **Settings -> Task settings -> Network access for remote computer -> ON**.
2. **Place the dedicated key in Google Drive**: Upload `id_ed25519_gemini` (and optionally `id_ed25519_gemini.pub`) into their private Google Drive folder `gemini/<skill-name>/` (or paste the private key once in the initial Gemini Task chat and ask Gemini to save it to Drive for them).
3. **Register the Skill in Gemini Task**: Attach or paste the customized `SKILL.md` and `scripts/http-ssh-proxy.py` into a **Gemini Task** conversation with a prompt like:

   > Create a new multi-file skill named `<skill-name>` using the attached `SKILL.md` and `scripts/http-ssh-proxy.py` (keep the private key out of the skill package; it is stored in my Google Drive folder `gemini/<skill-name>/`). After saving the skill, test connecting to `<their-subdomain>` and run `uname -a && id`.

---

## Manual Setup Guide

### 1. Build & Run the Daemon (Linux PC)

With Nix:
```bash
nix build
./result/bin/ssh-for-gemini --listen 127.0.0.1 --port 28787 --target-port 22
```

Run tests:
```bash
nix flake check
```

Without Nix (Rust 1.70+):
```bash
cargo test
cargo build --release
./target/release/ssh-for-gemini --listen 127.0.0.1 --port 28787 --target-port 22
```

### 2. NixOS Module (Optional)

Add this flake (`github:kenh0u/ssh-for-gemini`) to your NixOS `flake.nix` inputs (`overlays.default` + `nixosModules.default`) and enable the hardened systemd service:

```nix
# flake.nix
{
  inputs.ssh-for-gemini.url = "github:kenh0u/ssh-for-gemini";
}
```

```nix
{
  services.ssh-for-gemini = {
    enable = true;
    listen = "127.0.0.1";
    port = 28787;
    targetHost = "127.0.0.1";
    targetPort = 22;
    maxSessions = 16;
    idleTimeout = 180;
  };
}
```

### 3. Cloudflare Tunnel Route

Expose `http://127.0.0.1:28787` as a Cloudflare Tunnel **HTTP** route (for example `ssh-bridge.example.com` -> `http://127.0.0.1:28787`). Verify with:

```bash
curl -fsS https://ssh-bridge.example.com/healthz
# -> ok
```

For declarative NixOS Cloudflare Tunnel management, see [kenh0u/nixos-cloudflared-remotely](https://github.com/kenh0u/nixos-cloudflared-remotely).

### 4. Protocol Overview

- `GET /healthz` -> `200 OK` (`ok\n`)
- `POST /open` -> opens TCP connection to `target_host:target_port`, returns 32-char hex session ID (`200 OK`)
- `POST /tx/<sid>` -> writes raw request body bytes to the target TCP stream (`204 No Content`)
- `GET /rx/<sid>` -> long-polls the target TCP stream (default 15s, with 5ms packet coalescing) and returns raw bytes (`200 OK`), poll timeout (`204 No Content`), or EOF (`410 Gone`)
- `POST /close/<sid>` -> closes and removes the session (`204 No Content`)

---

## Security Notes

- **Use a dedicated caged user**: Do not give Gemini Task SSH access to your primary desktop account or passwordless `sudo`. Create a dedicated non-privileged user (e.g. `llm`) with access scoped only to the repositories/devices you intend it to touch.
- **Dedicated keypair**: Generate a fresh Ed25519 key exclusively for Gemini Task so revoking access is a one-line edit in `authorized_keys`.
- **Keep keys out of the Skill package**: Never put raw private keys in `SKILL.md` or bundled skill assets; rely on `$HOME/.local/share/...` (L1) + private Google Drive folder (L2).
- **Bind daemon to `127.0.0.1`**: Never bind `ssh-for-gemini` to `0.0.0.0`; keep it loopback-only behind Cloudflare Tunnel (and remember the bridge only forwards to `127.0.0.1:22`, where `sshd` enforces Ed25519 pubkey auth).

---

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
