# Runtime Topology, Monitoring, And Gateway Enrollment

Vifu uses the same three roles in local, self-hosted, and hosted deployments:

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

The Server is the operational source of truth. Gateways publish their agent
roster and Runtime telemetry to it. Every TUI—including a TUI running in the
same process as a local Gateway—reads the Server monitor stream. This keeps the
view consistent when several laptops, phones, or embedded products connect to
one deployment.

Project monitor authorization follows deployment-to-Gateway assignments, so a
single project can expose every enrolled installation. Logical project profiles
are resolved to their physical provider/resource IDs before the Server filters
the device roster; profile display slugs are not treated as Gateway agent IDs.

An application may also use an embedded Runtime offline. In that shape the
host invokes `VifuRuntime` directly and there is no Server, Gateway connection,
monitor stream, or enrollment step. Connecting an embedded Runtime later does
not change its application-facing Runtime API.

## The Four Network Shapes

`server.address` selects where the Server runs. `gateway.address` selects
whether this CLI starts a Gateway. A loopback/local address means the role is
owned by this process; another origin means the role runs elsewhere.

| Server | Gateway | What this `vifu` process starts | TUI data source |
| --- | --- | --- | --- |
| Local | Local | Server and Gateway | Local Server monitor |
| Local | Remote | Server | Local Server monitor |
| Remote | Local | Gateway | Remote Server monitor |
| Remote | Remote or omitted | Neither network role | Remote Server monitor |

The default first run is the first row. It creates a loopback configuration,
starts the local Server and Gateway, creates one temporary local Guest project,
and opens the TUI. Device enrollment is not part of startup and no pairing QR
is shown automatically. Press `P` only when another Runtime installation is
ready to scan its one-time enrollment QR.

For a remote Server, an interactive TUI normally uses a separate monitor
credential:

```bash
export VIFU_MONITOR_KEY='vifu_pk_...'
./vifu
```

`VIFU_MONITOR_KEY_FILE` reads the credential from a private file. A project API
key must have `project: read` or `project: write`; its monitor stream contains
only that project. An account deployment credential can see projects owned by
that account. Deployment operators may use `VIFU_ADMIN_KEY` or
`VIFU_ADMIN_KEY_FILE` as a compatibility fallback and receive the deployment
scope. Headless Gateway-only operation does not need a monitor credential.
When the same CLI starts a new local Gateway and the Server explicitly grants
it a Guest project, the TUI waits for bootstrap to finish and reads that Guest
project key from the Gateway's protected local session automatically.

`server.address` is the single Server origin for API and Dashboard access.
Press `B` to open that address. The TUI does not classify the Server as
loopback, LAN, self-hosted, or hosted before deciding where the Dashboard lives;
it opens the configured Server just like any other client. In local mode the
Server serves the embedded Console itself. Self-hosted and hosted Servers may
attach their authenticated operations Dashboard to the same origin.

## Enrollment Is Gateway To Server

Pairing authorizes one Gateway installation to one project deployment. It does
not pair a phone to a TUI, and it does not identify two processes as the same
user.

1. In Dashboard, open **Project → Deployments → Pair gateway**, or press `P` in
   a TUI whose Gateway owns a Guest project.
2. Server creates a `vifu_ge_...` enrollment token with a five-minute lifetime.
3. A mobile application scans the HTTPS pairing QR, or a CLI reads the token
   through `VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN_FILE`.
4. Gateway presents its stable Machine identity and consumes the token once.
5. Server returns a Server-scoped Device Token. The Gateway stores that token
   in the platform credential store and uses it for later reconnects.

The TUI tracks the enrollment ID behind the displayed QR and closes that QR
only when the Server confirms that exact enrollment. Other Gateway reconnects
do not dismiss it. The QR is rendered at its exact terminal dimensions; if the
window is too small, the TUI asks for the required size instead of clipping or
reflowing the code.

Dashboard QR codes may contain an `https://vifu.ai/pair#...` bridge URL so an
ordinary camera app can open them. Sensitive enrollment data stays in the URL
fragment and the bridge hands it to the installed application as a
`vifu://gateway/enroll?...` deep link. The TUI displays the direct `vifu://`
payload for its in-app scanner, including the generated local Server certificate
fingerprint when the Server is privately hosted. The app captures the presented
certificate, verifies that fingerprint, and pins the verified DER for later
connections. In both cases, the selected Server is the only component that
validates and consumes the token.

`vifu_gb_...` is the managed deployment bootstrap credential used between
Server and Gateway roles started by the same deployment. It is not a user
pairing code and is never emitted by the CLI pairing action or accepted by the
mobile pairing parser.

## Guest Bootstrap

Guest bootstrap is an explicit Server policy. When enabled, a Gateway that
connects without an enrollment token may receive one independent temporary
project, deployment, project API key, and claim token. Each Gateway Machine
identity receives its own guest project. The returned project key has project
read access, so it can authenticate a project-scoped monitor without receiving
deployment-admin authority.

