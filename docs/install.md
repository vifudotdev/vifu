# Install Vifu

## Download A Release

Download the archive for your platform from the
[latest release](https://github.com/vifudotdev/vifu/releases/latest).

- macOS and Linux archives use `.tar.gz`.
- Windows archives use `.zip`.

Extract the archive. Then start Vifu:

```bash
./vifu
```

In an interactive terminal, Vifu opens the Runtime TUI. Press `B` to open the
Dashboard. The default address is:

```text
http://127.0.0.1:6790
```

On Windows:

```powershell
.\vifu.exe
```

## Install a mobile starter

Use the release applications to verify an embedded mobile agent before opening
the source projects:

- Android: install
  [vifu-android-starter.apk](https://github.com/vifudotdev/vifu/releases/download/android-starter-v0.1.1/vifu-android-starter.apk).
  Install the baseline APK beside it to compare both backends on one device.
- iPhone and iPad: install the Vifu iOS Starter if a beta has been shared with
  your TestFlight testing group, or use the advanced Xcode build path.

Both Starters can download the verified Qwen2.5 0.5B model, run chat locally,
pair with the Vifu TUI, and upload performance traces to the developer
computer. Use the [examples guide](../examples/README.md) to select a platform.
Each platform guide contains pairing and source-build instructions.

## Build From Source

Use Cargo when you want to build from source.

### Requirements

- Rust from `rust-toolchain.toml`
- CMake, a C/C++ compiler, and libclang
- Bun and Node.js for the embedded Console bundle

The default Vifu build includes llama.cpp and Local Whisper. These Providers
require the native build tools. The Rust
[bindgen requirements](https://rust-lang.github.io/rust-bindgen/requirements.html)
explain why `libclang` is required and list the platform package names.

### Install Native Build Dependencies

Ubuntu and Debian:

```bash
sudo apt-get update
sudo apt-get install -y build-essential cmake clang libclang-dev
```

Fedora:

```bash
sudo dnf install -y gcc-c++ cmake clang clang-devel
```

Arch Linux:

```bash
sudo pacman -S --needed base-devel cmake clang
```

macOS ([Apple Command Line Tools installation](https://developer.apple.com/documentation/xcode/installing-the-command-line-tools)):

```bash
xcode-select --install
brew install cmake llvm
```

If Homebrew LLVM is installed but bindgen cannot find `libclang`, set its
library directory for the current shell:

```bash
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
```

On Windows, install
[**Desktop development with C++**](https://learn.microsoft.com/en-us/cpp/overview/acquire-msvc)
from Visual Studio Build Tools and include its CMake tools. Then install LLVM:

```powershell
winget install LLVM.LLVM
```

Open a new terminal after installation. If bindgen still cannot find LLVM, set
`LIBCLANG_PATH` to the directory containing `libclang.dll`:

```powershell
[Environment]::SetEnvironmentVariable(
  "LIBCLANG_PATH",
  "C:\Program Files\LLVM\bin",
  "User"
)
```

Verify that the tools are available before you compile Vifu:

```bash
cmake --version
clang --version
cargo --version
```

### Run Vifu

```bash
bun install --frozen-lockfile
cargo vifu
```

The first run creates these files:

- `~/.vifu/config.toml`
- `~/.vifu/providers.json`
- `~/.vifu/runtime.sqlite`
- `~/.vifu/vifu.sqlite`

The default process starts the Server and Agent Gateway. It also opens the Vifu
TUI in an interactive terminal. The first run creates one permanent `Local app`
and connects the bundled local Gateway to it. The Gateway stores its
Server-issued Device Token and reconnects with the same identity. Vifu does not
put an App ID in the runtime configuration.

Press `B` to open the local Dashboard. Press `q` to stop Vifu. Vifu asks for
confirmation if requests, a comparison, or a route override is active.

The first launch does not run a model. To add a model, read
[Agent Providers](../providers/README.md).

The generated configuration makes both network roles explicit:

```toml
[server]
address = "http://127.0.0.1:6790"

[gateway]
address = "http://127.0.0.1:6790"
```

The generated configuration starts both roles in the same process. The Server
uses `server.address`. The Agent Gateway uses `gateway.address` and connects to
the Server. Apps and deployment assignments remain Server state. If the Server
runs without the bundled Gateway, the first run still creates the `Local app`
and its `development` deployment, ready for a device to enroll.

For remote Servers, separate Gateways, monitor keys, and device enrollment, read
[Runtime topology, monitoring, and Gateway enrollment](topology-and-pairing.md).

On Unix systems, Vifu restricts the `~/.vifu` directory to mode `0700` and its
configuration and local database files to mode `0600`.

`cargo vifu` builds the official Console and then runs the Vifu binary. Plain
`cargo run -p vifu` embeds the assets already present in
`target/vifu-console-assets/`. It does not invoke Bun automatically. Use
`cargo vifu` after changing the Console UI so the embedded bundle stays current.
See [Embedded Console](embedded-console.md).

### Build Troubleshooting

If the build reports `Unable to find libclang`, verify that the operating
system package is installed. `LIBCLANG_PATH` must name the directory that
contains `libclang.so`, `libclang.dylib`, or `libclang.dll`. It must not name the
library file.

Errors from `llama-cpp-sys` or `whisper-rs-sys` normally mean that CMake, the
C/C++ compiler, or libclang is missing. Run the three verification commands
above, correct the missing tool, then run `cargo vifu` again.

The repository requests Rust's minimal toolchain profile so a first build does
not download the optional offline Rust documentation component. If an
interrupted rustup download left a partial toolchain, retry:

```bash
rustup toolchain install 1.95.0 --profile minimal --component clippy --component rustfmt
```

If a command fails before Cargo starts with
`bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted`, the host
sandbox did not create its loopback network interface. Vifu code did not run.
On Ubuntu 24.04, keep the user-namespace restriction enabled and load the
AppArmor profile recommended by the
[Codex sandbox prerequisites](https://developers.openai.com/codex/concepts/sandboxing#prerequisites):

```bash
sudo apt update
sudo apt install apparmor-profiles apparmor-utils
sudo install -m 0644 \
  /usr/share/apparmor/extra-profiles/bwrap-userns-restrict \
  /etc/apparmor.d/bwrap-userns-restrict
sudo apparmor_parser -r /etc/apparmor.d/bwrap-userns-restrict
```

The last command loads the profile without a reboot. Other sandbox runners must
use their own network-namespace policy. Do not change Vifu for this host error.

## Self-host

Docker Compose supports a complete operations stack with Dashboard, a headless
Server and Agent Gateway deployment, and a Server-only deployment. Follow the
canonical [self-hosting guide](self-hosting.md) for those paths.

## Stop

In the interactive TUI, press `q` or `Ctrl-C`. Vifu asks for confirmation if a
request or route override is active.

To preserve the PostgreSQL volume, stop the self-host Console stack with:

```bash
docker compose down
```
