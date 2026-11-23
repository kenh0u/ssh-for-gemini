# SSH for Gemini — Agent Setup & Architecture Guide

`ssh-for-gemini` is a lightweight, zero-dependency Rust daemon (~600 KB stripped) that bridges pure HTTP/1.1 `POST`/`GET` requests to a local OpenSSH server (`127.0.0.1:22`), paired with a Gemini Skill template (`skill-template/`). Once configured over a user-owned Cloudflare Tunnel HTTPS endpoint, **Gemini Task mode** can inspect repositories, run tests, kick off builds, check hardware/USB devices, or dispatch detached jobs on the user's Linux machine.

---

## Architecture Overview

- **Pure HTTP/1.1 Transport**: Uses standard HTTP/1.1 `POST` (`/tx/<sid>`) and long-poll `GET` (`/rx/<sid>`) with finite `Content-Length` and Keep-Alive over HTTPS, paired with a pure-stdlib Python 3 `ProxyCommand` (`skill-template/scripts/http-ssh-proxy.py`). No raw TCP port 22 exposure, no WebSocket upgrades, and no kernel TUN interface are required.
- **100% OpenSSH + Ed25519 Authentication**: End-to-end encryption and authentication stay strictly inside OpenSSH (`127.0.0.1:22`). Only holders of the dedicated Ed25519 private key can authenticate with `sshd`.
- **Key Placement & Recovery**:
  - **L1 (Local client cache)**: Store the dedicated private key at `$HOME/.local/share/<key>` (`chmod 600`) so `~/.ssh` remains untouched on the Gemini Task client.
  - **L2 (Optional Google Drive backup)**: Store a copy of the dedicated key in a private Google Drive folder (e.g. `gemini/my-linux-ssh/`) so Gemini Task can restore `$HOME/.local/share/<key>` if the client environment resets. Note: if a plain-text key in Drive is read as a Google Doc, strip any leading `\` before `-----` and drop blank lines before writing `chmod 600`.
- **HTTP User-Agent**: Outbound HTTP requests from Gemini Task use the `Google-Agent` User-Agent header (already built into `skill-template/scripts/http-ssh-proxy.py`).
- **Detached Long-Running Jobs**: Detach heavy builds/tests on the Linux host (`tmux`, `nohup`, or `systemd-run --user`) and poll logs/panes across turns rather than blocking a single foreground SSH command.

---

## Repository Layout

```text
ssh-for-gemini/
├── AGENTS.md                          # Agent setup guide (this file)
├── LICENSE-MIT / LICENSE-APACHE       # Dual-licensed (MIT OR Apache-2.0)
├── Cargo.toml / Cargo.lock            # Zero-dependency Rust crate
├── src/main.rs                        # HTTP/1.1 POST/GET <-> local TCP(22) daemon (default port 28787)
├── flake.nix / flake.lock             # Nix flake (packages, checks, devShells, overlays, nixosModules)
├── package.nix                        # rustPlatform.buildRustPackage derivation
├── nix/module.nix                     # Hardened NixOS systemd module (services.ssh-for-gemini)
└── skill-template/                    # Template to register the Gemini Skill
    ├── SKILL.md
    └── scripts/http-ssh-proxy.py
