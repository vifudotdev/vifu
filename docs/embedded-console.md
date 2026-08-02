# Embedded Console

The Vifu binary includes a local Console for the loopback runtime. A release
binary starts the local Server, Agent Gateway, and Console from one process:

```bash
vifu
```

The binary prints the Console URL at startup. With the default local address it
is:

```text
http://127.0.0.1:6790
```

The Docker self-host Console remains the operations Console for a PostgreSQL
deployment. See [Self-host with Docker](self-hosting.md) for that stack.

## Runtime Shape

The embedded Console is a static browser application served by the Rust server:

```text
packages/runtime-console
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
brand assets, serves the Console from `/`, and proxies `/api/runtime/*` to the
local Server API with server-side authority.
The browser executes the React bundle and calls the Runtime API through that
same-origin proxy.

This is the same deployment class as tools that compile a web GUI into a
native server binary: the UI is ordinary HTML/CSS/JavaScript, but distribution
is one executable.

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
does not invoke Bun automatically, so Rust builds still work in environments
that do not have JavaScript dependencies installed.

If the asset directory is absent, the Rust server embeds a fallback page that
asks the developer to run `bun run build:console`.

Release packaging should run `bun run build:console` before compiling the
release binary.

## Development Boundaries

Use `packages/runtime-console` for host-neutral React views and contracts. The
embedded Console supplies only the local host adapter: routing, branding,
runtime API base, upload behavior, and refresh behavior.

The Next.js dashboard host remains responsible for its own server-side
authority, sessions, and deployment policy. New shared views should live in the
runtime console package when they are useful to both hosts, and host-specific
behavior should stay in the host adapter.

Current limitations:

- The embedded Console is available only in local loopback mode.
- It is a static browser app, so it should not depend on Next.js server
  features, server actions, or React Server Components.
- Browser-visible code must not contain admin keys, provider credentials,
  deploy keys, or signing material.
- Large provider files, model downloads, and device-local credentials are not
  bundled into the Console assets.
- Self-hosted and cloud deployments should continue to use the dashboard host
  that matches their authority model.
