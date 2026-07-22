import { runtimeBrowserRequest, runtimeBrowserUpload } from "./runtime-browser-client";
import type {
  AgentProfileDetail,
  GameAsset,
  GameAssetVersion,
  GameDraft,
  GameResource,
  GameSource,
  ProfileVersionWithCapabilities,
  ProjectProvider,
  RuntimeProject,
} from "./runtime-types";

export const VIFU_PROJECT_FORMAT = "vifu-project";
export const VIFU_PROJECT_SCHEMA_VERSION = 1;
export const VIFU_PROJECT_MIME_TYPE = "application/vnd.vifu.project+json";

const MAX_ARCHIVE_BYTES = 256 * 1024 * 1024;

type ArchiveProject = Pick<RuntimeProject, "name" | "description"> & { originalSlug: string };

type ArchiveProfileVersion = {
  archiveId: string;
  persona: Record<string, unknown>;
  runtime: Record<string, unknown>;
  presentation: Record<string, unknown>;
  source: Record<string, unknown>;
  capabilities: Array<{
    kind: string;
    providerType: string;
    providerKey: string;
    resourceId: string | null;
    config: Record<string, unknown>;
    inputSchema: Record<string, unknown>;
    outputSchema: Record<string, unknown>;
  }>;
  changeSummary: string | null;
};

type ArchiveProfile = {
  archiveId: string;
  slug: string;
  name: string;
  description: string | null;
  activeVersionId: string;
  versions: ArchiveProfileVersion[];
};

type ArchiveResource = Omit<GameResource, "id" | "projectId" | "createdAt" | "updatedAt"> & {
  archiveId: string;
};

type ArchiveAssetVersion = Omit<GameAssetVersion, "id" | "projectId" | "assetId" | "createdAt"> & {
  archiveId: string;
  data: string;
};

type ArchiveAsset = Pick<GameAsset, "assetKey" | "name" | "kind"> & {
  archiveId: string;
  versions: ArchiveAssetVersion[];
};

type ArchiveProviderRequirement = Pick<
  ProjectProvider,
  "providerKey" | "name" | "providerType" | "baseUrl" | "config" | "sourceKind" | "sourceKey"
>;

export type VifuProjectArchiveV1 = {
  format: typeof VIFU_PROJECT_FORMAT;
  schemaVersion: typeof VIFU_PROJECT_SCHEMA_VERSION;
  exportedAt: string;
  generator: { name: "Vifu"; version: string };
  project: ArchiveProject;
  source: GameSource;
  profiles: ArchiveProfile[];
  resources: ArchiveResource[];
  assets: ArchiveAsset[];
  providerRequirements: ArchiveProviderRequirement[];
  historyIncluded: false;
  integrity: { algorithm: "sha256"; digest: string };
};

type ImportProgress = (message: string) => void;

