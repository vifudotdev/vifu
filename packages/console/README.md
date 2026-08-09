# @vifu/console

Shared Vifu Console contracts and React views for local, self-hosted, and Cloud
hosts.

The package is host-neutral: callers provide routing, refresh, branding, and
runtime API behavior through a React provider. It does not own deployment auth
or server-side proxy policy.

`browserApiBaseUrl` is the inference service base. The Console presents the
fixed OpenAI-compatible `/v1` routes below that base. A project key selects the
project, and the request `model` selects an Agent in that project. Management
requests continue through the host-provided request adapter.
