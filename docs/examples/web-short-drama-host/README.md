# Web Short Drama Host

This dependency-free example renders a published Vifu Game Runtime endpoint as
a portrait interactive anime drama. The browser receives presentation assets,
runtime events, and session commands from this local host. The Game API key
stays in the Node process and is never sent to browser code.

Create a Game key in **Publish & API**, then start the example:

```bash
cd docs/examples/web-short-drama-host
VIFU_GAME_URL='http://127.0.0.1:6790/last-train-to-the-moon/v1/game' \
VIFU_API_KEY_FILE="$HOME/.vifu/last-train-game-key" \
node server.mjs
```

Open <http://127.0.0.1:4180>. The host supports the published image and audio
presentation, choices, free-text Agent interactions, durable session events,
and the `vifu.world.object-action.v1` hold action used by the sample game.

The example uses only the public routes documented in
[`game-runtime-openapi.yaml`](../../game-runtime-openapi.yaml). Replace
`VIFU_GAME_URL` with another published Project endpoint to use the same host
with a different runtime release.