export async function exportProjectArchive(
  projectSlug: string,
  source: GameSource,
  onProgress: ImportProgress = () => undefined,
): Promise<void> {
  onProgress("Collecting project data...");
  const [{ projects }, { profiles }, { resources }, { assets }, { providers }, status] = await Promise.all([
    runtimeBrowserRequest<{ projects: RuntimeProject[] }>("projects"),
    runtimeBrowserRequest<{ profiles: Array<{ id: string }> }>(`project/${encodeURIComponent(projectSlug)}/profiles`),
    runtimeBrowserRequest<{ resources: GameResource[] }>(`project/${encodeURIComponent(projectSlug)}/game/resources`),
    runtimeBrowserRequest<{ assets: GameAsset[] }>(`project/${encodeURIComponent(projectSlug)}/game/assets`),
    runtimeBrowserRequest<{ providers: ProjectProvider[] }>(`project/${encodeURIComponent(projectSlug)}/providers`),
    runtimeBrowserRequest<{ version: string }>("status"),
  ]);
  const project = projects.find((item) => item.slug === projectSlug);
  if (!project) throw new Error("The project is no longer available.");

  const referencedIds = collectStringValues(source);
  const profileDetails = await Promise.all(profiles.map(({ id }) => (
    runtimeBrowserRequest<AgentProfileDetail>(`project/${encodeURIComponent(projectSlug)}/profiles/${encodeURIComponent(id)}`)
  )));
  const archiveProfiles = profileDetails.map((detail) => archiveProfile(detail, referencedIds));
  const archiveResources = await collectResources(projectSlug, resources, source);
  const archiveAssets = await collectAssets(projectSlug, assets, referencedIds, onProgress);
  const payload: Omit<VifuProjectArchiveV1, "integrity"> = {
    format: VIFU_PROJECT_FORMAT,
    schemaVersion: VIFU_PROJECT_SCHEMA_VERSION,
    exportedAt: new Date().toISOString(),
    generator: { name: "Vifu" as const, version: status.version },
    project: { name: project.name, description: project.description, originalSlug: project.slug },
    source,
    profiles: archiveProfiles,
    resources: archiveResources,
    assets: archiveAssets,
    providerRequirements: providers.map(({ providerKey, name, providerType, baseUrl, config, sourceKind, sourceKey }) => ({
      providerKey,
      name,
      providerType,
      baseUrl,
      config,
      sourceKind,
      sourceKey,
    })),
    historyIncluded: false as const,
  };
  onProgress("Finalizing project file...");
  const archive = await createProjectArchive(payload);
  downloadBlob(
    `${project.slug}.vf`,
    new Blob([JSON.stringify(archive, null, 2)], { type: VIFU_PROJECT_MIME_TYPE }),
  );
}

export async function createProjectArchive(
  payload: Omit<VifuProjectArchiveV1, "integrity">,
): Promise<VifuProjectArchiveV1> {
  return {
    ...payload,
    integrity: { algorithm: "sha256", digest: await sha256Text(canonicalJson(payload)) },
  };
}

export async function readProjectArchive(file: File): Promise<VifuProjectArchiveV1> {
  if (file.size > MAX_ARCHIVE_BYTES) throw new Error("The .vf file exceeds the 256 MiB import limit.");
  let archive: unknown;
  try {
    archive = JSON.parse(await file.text());
  } catch {
    throw new Error("The selected file is not a valid Vifu project.");
  }
  assertArchiveShape(archive);
  const { integrity, ...payload } = archive;
  const digest = await sha256Text(canonicalJson(payload));
  if (digest !== integrity.digest) throw new Error("The project file failed its integrity check.");
  for (const asset of archive.assets) {
    for (const version of asset.versions) {
      const bytes = decodeBase64(version.data);
      if (bytes.byteLength !== version.sizeBytes) throw new Error(`Media ${asset.name} has an invalid size.`);
      const digest = `sha256:${await sha256Bytes(bytes)}`;
      if (digest !== version.contentHash) throw new Error(`Media ${asset.name} failed its integrity check.`);
    }
  }
  return archive;
}

