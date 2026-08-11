# Apps and App IDs

An App is the public unit that groups Agent Profiles, providers, endpoints,
deployments, Gateways, API keys, and traces. Creating an App also creates:

- its primary `development` deployment; and
- a stable public App ID in the form `vifu_app_<64 hex characters>`.

The App ID lets an installation register its own Gateway with that App. It is
an enrollment selector, not an API key. It cannot read traces, call Agents, or
open the Console. Each installation creates its own Machine identity and
receives its own Server-issued Device Token after registration.

## Configure a CLI Gateway

Copy the App ID from **App settings** in the Console. Configure the reachable
Server and the App ID in `~/.vifu/config.toml`:

```toml
[server]
address = "https://api.vifu.dev"

[gateway]
address = "http://127.0.0.1:6790"
app_id = "vifu_app_<64 hex characters>"
```

`VIFU_APP_ID` can provide the same value for one process. On the first
connection, the Gateway registers with the App and stores its Device Token.
Later connections use that Device Token. Changing `app_id` selects another App
and uses a separate persisted Gateway session.

## Configure an Embedded Mobile Runtime

Store the Server URL and App ID as editable application state. Pass both values
to the platform Gateway adapter when the user selects an App. Do not require a
new application build to change them.

All installations can use the same App ID. Each phone still appears as a
separate Gateway because every installation has a separate Machine identity
and Device Token. Requests made through an App use that App's configuration and
produce traces in that App.

## Create an App through the API

An account or deployment credential with App write access can create an App:

```bash
curl --fail-with-body \
  --request POST \
  --header "Authorization: Vifu $VIFU_CREDENTIAL" \
  --header 'Content-Type: application/json' \
  --data '{"name":"My agent app"}' \
  "$VIFU_SERVER/v1/apps"
```

The response contains `app.appId`. The development deployment and App
registration are ready when this request returns.

## Guest Apps and Manual Pairing

When Guest bootstrap is enabled, an unconfigured Gateway receives a temporary
Guest App. Claiming it transfers the App to the signed-in account without
changing its App ID, Gateway installations, or traces.

QR pairing remains available as an operator-directed rebind. Open
**App → Deployments → Pair gateway**, or press `P` in a TUI that can manage the
current Guest App. The one-time QR selects that App deployment for the scanned
installation. Normal packaged Apps should use the editable App ID path.
