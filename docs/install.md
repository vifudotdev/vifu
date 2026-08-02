# Install Vifu

## Download A Release

Download the archive for your platform from the
[latest release](https://github.com/vifudotdev/vifu/releases/latest).

- macOS and Linux archives use `.tar.gz`.
- Windows archives use `.zip`.

Extract the archive, then start the local Server and Agent Gateway:

```bash
./vifu
```

The same process serves the local Console. Open the URL printed at startup,
normally:

```text
http://127.0.0.1:6790
```

On Windows:

```powershell
.\vifu.exe
```

## Build From Source

Use Cargo when you want to build from source.

### Requirements

- Rust from `rust-toolchain.toml`
- CMake, a C/C++ compiler, and libclang
- Bun and Node.js for the embedded Console bundle

The default Vifu build includes llama.cpp and Local Whisper, so the native build
tools are normal source-build dependencies. The Rust
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

Verify the tools before compiling:

```bash
cmake --version
clang --version
cargo --version
```

### Run Vifu

```bash
bun install --frozen-lockfile
bun run build:console
cargo run -p vifu
```

The first run creates `~/.vifu/config.json` and
`~/.vifu/providers.json`. With the default configuration, one process runs the
Server and Agent Gateway roles. Runtime and Gateway state is stored in
`~/.vifu/runtime.sqlite`; local Server data is stored separately in
`~/.vifu/vifu.sqlite`. It opens the local Dashboard automatically and keeps
running until you press `Ctrl-C`. Provider credentials and model files are not
required for this first launch. Add providers when you are ready to run a model;
see [Agent Providers](../providers/README.md).

On Unix systems, Vifu restricts the `~/.vifu` directory to mode `0700` and its
configuration and local database files to mode `0600`.

`cargo run` embeds the assets already generated in
`target/vifu-console-assets/`; it does not invoke Bun automatically. The source
startup sequence above builds the official Console first. Re-run
`bun run build:console` after changing its UI and before rebuilding Vifu. See
[Embedded Console](embedded-console.md).

### Build Troubleshooting

If the build reports `Unable to find libclang`, confirm that the operating
system package above is installed. `LIBCLANG_PATH` must name the directory
that contains `libclang.so`, `libclang.dylib`, or `libclang.dll`; it must
not name the library file itself.

Errors from `llama-cpp-sys` or `whisper-rs-sys` normally mean that CMake, the
C/C++ compiler, or libclang is missing. Run the three verification commands
above, correct the missing tool, then run `cargo run -p vifu` again.

The repository requests Rust's minimal toolchain profile so a first build does
not download the optional offline Rust documentation component. If an
interrupted rustup download left a partial toolchain, retry:

```bash
rustup toolchain install 1.95.0 --profile minimal --component clippy --component rustfmt
```

If a command fails before Cargo starts with
`bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted`, the host
sandbox could not create its loopback network interface. Vifu code did not run.
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

The last command loads the profile without a reboot. Other sandbox runners
should be fixed through their own network-namespace policy rather than by
changing Vifu.

## Self-host

Docker Compose supports a complete operations stack with Dashboard, a headless
Server and Agent Gateway deployment, and a Server-only deployment. Follow the
canonical [self-hosting guide](self-hosting.md) for those paths.

## Stop

Stop a release or source process with `Ctrl-C`. Stop the Console stack while
preserving its PostgreSQL volume with:

```bash
docker compose down
```