export async function importProjectArchive(
  archive: VifuProjectArchiveV1,
  onProgress: ImportProgress = () => undefined,
): Promise<RuntimeProject> {
  let createdProject: RuntimeProject | null = null;
  try {
    onProgress("Creating project...");
    const { projects } = await runtimeBrowserRequest<{ projects: RuntimeProject[] }>("projects");
    const slug = importedProjectSlug(archive.project.originalSlug, projects);
    const created = await runtimeBrowserRequest<{ project: RuntimeProject }>("projects", "POST", {
      slug,
      name: archive.project.name,
      description: archive.project.description,
    });
    createdProject = created.project;
    const createdSlug = createdProject.slug;
    const idMap = new Map<string, string>();

    for (const [index, provider] of archive.providerRequirements.entries()) {
      onProgress(`Restoring providers ${index + 1}/${archive.providerRequirements.length}...`);
      await runtimeBrowserRequest(
        `project/${encodeURIComponent(createdSlug)}/providers/import`,
        "POST",
        {
          providerKey: provider.providerKey,
          name: provider.name,
          providerType: provider.providerType,
          baseUrl: provider.baseUrl,
          config: provider.config,
        },
      );
    }

    for (const [index, profile] of archive.profiles.entries()) {
      onProgress(`Restoring agents ${index + 1}/${archive.profiles.length}...`);
      const imported = await runtimeBrowserRequest<{
        profile: { id: string };
        versionMap: Record<string, string>;
      }>(`project/${encodeURIComponent(createdSlug)}/profiles/import`, "POST", profile);
      idMap.set(profile.archiveId, imported.profile.id);
      for (const [archiveId, versionId] of Object.entries(imported.versionMap)) idMap.set(archiveId, versionId);
    }

    const resourcesByKey = new Map<string, ArchiveResource[]>();
    for (const resource of archive.resources) {
      const versions = resourcesByKey.get(resource.resourceKey) ?? [];
      versions.push(resource);
      resourcesByKey.set(resource.resourceKey, versions);
    }
    let restoredResources = 0;
    for (const versions of resourcesByKey.values()) {
      versions.sort((left, right) => left.version - right.version);
      let current: GameResource | null = null;
      for (const resource of versions) {
        onProgress(`Restoring data ${++restoredResources}/${archive.resources.length}...`);
        const result: { resource: GameResource } = current
          ? await runtimeBrowserRequest<{ resource: GameResource }>(
            `project/${encodeURIComponent(createdSlug)}/game/resources/${encodeURIComponent(current.id)}`,
            "PATCH",
            { name: resource.name, kind: resource.kind, content: resource.content, approved: resource.approved },
          )
          : await runtimeBrowserRequest<{ resource: GameResource }>(
            `project/${encodeURIComponent(createdSlug)}/game/resources`,
            "POST",
            {
              resourceKey: resource.resourceKey,
              name: resource.name,
              kind: resource.kind,
              content: resource.content,
              approved: resource.approved,
            },
          );
        const restored = result.resource;
        current = restored;
        idMap.set(resource.archiveId, restored.id);
      }
    }

    let restoredAssets = 0;
    const assetVersionCount = archive.assets.reduce((total, asset) => total + asset.versions.length, 0);
    for (const asset of archive.assets) {
      const createdAsset = await runtimeBrowserRequest<{ asset: { id: string } }>(
        `project/${encodeURIComponent(createdSlug)}/game/assets`,
        "POST",
        { assetKey: asset.assetKey, name: asset.name, kind: asset.kind },
      );
      idMap.set(asset.archiveId, createdAsset.asset.id);
      for (const version of asset.versions) {
        onProgress(`Restoring media ${++restoredAssets}/${assetVersionCount}...`);
        const bytes = decodeBase64(version.data);
        const form = new FormData();
        const fileBytes = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
        form.set("file", new File([fileBytes], asset.name, { type: version.mimeType }));
        form.set("metadata", JSON.stringify(version.metadata));
        form.set("provenance", JSON.stringify(version.provenance));
        form.set("rightsStatus", version.rightsStatus);
        const uploaded = await runtimeBrowserUpload<{ version: GameAssetVersion }>(
          `project/${encodeURIComponent(createdSlug)}/game/assets/${encodeURIComponent(createdAsset.asset.id)}/versions`,
          form,
        );
        idMap.set(version.archiveId, uploaded.version.id);
        if (version.approvalStatus !== "pending") {
          await runtimeBrowserRequest(
            `project/${encodeURIComponent(createdSlug)}/game/assets/${encodeURIComponent(createdAsset.asset.id)}/versions/${encodeURIComponent(uploaded.version.id)}/approve`,
            "POST",
            { status: version.approvalStatus },
          );
        }
      }
    }

    onProgress("Restoring the game design...");
    const { draft } = await runtimeBrowserRequest<{ draft: GameDraft }>(
      `project/${encodeURIComponent(createdSlug)}/game/source`,
    );
    await runtimeBrowserRequest(
      `project/${encodeURIComponent(createdSlug)}/game/source`,
      "PUT",
      {
        source: replaceArchiveIds(archive.source, idMap),
        expectedRevision: draft.revision,
        expectedHash: draft.contentHash,
      },
    );
    return createdProject;
  } catch (error) {
    if (createdProject) {
      await runtimeBrowserRequest(`projects/${encodeURIComponent(createdProject.id)}`, "DELETE").catch(() => undefined);
    }
    throw error;
  }
}

