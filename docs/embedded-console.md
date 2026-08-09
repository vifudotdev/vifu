# Embedded Console

The Vifu binary includes a Console on the configured Vifu Server. A release
binary starts the default local Server, Agent Gateway, and Console from one
process:

```bash
vifu
```

In an interactive terminal, the binary opens the live Runtime TUI. Press `B`
to open the Console at `server.address`. Its default address is:

```text
http://127.0.0.1:6790
```

The Docker self-host Console remains the operations Console for a PostgreSQL
deployment. See [Self-host with Docker](self-hosting.md) for that stack.

## Runtime Shape

The embedded Console is a static browser application served by the Rust server:

```text
packages/console
        |
        v
npm-packages/dashboard/embedded/main.tsx
        |
        v
bun run build:console
        |
        v
target/vifu-console-assets/
        |
        v
crates/vifu-server/build.rs
        |
        v
vifu binary -> /
```

Rust does not render React. It embeds the generated HTML, JavaScript, CSS, and
brand assets, serves the Console and API from the same configured Server
listener. It proxies `/api/runtime/*` internally with server-side authority.
Changing `server.address` changes the address for both surfaces. It does not
start another Console listener.
The browser executes the React bundle and calls the Runtime API through that
same-origin proxy.

This deployment is similar to a web GUI inside a native server binary. The UI
uses HTML, CSS, and JavaScript. Vifu distributes it as one executable.

## Navigation And Caching

The embedded Console is a single-page application. Internal `/project/...`
links update browser history and React route state instead of reloading the HTML
shell. Modified clicks, downloads, external URLs, and new-tab targets keep
normal browser behavior.

Generated JavaScript and CSS use hashed filenames:

```text
/assets/main-<hash>.js
/assets/main-<hash>.css
```

Those files are served with long-lived cache headers. `index.html` is served
with `no-store`, so a rebuilt or released binary can point browsers at the new
asset hash.

## Project State

The Console edits runtime project state through the Server API. Projects,
provider bindings, agents, endpoints, API keys, deployments, releases, and
traces live in the configured database.

Project Settings JSON is only an import/export artifact for backup, migration,
and embedded targets. Normal runtime reads come from the database, not from a
project settings file. See [Project Settings](project-settings.md).

Provider-local details such as model paths, URLs, and credentials stay with the
Provider registry or host process. Project state binds provider keys to agents
and endpoints.

## Build From Source

When building from source, build the Console assets before building or running
the Rust binary:

```bash
bun run build:console
cargo run -p vifu
```

`cargo run` embeds whatever is already in `target/vifu-console-assets/`. It
does not invoke Bun automatically, so build the Console before compiling Vifu.
If the asset directory is absent, the Rust server embeds a fallback page that
asks the developer to run `bun run build:console`.

After startup, use the TUI for live supervision and press `B` for persistent
Trace and Comparison history. The Console uses Vifu's same-origin Server proxy,
so its browser bundle never receives the local Admin Key.

The release workflow runs `bun run build:console`, verifies `index.html`, and
requires the bundle when compiling every release binary. A release fails rather
than silently shipping the fallback page when those assets are unavailable.

## Development Boundaries

Use `packages/console` for the host-neutral `@vifu/console` React views and
contracts. The embedded Console supplies only the local host adapter. This
adapter defines routing, branding, the Runtime API base, uploads, and refreshes.

The Dashboard host remains responsible for its server-side authority, sessions,
and deployment policy. Put a shared view in the Console package if another host
uses it. Keep host-specific behavior in the host adapter.

Current limitations:

- When no operations Dashboard is attached, the Server serves the embedded
  Console on `server.address`. Its administration proxy accepts only requests
  originating on the Server host. Other devices continue to use the public
  Runtime and Gateway APIs.
- It is a static browser app. It must not depend on Next.js server
  features, server actions, or React Server Components.
- Browser-visible code must not contain admin keys, provider credentials,
  deploy keys, or signing material.
- Large provider files, model downloads, and device-local credentials are not
  bundled into the Console assets.
- Self-hosted and cloud deployments use the Dashboard host attached to that
  same Server address for account sessions and multi-user access.
