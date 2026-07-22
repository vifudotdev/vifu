# Vifu Project Files

A `.vf` file is a portable snapshot of one editable Vifu project. It is the
recommended way to move a project between local or self-hosted Vifu
deployments, keep an offline copy, or share an editable design with another
creator.

## Format

Version 1 uses UTF-8 JSON with the media bytes embedded as Base64. The media
type is `application/vnd.vifu.project+json` and the file extension is `.vf`.
Every file declares:

- `format: "vifu-project"` and an integer `schemaVersion`;
- the project name and description;
- the editable `GameSourceV1` document;
- the Agent Profile versions needed by the draft;
- current and pinned structured-resource versions;
- current and referenced media versions, including their SHA-256 hashes;
- non-secret provider requirements; and
- a SHA-256 integrity digest over the canonical project document.

The JSON container is intentionally inspectable and suitable for versioned
format migration. A later compressed container can preserve the same logical
schema without changing the project model.

## Security Boundary

A project file never contains provider credentials, API keys, Agent Gateway
credentials, user sessions, runtime traces, analytics, or database connection
details. Releases and session history also stay with the deployment. Imported
Agent Profiles retain provider references, and the target deployment asks the
creator to configure those providers separately. Import creates non-secret
provider placeholders in **Needs configuration** state so each missing
credential is visible on the Providers page.

Media hashes and the archive digest are verified before any project is
created. Import creates a new project and remaps profile, version, resource,
and asset identifiers. It does not overwrite an existing project. If an import
step fails, Dashboard removes the incomplete project.

## Dashboard Workflow

Use the download action in Canvas or Short Drama to export the current saved
project. To import, open the project switcher and choose **Import .vf project**.
The imported project opens on the same Dashboard section after its libraries
and editable graph are restored.

## Runtime Release Bundles

The `.vf` project file remains the editable source of truth. Published releases
can also be downloaded from **Publish & API** as immutable `.vifu-game.json`
runtime bundles. A runtime bundle contains the compiled Game Plan, public
manifest, and pinned backend resources. It does not contain the editable Canvas
layout, provider credentials, or a required web presentation.