function archiveProfile(detail: AgentProfileDetail, referencedIds: Set<string>): ArchiveProfile {
  const activeVersionId = detail.profile.activeVersionId;
  if (!activeVersionId) throw new Error(`Agent ${detail.profile.name} has no active version.`);
  const versions = detail.versions
    .filter(({ version }) => version.archivedAt === null && (version.id === activeVersionId || referencedIds.has(version.id)))
    .sort((left, right) => left.version.versionNumber - right.version.versionNumber)
    .map(archiveProfileVersion);
  if (!versions.some((version) => version.archiveId === activeVersionId)) {
    throw new Error(`Agent ${detail.profile.name} is missing its active version.`);
  }
  return {
    archiveId: detail.profile.id,
    slug: detail.profile.slug,
    name: detail.profile.name,
    description: detail.profile.description,
    activeVersionId,
    versions,
  };
}

function archiveProfileVersion({ version, capabilities }: ProfileVersionWithCapabilities): ArchiveProfileVersion {
  return {
    archiveId: version.id,
    persona: version.persona,
    runtime: version.runtime,
    presentation: version.presentation,
    source: version.source,
    capabilities: capabilities.map(({ kind, providerType, providerKey, resourceId, config, inputSchema, outputSchema }) => ({
      kind,
      providerType,
      providerKey,
      resourceId,
      config,
      inputSchema,
      outputSchema,
    })),
    changeSummary: version.changeSummary,
  };
}

async function collectResources(projectSlug: string, latest: GameResource[], source: GameSource): Promise<ArchiveResource[]> {
  const resources = new Map(latest.map((resource) => [resource.id, resource]));
  for (const reference of source.resources) {
    if (resources.has(reference.versionId)) continue;
    const result = await runtimeBrowserRequest<{ resource: GameResource }>(
      `project/${encodeURIComponent(projectSlug)}/game/resources/${encodeURIComponent(reference.versionId)}`,
    );
    resources.set(result.resource.id, result.resource);
  }
  return [...resources.values()].map(({ id, projectId: _projectId, createdAt: _createdAt, updatedAt: _updatedAt, ...resource }) => ({
    ...resource,
    archiveId: id,
  }));
}

async function collectAssets(
  projectSlug: string,
  assets: GameAsset[],
  referencedIds: Set<string>,
  onProgress: ImportProgress,
): Promise<ArchiveAsset[]> {
  let completed = 0;
  const selected = assets.map((asset) => ({
    asset,
    versions: asset.versions.filter((version, index) => index === 0 || referencedIds.has(version.id)),
  }));
  const total = selected.reduce((count, item) => count + item.versions.length, 0);
  const result: ArchiveAsset[] = [];
  for (const { asset, versions } of selected) {
    const archivedVersions: ArchiveAssetVersion[] = [];
    for (const version of [...versions].reverse()) {
      onProgress(`Packing media ${++completed}/${total}...`);
      const response = await fetch(
        `/api/runtime/project/${encodeURIComponent(projectSlug)}/game/assets/${encodeURIComponent(asset.id)}/versions/${encodeURIComponent(version.id)}/content`,
      );
      if (!response.ok) throw new Error(`Could not export ${asset.name}.`);
      const bytes = new Uint8Array(await response.arrayBuffer());
      if (`sha256:${await sha256Bytes(bytes)}` !== version.contentHash) {
        throw new Error(`Media ${asset.name} failed its integrity check.`);
      }
      const { id, projectId: _projectId, assetId: _assetId, createdAt: _createdAt, ...portable } = version;
      archivedVersions.push({ ...portable, archiveId: id, data: encodeBase64(bytes) });
    }
    result.push({
      archiveId: asset.id,
      assetKey: asset.assetKey,
      name: asset.name,
      kind: asset.kind,
      versions: archivedVersions,
    });
  }
  return result;
}