```

---

## Prerequisites

1. **Linux host** with OpenSSH server (`sshd`) accepting Ed25519 public-key authentication.
   - Strongly recommended: create a **dedicated, non-privileged user** (e.g. `llm`) whose home directory and permissions are scoped strictly to what the agent should access.
2. **Cloudflare Tunnel** on the user's domain, routing an **HTTP** (not SSH) published hostname such as `https://ssh-bridge.example.com` -> `http://127.0.0.1:28787`.
   - For declarative NixOS Cloudflare Tunnel management, see [kenh0u/nixos-cloudflared-remotely](https://github.com/kenh0u/nixos-cloudflared-remotely).
3. **Gemini Task mode** with **Network access for remote computer** enabled in Gemini **Settings -> Task settings**.

---

## Guided Setup Protocol (For Coding Agents)

If a user pointed you at this repository to set it up on their Linux machine, follow this division of responsibilities:

### Phase 1 — What YOU (the Coding Agent) do on the Linux PC
1. **Confirm target parameters with the user**:
   - Dedicated Linux username (recommend creating a non-privileged user such as `llm` if they do not already have one)
   - Cloudflare Tunnel subdomain they want to expose (e.g. `ssh-bridge.example.com`)
   - Local listen port (default: `28787`)
   - Desired Gemini Skill name (e.g. `my-linux-ssh`) and Google Drive folder path for L2 key backup (e.g. `gemini/my-linux-ssh`)
2. **Build & install `ssh-for-gemini`**:
   - On NixOS: build via `nix build`, or wire `github:kenh0u/ssh-for-gemini` (`nixosModules.default` + `overlays.default`) into their NixOS `flake.nix` and enable `services.ssh-for-gemini.enable = true;`.
   - On other Linux distros: `cargo build --release` and install `target/release/ssh-for-gemini` as a systemd service.
3. **Create a dedicated Ed25519 keypair for Gemini**:
   ```bash
   ssh-keygen -t ed25519 -N "" -C "gemini-task-ssh" -f ./id_ed25519_gemini
   ```
   - Install `id_ed25519_gemini.pub` into the dedicated Linux user's `~/.ssh/authorized_keys` (consider prefixing with `no-port-forwarding,no-agent-forwarding,no-X11-forwarding` if port forwarding is unnecessary).
4. **Configure Cloudflare Tunnel** so `https://<their-subdomain>` forwards to **`http://127.0.0.1:28787`** (service type: **HTTP**, NOT SSH).
5. **Verify locally and externally**:
   - Confirm `curl -fsS http://127.0.0.1:28787/healthz` returns `ok`.
   - Confirm `curl -fsS https://<their-subdomain>/healthz` returns `ok`.
6. **Render a customized copy of `skill-template/`** (replacing `my-linux-ssh`, `my-linux-box`, `ssh-bridge.example.com`, `llm`, `my_linux_key`, and `gemini/my-linux-ssh` with the user's actual values — **never embed the private key inside `SKILL.md` or `skill-template/`**).

### Phase 2 — Hand off to the User (Exact Instructions to Present)
Once Phase 1 is verified, give the user these three concrete steps:
1. **Enable network access in Gemini**: In Gemini Web/Mobile -> **Settings -> Task settings -> Network access for remote computer -> ON**.
2. **Place the dedicated key in Google Drive**: Upload `id_ed25519_gemini` (and optionally `id_ed25519_gemini.pub`) into their private Google Drive folder `gemini/<skill-name>/` (or paste it once in the initial Gemini Task chat and ask Gemini to save it to Drive).
3. **Register the Skill in Gemini Task**: Attach or paste the customized `SKILL.md` and `scripts/http-ssh-proxy.py` into a **Gemini Task** conversation with a prompt like:

   > Create a new multi-file skill named `<skill-name>` using the attached `SKILL.md` and `scripts/http-ssh-proxy.py` (the private key is stored separately in my Google Drive folder `gemini/<skill-name>/`). After saving the skill, test connecting to `<their-subdomain>` and run `uname -a && id`.

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

Reference this repository directly in your NixOS `flake.nix` and enable the hardened systemd service:

```nix
# flake.nix
{
  inputs.ssh-for-gemini.url = "github:kenh0u/ssh-for-gemini";
  # Add ssh-for-gemini.overlays.default to nixpkgs.overlays
  # and ssh-for-gemini.nixosModules.default to your NixOS modules.
}
```

```nix
# configuration.nix
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

### 4. Protocol Overview

- `GET /healthz` -> `200 OK` (`ok\n`)
- `POST /open` -> opens TCP connection to `target_host:target_port`, returns 32-char hex session ID (`200 OK`)
- `POST /tx/<sid>` -> writes raw request body bytes to the target TCP stream (`204 No Content`)
- `GET /rx/<sid>` -> long-polls the target TCP stream (default 15s, with 5ms packet coalescing) and returns raw bytes (`200 OK`), poll timeout (`204 No Content`), or EOF (`410 Gone`)
- `POST /close/<sid>` -> closes and removes the session (`204 No Content`)

---

## Security Best Practices

- **Use a dedicated non-privileged user**: Do not grant SSH access to a primary desktop account or passwordless `sudo`. Create a dedicated user (e.g. `llm`) scoped only to intended repositories and devices.
- **Dedicated keypair**: Generate a fresh Ed25519 key exclusively for Gemini Task so revoking access is a one-line edit in `authorized_keys`.
- **Keep keys out of the Skill package**: Never include raw private keys in `SKILL.md` or bundled skill assets; store them in `$HOME/.local/share/...` and/or a private Google Drive folder.
- **Bind daemon to `127.0.0.1`**: Keep `ssh-for-gemini` loopback-only behind Cloudflare Tunnel, forwarding only to local `127.0.0.1:22`.

---

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