A `vifu_ge_...` enrollment always selects its project deployment and never
creates a Guest project. A managed `vifu_gb_...` credential never enters the
Guest flow. Claiming a Guest project associates it with the signed-in owner but
does not replace the Gateway identity.

## Android Runtime modes

One Android application and one Gateway protocol cover the complete lifecycle.
The application chooses its starting state from configuration and protected
device storage; it does not use a separate demo transport.

| Application state | First connection | Later connections | Intended use |
| --- | --- | --- | --- |
| Unconfigured | Scan a one-time `vifu_ge_...` QR | Stored Device Token | Personal evaluation or an operator-selected project |
| Distribution configured | Present a `vifu_di_...` Distribution ID | Stored Device Token | The same published app installed on many devices |
| Previously enrolled | Present the stored Device Token | Rotated Device Token | Normal restart and reconnect |

A Runtime Distribution is a Server resource that names one project deployment,
sets a maximum installation count, and can be revoked. Each installation keeps
its own Machine identity, Gateway row, and Device Token. Therefore one owner can
monitor the same application across many phones without sharing a device
credential between them. Distribution IDs select a project; they are not
monitor credentials and do not grant Dashboard or TUI access.

Create a Distribution with a project key that has project write access, then
put only the returned public ID and Server origin into the application build:

```bash
curl --fail-with-body \
  --request POST \
  --header "Authorization: Bearer $VIFU_PROJECT_KEY" \
  --header 'Content-Type: application/json' \
  --data '{"name":"Android public demo","maxGateways":1000}' \
  "$VIFU_SERVER/v1/project/$VIFU_PROJECT_SLUG/runtime-distributions"
```

The response field `distribution.publicId` is the `vifu_di_...` value. It is a
public enrollment selector, not a secret. Revoke the Distribution without
rebuilding the app by posting to
`/v1/project/{slug}/runtime-distributions/{distributionId}/revoke` with the
same project-write authority.

The Android Runtime publishes its agent roster and performance trace stages.
The public distribution path sends timing, stage status, model identity, and
bounded errors. It does not upload conversation input or model output. A product
that offers full-content debugging must expose that as a separate, explicit
in-app consent setting and call the opt-in `start_with_monitor_io` API. The
ordinary embedded Gateway `start` path keeps root invocation content on device.

## Private MacBook-to-phone setup

This path stays on infrastructure controlled by the user. Put the MacBook and
phone on a network where they can reach each other, choose the MacBook's current
LAN address, and configure:

```toml
[server]
address = "https://192.168.1.20:6790"

[server.guest_bootstrap]
enabled = true

[gateway]
address = "http://127.0.0.1:6790"
```

Starting `vifu` recognizes the non-loopback local address, listens on the LAN,
creates deployment secrets and a TLS certificate in the private Vifu config
directory, and opens the TUI. Press `P`, open **Scan Vifu QR** in the Android
app, and scan the terminal QR. The certificate fingerprint and one-time
enrollment travel inside the direct QR, so the phone can authenticate and pin
that private Server without using the Vifu website.

Changing Wi-Fi can change the MacBook address. Update `server.address` and scan
a fresh enrollment QR for a new installation. Networks with client isolation
may block direct device-to-laptop traffic; use a reachable self-hosted or hosted
Server for that network shape.

## Hosted first-use setup

For the shortest public evaluation path, point the CLI at the hosted Server for
that run. This leaves the user's default local configuration unchanged:

```bash
./vifu \
  -c server.address=https://api.vifu.ai \
  -c gateway.address=http://127.0.0.1:6790
```

The equivalent persistent configuration is:

```toml
[server]
address = "https://api.vifu.ai"

[gateway]
address = "http://127.0.0.1:6790"
```

The hosted Server grants that Gateway a temporary Guest project. The TUI uses
the returned project key in memory for the project-scoped monitor stream. Press
`P` and scan the QR in the Android app; the phone joins the same project and its
Runtime traces appear in that TUI. Claiming the project later preserves its
Gateway installations and trace history.

## Credential Boundaries

| Credential | Presented by | Purpose | Scope |
| --- | --- | --- | --- |
| `vifu_ge_...` | New external Gateway | One-time project deployment enrollment | One deployment, one use |
| Device Token | Enrolled Gateway | Reconnect and configuration/telemetry transport | One Gateway installation |
| `vifu_pk_...` with project read | TUI or project client | Project traces and monitor stream | One project |
| Account deployment credential | Signed-in operator tooling | Owned project operations and monitoring | Owner's projects |
| Admin Key | Deployment operator | Server administration and full monitoring | Whole deployment |
| `vifu_gb_...` | Managed internal Gateway | Deployment bootstrap | Internal deployment roles |

Gateway transport and monitor transport intentionally use different
credentials. Possession of a Gateway Device Token does not grant TUI access,
and a project monitor key cannot enroll a new Gateway.

The shared protocol boundaries can be exercised without a mobile UI using the
[topology protocol live-test matrix](topology-live-testing.md).
