# Runtime Topology, Monitoring, and Gateway Enrollment

Vifu uses four parts:

- An embedded Runtime runs Agents and Providers inside a product.
- An Agent Gateway connects a Runtime to a Server.
- A Vifu Server stores Apps, deployments, keys, and traces.
- A Vifu TUI reads the monitor stream from a Server.

```text
Application agents / embedded Runtime
                 |
                 v
          Agent Gateway
                 |
       Gateway WebSocket
                 v
             Vifu Server
                 ^
        Monitor WebSocket
                 |
              Vifu TUI
```

The Server is the source of operational state. Gateways send their Agent roster
and Runtime telemetry to the Server. Each TUI reads the Server monitor stream.
The Server resolves App profiles before it filters each device roster.

An application can also use an embedded Runtime without a Server. In this mode,
the host calls `VifuRuntime` directly. A later Gateway connection does not
change the Runtime API of the application.

## Choose a Network Shape

`server.address` selects the Server. `gateway.address` selects the Gateway. A
loopback address starts that role in this `vifu` process. Another address uses
a role that runs elsewhere.

| Server | Gateway | What this `vifu` process starts | TUI data source |
| --- | --- | --- | --- |
| Local | Local | Server and Gateway | Local Server monitor |
| Local | Remote | Server | Local Server monitor |
| Remote | Local | Gateway | Remote Server monitor |
| Remote | Remote or omitted | Neither network role | Remote Server monitor |

The default first run uses the first row. It starts a local Server, a local
Gateway, and the TUI. It also creates a temporary local Guest App.

Startup does not enroll another device. Press `P` to create an enrollment QR
for another Runtime installation.

## Monitor a Server

For a remote Server, give the TUI an App monitor key:

```bash
export VIFU_MONITOR_KEY='vifu_pk_...'
./vifu
```

`VIFU_MONITOR_KEY_FILE` reads the key from a private file. The App key must
have App read or write access. Its monitor stream contains only that App.

An account credential can monitor Apps owned by that account. A deployment
operator can use `VIFU_ADMIN_KEY` or `VIFU_ADMIN_KEY_FILE` for deployment-wide
access.

A headless Gateway does not need a monitor key. A local Guest Gateway gives its
App key to the TUI after bootstrap.

`server.address` is the Server API origin. Press `B` to open the Dashboard URL
reported by that Server. A local Server serves the embedded Console from the
same origin. A self-hosted or cloud Server can report a separate authenticated
Dashboard origin. Older Servers that do not report one keep the same-origin
behavior.

## Enroll a Gateway

Pairing authorizes one Gateway installation to one App deployment. It does
not pair a phone to a TUI, and it does not identify two processes as the same
user.

1. In Dashboard, open **App → Deployments → Pair gateway**, or press `P` in
   a TUI whose Gateway owns a Guest App.
2. Server creates a `vifu_ge_...` enrollment token with a five-minute lifetime.
3. A mobile application scans the HTTPS pairing QR, or a CLI reads the token
   through `VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN_FILE`.
4. Gateway presents its stable Machine identity and consumes the token once.
5. Server returns a Server-scoped Device Token. The Gateway stores that token
   in the platform credential store and uses it for later reconnects.

The TUI closes the QR after the Server verifies the same enrollment. A reconnect
from another Gateway does not close it.

The QR keeps its exact terminal dimensions. If the window is too small, the TUI
shows the required size.

A Dashboard QR uses the native `vifu://gateway/enroll?...` link for a local
Server with a pinned certificate. A Server with a system-trusted certificate
uses the `https://vifu.ai/pair#...` application-link bridge. Both links contain
the Server address from the current configuration.

The Dashboard's **Copy pairing code** action copies the complete native
`vifu://` payload, including the generated certificate and fingerprint for a
local Server. Android and iOS Starters validate that pair and pin the
certificate for later connections. The compact TUI QR carries the fingerprint
instead of the larger certificate and is intended for a scanner that resolves
that trust anchor before enrollment.

Only the selected Server validates and consumes the enrollment token.

`vifu_gb_...` is the managed deployment bootstrap credential used between
Server and Gateway roles started by the same deployment. It is not a user
pairing code and is never emitted by the CLI pairing action or accepted by the
mobile pairing parser.

## Guest Bootstrap

Guest bootstrap is an explicit Server policy. When enabled, a Gateway that
connects without an App ID or enrollment token can receive one independent
temporary App, deployment, App API key, and claim token. Each Gateway Machine
identity receives its own Guest App. The returned App key has read access, so
it can authenticate an App-scoped monitor without receiving
deployment-admin authority.

