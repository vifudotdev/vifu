# Project Settings

Project Settings are the provider, agent, and endpoint graph for one Vifu
project. They are normal project state: the Console edits them and Vifu Server
stores them in the database.

Use JSON only as an import/export artifact. Export settings to back up a
project, move it to another deployment, or bundle the same graph with an
embedded target. Import the JSON through the Console when the target should use
that project graph.

Project Settings reference Provider keys. Provider URLs, model paths,
credentials, and device-local resource settings stay with the Provider registry
or host process that runs the Provider.

Embedded runtimes can apply the same settings through the typed Runtime API:

```rust
use vifu_runtime::prelude::*;

let mut settings = ProjectSettings::new("my-project");
settings.providers = vec![];
settings.agents = vec![];
settings.endpoints = vec![];

runtime.apply_project_settings(settings)?;
```

See [Embed the Runtime](runtime-embedding.md) for host integration and
[Provider integrations](../providers/README.md) for Provider registration.
