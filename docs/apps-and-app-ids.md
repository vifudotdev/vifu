# Apps and App IDs

An App is the public unit that groups Agent Profiles, providers, endpoints,
deployments, Gateways, API keys, and traces. Creating an App also creates:

- its primary `development` deployment; and
- a stable public App ID in the form `vifu_app_<64 hex characters>`.

The App ID lets an installation register its own Gateway with that App. It is
an enrollment selector, not an API key. It cannot read traces, call Agents, or
open the Console. Each installation creates its own Machine identity and
receives its own Server-issued Device Token after registration.

## Enroll a CLI Gateway

The CLI configuration describes where the Server and Gateway run. It does not
select an App. Pair an unconfigured CLI or device Gateway from **App →
Devices → Pair device**. The one-time QR identifies the exact deployment
that the Gateway joins. The Server then issues a Device Token for reconnects.

On an empty local installation, Vifu creates one `Local app` so the TUI and
Dashboard have an immediate starting point. In the default combined process,
the bundled Gateway joins that App during the same first startup. This
attachment is not written to `config.toml`. A Server-only first startup creates
the App and its empty `development` deployment, then waits for Gateways to
enroll.

Later startups do not create another App, move a Gateway, or change deployment
assignments. Creating and selecting other Apps remains explicit. The TUI's
current App is a monitoring and pairing context; it is not an automatic
Gateway assignment rule.

When upgrading from a Vifu version that stored `gateway.app_id`, Vifu removes
that legacy field and moves its saved Gateway session to the normal profile
scope. The App ID is used only during that migration if the installation still
needs its first Device Token.

## Configure an Embedded Mobile Runtime

Store the Server URL and App ID as application state. Pass both values to the
platform Gateway adapter when the application selects an App. An App ID enrolls
a new installation into that App's primary deployment. Use a one-time pairing
QR when the operator must select another deployment precisely.

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

## Hosted First Use and Manual Pairing

The official hosted Server gives an unconfigured Gateway a temporary App.
Reconnects from the stored Machine identity and Device Token return to the same
App. Claiming it transfers the App to the signed-in account without changing
its App ID, Gateway installations, or existing traces. This hosted policy is
not part of the user configuration file.

QR pairing is the operator-directed enrollment path. Open
**App → Devices → Pair device**, or press `P` in a TUI connected to the
current App. The one-time QR selects that App deployment for the scanned
installation. Packaged Apps can use an App ID when every installation should
join the same App's primary deployment.