A `vifu_ge_...` enrollment always selects its App deployment and never creates
a Guest App. A managed `vifu_gb_...` credential never enters the Guest flow.
Claiming a Guest App associates it with the signed-in owner but does not replace
its App ID, Gateway identity, or traces.

## Android Runtime Modes

One Android application and one Gateway protocol cover the complete lifecycle.
The application chooses its starting state from configuration and protected
device storage. It does not use a separate demo transport.

| Application state | First connection | Later connections | Intended use |
| --- | --- | --- | --- |
| Unconfigured | Guest bootstrap or scan a one-time `vifu_ge_...` QR | Stored Device Token | Personal evaluation or an operator-selected App |
| App configured | Present the editable `vifu_app_...` App ID | Stored Device Token | The same App installed on many devices |
| Previously enrolled | Present the stored Device Token | Rotated Device Token | Normal restart and reconnect |

Every App has a stable App ID and primary development deployment. Store the
Server origin and App ID as editable application configuration. Each
installation keeps its own Machine identity, Gateway row, and Device Token, so
one owner can monitor the same App across many phones without sharing a device
credential. An App ID selects an App. It is not a monitor credential and does
not grant Dashboard, TUI, trace, or Agent access. See
[Apps and App IDs](apps-and-app-ids.md).

The Android Runtime publishes its agent roster and performance trace stages.
The App registration path sends timing, stage status, model identity, and
bounded errors. It does not upload conversation input or model output. A product
with full-content debugging must provide a separate in-app consent setting. The
product must call the opt-in `start_with_monitor_io` API. The
ordinary embedded Gateway `start` path keeps root invocation content on device.

## Private MacBook-to-Phone Setup

This path stays on infrastructure controlled by the user.

1. Connect the MacBook and phone to a network where they can reach each other.
2. Get the current LAN address of the MacBook.
3. Add this address to the configuration:

```toml
[server]
address = "https://192.168.1.20:6790"

[server.guest_bootstrap]
enabled = true

[gateway]
address = "http://127.0.0.1:6790"
```

Start `vifu`. It detects the non-loopback address and listens on the LAN. It also
creates deployment secrets and a TLS certificate in the private Vifu directory.

Press `P` and scan the terminal QR. Android can open the `vifu://` link from the
system camera; the iOS Starter can scan it from the Gateway sheet. The QR
contains the certificate fingerprint and one-time enrollment token. The phone
uses them to authenticate and pin the private Server.

Changing Wi-Fi can change the MacBook address. Update `server.address` and scan
a fresh enrollment QR for a new installation. Networks with client isolation
can block direct device-to-laptop traffic. Use a reachable self-hosted or cloud
Server for that network shape.

## Cloud First-Use Setup

For the shortest public evaluation path, point the CLI at the hosted Server for
that run. This leaves the user's default local configuration unchanged:

```bash
./vifu \
  -c server.address=https://api.vifu.dev \
  -c gateway.address=http://127.0.0.1:6790
```

The equivalent persistent configuration is:

```toml
[server]
address = "https://api.vifu.dev"

[gateway]
address = "http://127.0.0.1:6790"
```

The cloud Server grants that Gateway a temporary Guest App. The TUI uses the
returned App key in memory for the App-scoped monitor stream. Press `P` and scan
the QR with the Mobile Starter. The phone joins the same App, and its Runtime traces
appear in that TUI. Claiming the App later preserves its App ID, Gateway
installations, and trace history.

## Credential Boundaries

| Credential | Presented by | Purpose | Scope |
| --- | --- | --- | --- |
| `vifu_app_...` | New App installation | Automatic App registration | One App; separate identity per installation |
| `vifu_ge_...` | New external Gateway | One-time App deployment enrollment | One deployment, one use |
| Device Token | Enrolled Gateway | Reconnect and configuration/telemetry transport | One Gateway installation |
| `vifu_pk_...` with App read | TUI or App client | App traces and monitor stream | One App |
| Account deployment credential | Signed-in operator tooling | Owned App operations and monitoring | Owner's Apps |
| Admin Key | Deployment operator | Server administration and full monitoring | Whole deployment |
| `vifu_gb_...` | Managed internal Gateway | Deployment bootstrap | Internal deployment roles |

Gateway transport and monitor transport intentionally use different
credentials. Possession of a Gateway Device Token does not grant TUI access,
and an App monitor key cannot enroll a new Gateway.

The shared protocol boundaries can be exercised without a mobile UI using the
[topology protocol live-test matrix](topology-live-testing.md).
