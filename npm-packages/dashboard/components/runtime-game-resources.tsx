"use client";

import {
  Check,
  FileAudio,
  FileImage,
  FileJson,
  FileText,
  FileVideo,
  Link2,
  Plus,
  Trash2,
  Upload,
  X,
  type LucideIcon,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { runtimeBrowserRequest, runtimeBrowserUpload } from "../lib/runtime-browser-client";
import type {
  GameAsset,
  GameAssetVersion,
  GameDraft,
  GameResource,
  RuntimeProject,
} from "../lib/runtime-types";
import { RuntimeConfirmDialog } from "./runtime-confirm-dialog";

type LibraryTab = "structured" | "media";

export function RuntimeGameResources({
  project,
  draft,
  resources,
  assets,
}: {
  project: RuntimeProject;
  draft: GameDraft;
  resources: GameResource[];
  assets: GameAsset[];
}) {
  const [tab, setTab] = useState<LibraryTab>("structured");
  const [resourceDialog, setResourceDialog] = useState<GameResource | null | undefined>(undefined);
  const [assetDialog, setAssetDialog] = useState<GameAsset | null | undefined>(undefined);
  const [deletingResource, setDeletingResource] = useState<GameResource | null>(null);
  const [deletingAsset, setDeletingAsset] = useState<GameAsset | null>(null);
  const router = useRouter();

  async function deleteResource(resource: GameResource) {
    await runtimeBrowserRequest(
      `project/${encodeURIComponent(project.slug)}/game/resources/${resource.id}`,
      "DELETE",
    );
    setDeletingResource(null);
    router.refresh();
  }

  async function deleteAsset(asset: GameAsset) {
    await runtimeBrowserRequest(
      `project/${encodeURIComponent(project.slug)}/game/assets/${asset.id}`,
      "DELETE",
    );
    setDeletingAsset(null);
    router.refresh();
  }

  return (
    <div className="game-library-page">
      <header className="resource-page-heading game-library-heading">
        <div className="resource-page-summary">
          <strong>{resources.length + assets.length} project resources</strong>
          <span>Versioned data and media shared by Canvas, Short Drama, and published releases.</span>
        </div>
        <button
          className="primary-button compact"
          type="button"
          onClick={() => tab === "structured" ? setResourceDialog(null) : setAssetDialog(null)}
        >
          <Plus aria-hidden="true" />{tab === "structured" ? "Add resource" : "Import media"}
        </button>
      </header>

      <div className="game-library-tabs" role="tablist" aria-label="Resource type">
        <button className={tab === "structured" ? "active" : ""} type="button" onClick={() => setTab("structured")}>
          Structured <span>{resources.length}</span>
        </button>
        <button className={tab === "media" ? "active" : ""} type="button" onClick={() => setTab("media")}>
          Media <span>{assets.length}</span>
        </button>
      </div>

      {tab === "structured" ? (
        resources.length > 0 ? (
          <div className="game-resource-grid">
            {resources.map((resource) => (
              <StructuredResourceCard
                key={resource.id}
                project={project}
                draft={draft}
                resource={resource}
                onEdit={() => setResourceDialog(resource)}
                onDelete={() => setDeletingResource(resource)}
              />
            ))}
          </div>
        ) : (
          <button className="resource-empty-action" type="button" onClick={() => setResourceDialog(null)}>
            <span className="resource-empty-icon"><FileJson aria-hidden="true" /></span>
            <strong>Add gameplay data</strong>
            <span>Store prompts, lore, scripts, schemas, and localization as pinned versions.</span>
          </button>
        )
      ) : assets.length > 0 ? (
        <div className="game-resource-grid">
          {assets.map((asset) => (
            <MediaAssetCard
              key={asset.id}
              project={project}
              asset={asset}
              onUpload={() => setAssetDialog(asset)}
              onDelete={() => setDeletingAsset(asset)}
            />
          ))}
        </div>
      ) : (
        <button className="resource-empty-action" type="button" onClick={() => setAssetDialog(null)}>
          <span className="resource-empty-icon"><FileImage aria-hidden="true" /></span>
          <strong>Import the first media asset</strong>
          <span>Add images, video, audio, fonts, or subtitles as immutable versions.</span>
        </button>
      )}

      {resourceDialog !== undefined ? (
        <ResourceDialog
          project={project}
          draft={draft}
          resource={resourceDialog}
          onClose={() => setResourceDialog(undefined)}
        />
      ) : null}
      {assetDialog !== undefined ? (
        <AssetDialog
          project={project}
          asset={assetDialog}
          onClose={() => setAssetDialog(undefined)}
        />
      ) : null}
      {deletingResource ? (
        <RuntimeConfirmDialog
          title="Delete resource?"
          description={`${deletingResource.name} and all of its versions will be removed. Published releases keep their immutable snapshot.`}
          confirmLabel="Delete resource"
          onCancel={() => setDeletingResource(null)}
          onConfirm={() => deleteResource(deletingResource)}
        />
      ) : null}
      {deletingAsset ? (
        <RuntimeConfirmDialog
          title="Delete media asset?"
          description={`${deletingAsset.name} and its versions will be removed. Assets used by a Presentation release cannot be deleted.`}
          confirmLabel="Delete asset"
          onCancel={() => setDeletingAsset(null)}
          onConfirm={() => deleteAsset(deletingAsset)}
        />
      ) : null}
    </div>
  );
}

function StructuredResourceCard({
  project,
  draft,
  resource,
  onEdit,
  onDelete,
}: {
  project: RuntimeProject;
  draft: GameDraft;
  resource: GameResource;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pinned = draft.source.resources.some((item) => item.id === resource.resourceKey);

  async function togglePin() {
    setPending(true);
    setError(null);
    const references = pinned
      ? draft.source.resources.filter((item) => item.id !== resource.resourceKey)
      : [
        ...draft.source.resources.filter((item) => item.id !== resource.resourceKey),
        {
          id: resource.resourceKey,
          versionId: resource.id,
          kind: resource.kind,
          contentHash: resource.contentHash,
          approved: resource.approved,
        },
      ];
    try {
      await runtimeBrowserRequest(
        `project/${encodeURIComponent(project.slug)}/game/source`,
        "PUT",
        {
          source: { ...draft.source, resources: references },
          expectedRevision: draft.revision,
          expectedHash: draft.contentHash,
        },
      );
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <article className="game-resource-card">
      <button className="game-resource-card-main" type="button" onClick={onEdit}>
        <span className="game-resource-mark"><FileJson aria-hidden="true" /></span>
        <span className="game-resource-copy">
          <strong>{resource.name}</strong>
          <code>{resource.resourceKey}</code>
        </span>
        <span className={`game-resource-status ${resource.approved ? "approved" : "pending"}`}>
          {resource.approved ? "Approved" : "Draft"}
        </span>
      </button>
      <div className="game-resource-detail">
        <span>{resource.kind}</span><span>Version {resource.version}</span>
        {pinned ? <span className="pinned"><Link2 aria-hidden="true" />In game</span> : null}
      </div>
      <footer>
        <button className="secondary-button compact" type="button" disabled={pending || (!resource.approved && !pinned)} onClick={() => void togglePin()}>
          {pinned ? "Remove from game" : "Add to game"}
        </button>
        <button className="icon-button danger" type="button" onClick={onDelete} title="Delete resource" aria-label={`Delete ${resource.name}`}>
          <Trash2 aria-hidden="true" />
        </button>
      </footer>
      {error ? <p className="game-resource-error" role="alert">{error}</p> : null}
    </article>
  );
}

function MediaAssetCard({
  project,
  asset,
  onUpload,
  onDelete,
}: {
  project: RuntimeProject;
  asset: GameAsset;
  onUpload: () => void;
  onDelete: () => void;
}) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const latest = asset.versions[0];
  const Icon = mediaIcon(asset.kind);

  async function approve(version: GameAssetVersion) {
    setPending(true);
    setError(null);
    try {
      await runtimeBrowserRequest(
        `project/${encodeURIComponent(project.slug)}/game/assets/${asset.id}/versions/${version.id}/approve`,
        "POST",
        { status: "approved" },
      );
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <article className="game-resource-card media">
      <button className="game-resource-card-main" type="button" onClick={onUpload}>
        <span className="game-resource-mark"><Icon aria-hidden="true" /></span>
        <span className="game-resource-copy"><strong>{asset.name}</strong><code>{asset.assetKey}</code></span>
        <span className={`game-resource-status ${latest?.approvalStatus ?? "empty"}`}>
          {latest ? titleCase(latest.approvalStatus) : "No file"}
        </span>
      </button>
      <div className="game-resource-detail">
        <span>{asset.kind}</span>
        <span>{asset.versions.length} {asset.versions.length === 1 ? "version" : "versions"}</span>
        {latest ? <span>{formatBytes(latest.sizeBytes)}</span> : null}
      </div>
      <footer>
        <button className="secondary-button compact" type="button" onClick={onUpload}><Upload aria-hidden="true" />Upload version</button>
        {latest?.approvalStatus === "pending" ? (
          <button className="secondary-button compact" type="button" disabled={pending} onClick={() => void approve(latest)}><Check aria-hidden="true" />Approve</button>
        ) : null}
        <button className="icon-button danger" type="button" onClick={onDelete} title="Delete asset" aria-label={`Delete ${asset.name}`}><Trash2 aria-hidden="true" /></button>
      </footer>
      {error ? <p className="game-resource-error" role="alert">{error}</p> : null}
    </article>
  );
}

function ResourceDialog({
  project,
  draft,
  resource,
  onClose,
}: {
  project: RuntimeProject;
  draft: GameDraft;
  resource: GameResource | null;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => dialog.current?.showModal(), []);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const kind = String(form.get("kind") ?? "text");
    const rawContent = String(form.get("content") ?? "");
    let content: unknown = { text: rawContent };
    if (["json", "schema", "localization"].includes(kind)) {
      try {
        content = JSON.parse(rawContent);
      } catch {
        setError("Content must be valid JSON for this resource type.");
        return;
      }
    }
    setPending(true);
    setError(null);
    try {
      const path = resource
        ? `project/${encodeURIComponent(project.slug)}/game/resources/${resource.id}`
        : `project/${encodeURIComponent(project.slug)}/game/resources`;
      const result = await runtimeBrowserRequest<{ resource: GameResource }>(
        path,
        resource ? "PATCH" : "POST",
        {
          name: String(form.get("name") ?? ""),
          kind,
          content,
          approved: form.get("approved") === "on",
        },
      );
      const currentlyPinned = draft.source.resources.some((item) => item.id === result.resource.resourceKey);
      if (currentlyPinned) {
        const references = [
          ...draft.source.resources.filter((item) => item.id !== result.resource.resourceKey),
          {
            id: result.resource.resourceKey,
            versionId: result.resource.id,
            kind: result.resource.kind,
            contentHash: result.resource.contentHash,
            approved: result.resource.approved,
          },
        ];
        await runtimeBrowserRequest(
          `project/${encodeURIComponent(project.slug)}/game/source`,
          "PUT",
          {
            source: { ...draft.source, resources: references },
            expectedRevision: draft.revision,
            expectedHash: draft.contentHash,
          },
        );
      }
      dialog.current?.close();
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  const initialContent = resource
    ? typeof resource.content === "object" && resource.content !== null && "text" in resource.content
      ? String((resource.content as { text?: unknown }).text ?? "")
      : JSON.stringify(resource.content, null, 2)
    : "";
  return (
    <dialog className="resource-dialog game-resource-dialog" ref={dialog} onClose={onClose} onClick={(event) => {
      if (event.target === event.currentTarget) event.currentTarget.close();
    }}>
      <form className="resource-dialog-shell" onSubmit={(event) => void submit(event)}>
        <header><div><span>Structured resource</span><h2>{resource ? `Edit ${resource.name}` : "Add gameplay data"}</h2></div><button className="icon-button" type="button" onClick={() => dialog.current?.close()} aria-label="Close"><X aria-hidden="true" /></button></header>
        <div className="resource-dialog-fields">
          <label><span>Name</span><input name="name" defaultValue={resource?.name ?? ""} required maxLength={128} autoFocus /></label>
          <label><span>Type</span><select name="kind" defaultValue={resource?.kind ?? "prompt"}><option value="prompt">Prompt</option><option value="lore">Lore</option><option value="script">Script</option><option value="json">JSON data</option><option value="schema">Schema</option><option value="localization">Localization</option></select></label>
          <label className="wide"><span>Content</span><textarea name="content" defaultValue={initialContent} required spellCheck={false} /></label>
          <label className="resource-approval-check"><input name="approved" type="checkbox" defaultChecked={resource?.approved ?? true} /><span>Approved for publishing</span></label>
        </div>
        {resource ? <p className="resource-dialog-note">Saving creates a new immutable version. A draft using this resource will move to the new version.</p> : null}
        {error ? <p className="inline-error" role="alert">{error}</p> : null}
        <footer><button className="secondary-button" type="button" onClick={() => dialog.current?.close()}>Cancel</button><button className="primary-button" type="submit" disabled={pending}>{pending ? "Saving..." : resource ? "Save new version" : "Add resource"}</button></footer>
      </form>
    </dialog>
  );
}

function AssetDialog({ project, asset, onClose }: { project: RuntimeProject; asset: GameAsset | null; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => dialog.current?.showModal(), []);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const file = form.get("file");
    if (!(file instanceof File) || file.size === 0) {
      setError("Choose a media file to upload.");
      return;
    }
    setPending(true);
    setError(null);
    try {
      let target = asset;
      if (!target) {
        const created = await runtimeBrowserRequest<{ asset: GameAsset }>(
          `project/${encodeURIComponent(project.slug)}/game/assets`,
          "POST",
          { name: String(form.get("name") ?? ""), kind: String(form.get("kind") ?? "image") },
        );
        target = created.asset;
      }
      const upload = new FormData();
      upload.set("file", normalizedMediaFile(file, target.kind));
      upload.set("rightsStatus", String(form.get("rightsStatus") ?? "unreviewed"));
      upload.set("metadata", "{}");
      upload.set("provenance", JSON.stringify({ originalName: file.name }));
      const uploaded = await runtimeBrowserUpload<{ version: GameAssetVersion }>(
        `project/${encodeURIComponent(project.slug)}/game/assets/${target.id}/versions`,
        upload,
      );
      if (form.get("approved") === "on" && uploaded.version.approvalStatus !== "approved") {
        await runtimeBrowserRequest(
          `project/${encodeURIComponent(project.slug)}/game/assets/${target.id}/versions/${uploaded.version.id}/approve`,
          "POST",
          { status: "approved" },
        );
      }
      dialog.current?.close();
      router.refresh();
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setPending(false);
    }
  }

  return (
    <dialog className="resource-dialog game-resource-dialog" ref={dialog} onClose={onClose} onClick={(event) => {
      if (event.target === event.currentTarget) event.currentTarget.close();
    }}>
      <form className="resource-dialog-shell" onSubmit={(event) => void submit(event)}>
        <header><div><span>Immutable media</span><h2>{asset ? `Upload ${asset.name}` : "Import media"}</h2></div><button className="icon-button" type="button" onClick={() => dialog.current?.close()} aria-label="Close"><X aria-hidden="true" /></button></header>
        <div className="resource-dialog-fields">
          {!asset ? <label><span>Name</span><input name="name" required maxLength={128} autoFocus /></label> : null}
          {!asset ? <label><span>Type</span><select name="kind" defaultValue="image"><option value="image">Image</option><option value="video">Video</option><option value="audio">Audio</option><option value="subtitle">Subtitle</option><option value="font">Font</option></select></label> : null}
          <label className="wide"><span>File</span><input name="file" type="file" required /></label>
          <label><span>Rights</span><select name="rightsStatus" defaultValue="unreviewed"><option value="unreviewed">Unreviewed</option><option value="owned">Owned</option><option value="licensed">Licensed</option><option value="public_domain">Public domain</option></select></label>
          <label className="resource-approval-check"><input name="approved" type="checkbox" /><span>Approve after upload</span></label>
        </div>
        <p className="resource-dialog-note">Media is stored by content hash. Uploading again creates or reuses an immutable version.</p>
        {error ? <p className="inline-error" role="alert">{error}</p> : null}
        <footer><button className="secondary-button" type="button" onClick={() => dialog.current?.close()}>Cancel</button><button className="primary-button" type="submit" disabled={pending}>{pending ? "Uploading..." : "Upload"}</button></footer>
      </form>
    </dialog>
  );
}

function mediaIcon(kind: string): LucideIcon {
  if (kind === "video") return FileVideo;
  if (kind === "audio") return FileAudio;
  if (kind === "image") return FileImage;
  return FileText;
}

function normalizedMediaFile(file: File, kind: string): File {
  if (file.type || kind !== "subtitle") return file;
  return new File([file], file.name, {
    type: file.name.toLowerCase().endsWith(".vtt") ? "text/vtt" : "application/x-subrip",
    lastModified: file.lastModified,
  });
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / 1024 / 1024).toFixed(1)} MiB`;
}

function titleCase(value: string): string {
  return value.replaceAll("_", " ").replace(/^./, (letter) => letter.toUpperCase());
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Resource request failed.";
}
