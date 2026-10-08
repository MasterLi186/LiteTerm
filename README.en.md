# LiteTerm

**Language:** [简体中文](README.md) | English

> **Development status: Native only.** The sole active development and runtime target is `native-prototype/` (Rust + winit + wgpu + egui), launched with `./run-native.sh`. React/Tauri and legacy GTK code are kept for reference and are not modified unless explicitly requested. See [Native-only development policy](docs/NATIVE_ONLY.md).

A lightweight, cross-platform SSH client and open-source alternative to FinalShell, Tabby, and XShell.

The current version is built with **Rust + winit + wgpu + egui**.

## Features

### Terminal

- **Multiple tabs** — Local terminal, SSH, and serial; reorder tabs by dragging and rename them from the context menu
- **Split panes** — Horizontal and vertical nested splits with draggable sizing
- **Multiple shells** — bash, zsh, fish, dash, and other installed system shells
- **Serial terminal** — Detect USB/ACM devices and choose the baud rate
- **Terminal search** — Search terminal output with Ctrl+Shift+F
- **Six themes** — Dark default, Monokai, Solarized Dark, Dracula, One Dark, and Light
- **Copy on select** — Selection copies automatically; middle-click pastes
- **Session recording** — Record terminal activity as asciicast v2 and replay it with speed, pause, and seek controls
- **Log recording** — Start recording from the context menu, choose a save path, and strip ANSI escape sequences automatically
- **Custom shortcuts** — Configure nine keyboard shortcuts

### SSH

- **Connection manager** — Organize saved connections into groups; passwords are stored in a locally encrypted credential file
- **ProxyJump** — Jump-host support through the system `ssh -J` command
- **Port forwarding** — Run multiple local tunnels at once
- **Automatic reconnect** — Exponential backoff with up to five attempts
- **Persistent sessions** — Restore connections after restarting the app
- **Batch commands** — Send the same command to selected SSH tabs
- **ZMODEM** — Transfer files through bastions and jump hosts with `sz`/`rz`

### System monitoring

- **CPU** — Model, core count, and live utilization
- **Memory and swap** — Used and total amounts with percentages
- **Process list** — Sort by memory, CPU, or command; show RSS and CPU utilization
- **Process manager** — Sort all processes and inspect command lines, environment variables, and working directories
- **Network traffic** — Select network interfaces, view upload/download rates, and see live charts
- **Disk usage** — View mount points, available/total space, and percentages
- **Local and remote monitoring** — Show local system data when no SSH session is active

### File manager

- **Dual-pane layout** — Local files on the left and remote SFTP files on the right
- **Uploads and downloads** — Transfer files from the context menu with progress indicators
- **Directory navigation** — Path bar, sorting, and hidden-file toggle
- **Favorite commands** — Save common commands and send them to the terminal with one click

### Configuration

- **Import and export** — Back up and restore `connections.toml`
- **SSH key manager** — View or generate Ed25519, RSA, and ECDSA keys, and copy public keys

## Installation

### Download a release

Go to [Releases](https://github.com/MasterLi186/LiteTerm/releases) to download a published asset. The table lists formats supported by the project overall; it does not mean the local build script creates every format.

| Platform | Formats |
|---|---|
| Linux | `.deb`, `.rpm`, `.AppImage` |
| macOS | `.dmg` |
| Windows | `.msi`, `.exe` |

### Build from source

```bash
# System dependencies (Ubuntu/Debian)
sudo apt install -y build-essential pkg-config libudev-dev

# Get the source
git clone https://github.com/MasterLi186/LiteTerm.git
cd LiteTerm

# Build the Native debug version (does not run automated tests)
cd native-prototype
cargo build --locked --bin liteterm-native
cd ..

# Launch from the repository root (does not create an installer)
./run-native.sh
```

The equivalent build command from the repository root is `cargo build --manifest-path native-prototype/Cargo.toml --locked --bin liteterm-native`. If the debug binary is missing or stale, `./run-native.sh` runs this Native build directly and does not create a `.deb` package.

### Build a local Debian package

Run this from the repository root:

```bash
./native-prototype/build.sh
```

This no-argument packaging entry point builds the locked release binary and creates a Debian `.deb`. It does not run automated tests or install the package with `sudo`. The output is written to `native-prototype/dist/installers/`; the filename includes the version and Debian architecture:

```text
native-prototype/dist/installers/LiteTerm-Native_<version>_<architecture>.deb
```

Copy the install command printed by `build.sh`. For example, the current version on a common amd64 system may look like:

```bash
sudo apt install ./native-prototype/dist/installers/LiteTerm-Native_0.1.15_amd64.deb
```

The script prints the actual package path and a ready-to-copy install command when it finishes.

## Technology stack

| Layer | Technology |
|---|---|
| Window and UI | winit + egui |
| Rendering | wgpu + cosmic-text |
| Core | Rust |
| SSH/SFTP | ssh2 (libssh2) |
| Local terminal | portable-pty |
| Serial | serialport |
| Key storage | Locally encrypted file using OpenSSL AES-256-GCM |

## Screenshots

Coming soon.

## License

MIT
