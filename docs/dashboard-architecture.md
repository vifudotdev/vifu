# Dashboard architecture

Vifu provides one Dashboard for Runtime operations. The same application is
embedded in the `vifu` binary and can run beside a self-managed Vifu Server.

## Package responsibilities

- `packages/console` contains the host-neutral `@vifu/console` project, Agent, Provider,
  endpoint, trace, release, deployment, and settings views.
- `npm-packages/dashboard` composes those views into the bundled Next.js
  Dashboard, including local authority, HttpOnly sessions, routing, and the
  embedded-browser entrypoint.
- Vifu Server remains the authority for Runtime state and permissions. The
  browser never owns an Admin Key or Provider credential.

## Host integration

`@vifu/console` is also the integration surface for other web hosts. A host
provides its Runtime API base URL, credential source, navigation, branding, and
refresh behavior while reusing the same views and HTTP contracts.

The browser inference base and management transport have separate jobs. The
inference base exposes fixed OpenAI-compatible routes such as
`/v1/chat/completions`; the project key selects the project. Project CRUD,
traces, keys, and deployment operations use the host's injected request adapter
and retain their project-addressed management routes.

The dependency direction stays one-way:

```text
host application -> @vifu/console -> Vifu Server HTTP API
```

`@vifu/console` does not import a host application, identity provider, edge
runtime, or deployment implementation. Runtime capabilities and authorization
come from the connected Vifu Server, so hosts do not duplicate permission
logic in the browser.