function assertArchiveShape(value: unknown): asserts value is VifuProjectArchiveV1 {
  if (!isRecord(value) || value.format !== VIFU_PROJECT_FORMAT || value.schemaVersion !== VIFU_PROJECT_SCHEMA_VERSION) {
    throw new Error("This Vifu project format is not supported.");
  }
  if (!isRecord(value.project) || typeof value.project.name !== "string" || !isRecord(value.source)) {
    throw new Error("The Vifu project is missing its editable source.");
  }
  if (
    !Array.isArray(value.profiles)
    || !Array.isArray(value.resources)
    || !Array.isArray(value.assets)
    || !Array.isArray(value.providerRequirements)
  ) {
    throw new Error("The Vifu project libraries are invalid.");
  }
  if (!isRecord(value.integrity) || value.integrity.algorithm !== "sha256" || typeof value.integrity.digest !== "string") {
    throw new Error("The Vifu project has no supported integrity digest.");
  }
}

function replaceArchiveIds<T>(value: T, idMap: Map<string, string>): T {
  if (typeof value === "string") return (idMap.get(value) ?? value) as T;
  if (Array.isArray(value)) return value.map((item) => replaceArchiveIds(item, idMap)) as T;
  if (!isRecord(value)) return value;
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, replaceArchiveIds(item, idMap)])) as T;
}

function collectStringValues(value: unknown, result = new Set<string>()): Set<string> {
  if (typeof value === "string") result.add(value);
  else if (Array.isArray(value)) value.forEach((item) => collectStringValues(item, result));
  else if (isRecord(value)) Object.values(value).forEach((item) => collectStringValues(item, result));
  return result;
}

function importedProjectSlug(originalSlug: string, projects: RuntimeProject[]): string {
  const existing = new Set(projects.map((project) => project.slug));
  if (!existing.has(originalSlug)) return originalSlug;
  const base = `${originalSlug.slice(0, Math.max(1, 64 - "-imported".length))}-imported`;
  if (!existing.has(base)) return base;
  for (let suffix = 2; suffix < 1000; suffix += 1) {
    const ending = `-${suffix}`;
    const candidate = `${base.slice(0, 64 - ending.length)}${ending}`;
    if (!existing.has(candidate)) return candidate;
  }
  throw new Error("Could not allocate a project slug for this import.");
}

function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (isRecord(value)) {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

async function sha256Text(value: string): Promise<string> {
  return sha256Bytes(new TextEncoder().encode(value));
}

async function sha256Bytes(value: Uint8Array): Promise<string> {
  const source = value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength) as ArrayBuffer;
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", source));
  return [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function encodeBase64(bytes: Uint8Array): string {
  const chunks: string[] = [];
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    chunks.push(String.fromCharCode(...bytes.subarray(offset, offset + 0x8000)));
  }
  return btoa(chunks.join(""));
}

function decodeBase64(value: string): Uint8Array {
  const encoded = atob(value);
  const bytes = new Uint8Array(encoded.length);
  for (let index = 0; index < encoded.length; index += 1) bytes[index] = encoded.charCodeAt(index);
  return bytes;
}

function downloadBlob(name: string, blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
