# Game Runtime

Vifu publishes a Project's connected gameplay graph as a durable, engine-neutral
Game endpoint. The endpoint owns Agent orchestration, gameplay state, choices,
effects, and session recovery. A web game, native engine, or headless process can
drive the same release through HTTP and CloudEvents over SSE.

## Publish And Create A Key

Build the graph in **Canvas** or **Short Drama**, validate it, and publish an
immutable Game Release. Open **Publish & API** to copy the Game URL and create a
key with only **Game Runtime: Execute** enabled.

```text
http://localhost:6790/{projectSlug}/v1/game
```

Keep the key in a server-side secret store when the game has a trusted backend.
A key embedded in a distributed browser or native build can be extracted, so
use a dedicated Game key and revoke or replace it independently of management
keys.

## Preview A Draft

**Preview & QA** compiles the current draft revision and starts it with the same
Bevy runtime used by a published Game. The preview session pins that compiled
plan, so later Canvas or Short Drama edits cannot alter a session already in
progress. Preview sessions are management-only, do not become public releases,
and are excluded from production analytics.

Agent nodes pin an exact Profile version when the draft is compiled. Tool nodes
also select an Agent and may invoke only tools recorded in that Profile
version's capability catalog. The matching provider must be configured for the
Project. Provider credentials are resolved by Vifu Server at execution time and
never enter the Game plan, manifest, events, or public session response.

## Run A Release

The convenience route creates a durable session and sends `game.start`:

```bash
curl 'http://localhost:6790/my-game/v1/game/run' \
  --request POST \
  --header "Authorization: Bearer $VIFU_API_KEY" \
  --header 'Content-Type: application/json' \
  --data '{
    "host": {
      "engine": "my-game",
      "adapterVersion": "1.0.0",
      "capabilities": []
    },
    "input": {}
  }'
```

For full lifecycle control, create a session and submit commands separately:

```bash
curl 'http://localhost:6790/my-game/v1/game/sessions' \
  --request POST \
  --header "Authorization: Bearer $VIFU_API_KEY" \
  --header 'Content-Type: application/json' \
  --data '{"host":{"engine":"my-game","capabilities":[]}}'
```

```bash
curl 'http://localhost:6790/my-game/v1/game/sessions/$SESSION_ID/commands' \
  --request POST \
  --header "Authorization: Bearer $VIFU_API_KEY" \
  --header 'Content-Type: application/json' \
  --data '{
    "idempotencyKey": "start-1",
    "expectedRevision": 0,
    "type": "game.start",
    "data": {}
  }'
```

Use a unique `idempotencyKey` for each intended command. Repeating the same key
returns the stored result instead of advancing the session twice. Supply the
latest session revision to detect concurrent updates.

## Events And Reconnect

Events follow CloudEvents 1.0 and include a monotonic `sequence`. Reconnect with
the last processed sequence to replay everything committed after it:

```bash
curl --no-buffer \
  'http://localhost:6790/my-game/v1/game/sessions/$SESSION_ID/events' \
  --header "Authorization: Bearer $VIFU_API_KEY" \
  --header 'Accept: text/event-stream' \
  --header 'Last-Event-ID: 12'
```

The public session response contains status, revision, public output, failures,
and outstanding host actions. Internal state, Agent conversations, prompts,
provider routing, and effect payloads stay on authenticated management traces.
Agent and Tool work appears as child spans of the `game.run` or `game.command`
trace that requested the effect. Public trace summaries contain effect identity
and status rather than prompt or provider response content.

## Bind Engine Resources

`GET /manifest` declares stable logical resources and required host
capabilities. A host binding maps those IDs to resources owned by the game:

```json
{
  "schemaVersion": 1,
  "engine": "godot",
  "bindings": {
    "scene.tavern.background": {
      "kind": "node-path",
      "value": "/root/Game/TavernBackground"
    }
  }
}
```

The binding can remain in the game's repository. Vifu's optional Presentation
Release uses the same contract to map logical IDs to managed asset versions.
Changing a concrete binding does not change the backend Game Release.

A blocking `host.action.requested` event includes an `actionId`, target, action,
and arguments. Perform the engine-native operation, then send
`host.action.completed` or `host.action.failed` with that `actionId`.

## Headless Host Example

The reference fixture uses the public HTTP and SSE contracts directly. It
validates capabilities and bindings, handles choices and host actions, waits for
Agent effects, and replays the completed event stream.

```bash
VIFU_GAME_URL='http://localhost:6790/my-game/v1/game' \
VIFU_API_KEY="$VIFU_API_KEY" \
VIFU_HOST_BINDINGS='docs/examples/host-bindings.example.json' \
node docs/examples/headless-host.mjs
```

The machine-readable route and schema reference is
[game-runtime-openapi.yaml](game-runtime-openapi.yaml).
