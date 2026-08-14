# Vifu

![Vifu](npm-packages/dashboard/public/brand/vifu-lockup.png)

<div align="center">

<b>Build, connect, and inspect on-device AI agents.</b>

[![Crates.io](https://img.shields.io/crates/v/vifu.svg)](https://crates.io/crates/vifu)
[![PyPI](https://img.shields.io/pypi/v/vifu.svg)](https://pypi.org/project/vifu/)
[![Runtime API](https://docs.rs/vifu-runtime/badge.svg)](https://docs.rs/vifu-runtime)
[![CI](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml/badge.svg)](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.com/invite/VdqqFwJbNE)

[quick start](#quick-start) / [developer guides](docs/get-started/README.md) / [examples](examples/README.md) / [documentation](docs/README.md) / [releases](https://github.com/vifudotdev/vifu/releases/latest)

</div>

## Quick start

### Desktop Arm: Build a Python Agent App on macOS

1. Run the [Web Research Agent App](examples/foundry-local-python/README.md).

```bash
uv run --with "vifu[foundry]" https://raw.githubusercontent.com/vifudotdev/vifu/main/examples/foundry-local-python/main.py
```

2. Open the Dashboard at [http://127.0.0.1:6790](http://127.0.0.1:6790).

3. Open **Web Research -> Traces**.

4. Enter a topic at the `Research>` prompt. Open the new Trace and check:

   - **Latency**: total request time.
   - **first_token**: time before the first model output.
   - **decode**: time spent generating the remaining output.

5. Open **Agents -> Local Researcher -> Prompt**, edit the Prompt, then select
   **Save & make live**.

6. Enter the same topic again.

7. Open the latest [Trace](docs/observability.md). Confirm **Configuration used**
   shows the new Prompt, then compare its timings with the first Trace.

### Mobile Arm: Trace On-device Model Performance on Android

1. Download and extract the [latest Vifu release](https://github.com/vifudotdev/vifu/releases/latest).

2. Start `./vifu` once. When the TUI opens, press `q` and confirm the stop.

3. Find the LAN IPv4 address of the computer. Edit `~/.vifu/config.toml`:

```toml
[server]
address = "https://<computer-lan-ip>:6790"

[gateway]
address = "http://127.0.0.1:6790"
```

Replace the complete `server.address`. Set its scheme to `https://`. Replace
`127.0.0.1` with the LAN IPv4 address of the computer. Keep `gateway.address`
on `http://127.0.0.1:6790`. Connect the phone and the computer to the same
network.

4. Start `./vifu` again. Keep the TUI open, then press `B` to open the Dashboard.

5. Install the [optimized Android Starter APK](https://github.com/vifudotdev/vifu/releases/download/android-starter-v0.1.1/vifu-android-starter.apk).

6. Open the Starter. Tap the folder button in the lower-right corner and select
   a GGUF model.

7. In **Overview**, select **Pair device**, then select **Copy pairing code**.

8. In the Starter, tap the Vifu status row at the top. Paste the code, select
   **Pair**, and wait for `Vifu: connected`.

9. Open **Traces** in the Dashboard. Keep the Dashboard and TUI visible.

10. Enter a message in the Starter. Inspect latency, first-token time, decode
    time, and token rate in the new Trace.

11. Install the [baseline APK](https://github.com/vifudotdev/vifu/releases/download/android-starter-v0.1.1/vifu-android-starter-baseline.apk)
    beside the optimized APK. Select the same model and create a new pairing
    code for this application.

12. Enter the same message in both applications. Compare their Traces to measure
    the ARM optimization on the same phone and model.

The [Android guide](examples/android-starter/README.md) includes model setup,
pairing details, and source-build instructions.

## What Vifu does

Vifu is a development platform for Agent Apps that use on-device models. Each
App groups its Agents, Providers, devices, endpoints, and traces.

Your product owns its interface, domain state, safety rules, and allowed
actions. Vifu supplies the Runtime, Gateway, Server, TUI, and Dashboard.

- Register multiple Agents and Providers in one App.
- Expose each Agent through a stable endpoint.
- Inspect prompts, versions, model calls, inference stages, and performance.
- Compare models, builds, Providers, and devices with attributable traces.
- Call local Agents through one OpenAI-compatible API.
- Manage many independent Apps from one Vifu installation.

## Supported environments

| Environment | Guide |
| --- | --- |
| macOS, Linux, or Windows | [Install Vifu](docs/install.md) |
| Python | [Build a Python App](docs/get-started/python.md) |
| Node.js | [Build a TypeScript App](docs/get-started/typescript.md) |
| Android | [Use the Android Starter](examples/android-starter/README.md) |
| iPhone, iPad, or macOS App | [Build with Swift](docs/get-started/swift.md) |
| Godot on iPhone, iPad, or macOS | [Build with Godot](docs/get-started/godot.md) |
| Rust or an embedded device | [Embed the Runtime](docs/runtime-embedding.md) |

## Documentation

#### Build applications

- [Build with Python, TypeScript, Swift, Kotlin, Godot, or Rust](docs/get-started/README.md)
- [Run the examples and mobile starters](examples/README.md)
- [Add Providers](providers/README.md) and [integrate agent frameworks](docs/integrations/README.md)
- [Embed the Runtime](docs/runtime-embedding.md)

#### Operate Vifu

- [Install Vifu](docs/install.md)
- [Read traces and compare performance](docs/observability.md)
- [Connect devices and Gateways](docs/topology-and-pairing.md)
- [Self-host Vifu](docs/self-hosting.md)
- [Read the full documentation index](docs/README.md)

## Contributing

Read the [build guide](BUILD.md), [contribution guide](CONTRIBUTING.md), and
[security policy](SECURITY.md).

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name and logos. See [TRADEMARKS.md](TRADEMARKS.md).
