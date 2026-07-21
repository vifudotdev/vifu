"use client";

import { useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent } from "react";
import { useRouter } from "next/navigation";
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  horizontalListSortingStrategy,
  sortableKeyboardCoordinates,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  Bot,
  Boxes,
  Check,
  ChevronRight,
  Clapperboard,
  ExternalLink,
  GitBranch,
  Image as ImageIcon,
  Languages,
  Library,
  ListTree,
  LoaderCircle,
  Pause,
  Play,
  Plus,
  Scissors,
  Search,
  Sparkles,
  Trash2,
  Users,
  Volume2,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import {
  DEFAULT_SHORT_DRAMA_VIEWPORT,
  SHORT_DRAMA_TRACKS,
  bindManagedPresentationAsset,
  definitionForNode,
  isShortDramaStoryDefinition,
  localizedMessage,
  localizationSourceHash,
  nextCanvasNodePosition,
  nextShortDramaStart,
  addAgentCharacter,
  addPlayerCharacter,
  removeCharacter,
  setLocalizedMessage,
  shortDramaTrack,
  supportedLocales,
  timelineDuration,
  timelineStart,
  nodeInputPorts,
  nodeOutputPorts,
  parseSubtitleCues,
  presentationViewport,
} from "../../lib/game-authoring";
import type {
  AgentProfile,
  GameAsset,
  GameDraft,
  GameNodeDefinition,
  GameResource,
  GameSource,
  GameSourceNode,
  GameTranslationPack,
  ProjectProvider,
  RuntimeProject,
} from "../../lib/runtime-types";
import { runtimeBrowserRequest } from "../../lib/runtime-browser-client";
import { GameNodeInspector } from "./inspector";
import {
  GameAuthoringProvider,
  useGameAuthoring,
  useGameAuthoringStore,
  useGameDraftSync,
} from "./store";
import { GameAuthoringToolbar } from "./toolbar";

const PIXELS_PER_SECOND = 18;
const STAGE_BACKGROUND_TYPES = new Set(["background", "video"]);
const STAGE_CHARACTER_TYPES = new Set(["character_visual"]);
const STAGE_AUDIO_TYPES = new Set(["audio", "voice"]);
const STAGE_SUBTITLE_TYPES = new Set(["subtitle"]);
const STAGE_STORY_TYPES = new Set(["scene", "dialogue", "choice", "player_input", "host_action", "ending", "agent"]);
export function RuntimeShortDrama({
  project,
  draft,
  definitions,
  profiles,
  resources,
  assets,
  providers,
}: {
  project: RuntimeProject;
  draft: GameDraft;
  definitions: GameNodeDefinition[];
  profiles: AgentProfile[];
  resources: GameResource[];
  assets: GameAsset[];
  providers: ProjectProvider[];
}) {
  return (
    <GameAuthoringProvider draft={draft}>
      <ShortDramaWorkspace project={project} definitions={definitions} profiles={profiles} resources={resources} assets={assets} providers={providers} />
    </GameAuthoringProvider>
  );
}

function ShortDramaWorkspace({
  project,
  definitions,
  profiles,
  resources,
  assets,
  providers,
}: {
  project: RuntimeProject;
  definitions: GameNodeDefinition[];
  profiles: AgentProfile[];
  resources: GameResource[];
  assets: GameAsset[];
  providers: ProjectProvider[];
}) {
  useGameDraftSync(project.slug);
  const selectedNodeId = useGameAuthoring((state) => state.selectedNodeId);
  const source = useGameAuthoring((state) => state.source);
  const [locale, setLocale] = useState(source.localization.defaultLocale);
  return (
    <section className={`game-authoring-workspace short-drama-workspace ${selectedNodeId ? "inspector-open" : ""}`}>
      <GameAuthoringToolbar projectSlug={project.slug} viewLabel="Short Drama" />
      <DramaCreatorBar project={project} locale={locale} onLocaleChange={setLocale} profiles={profiles} providers={providers} />
      <div className="short-drama-layout">
        <DramaLibrary definitions={definitions} profiles={profiles} resources={resources} assets={assets} />
        <DramaStage projectSlug={project.slug} locale={locale} assets={assets} />
        <GameNodeInspector definitions={definitions} profiles={profiles} locale={locale} />
        <DramaTimeline definitions={definitions} profiles={profiles} />
      </div>
    </section>
  );
}

const CREATOR_LOCALES = [
  { id: "zh-CN", label: "简体中文" },
  { id: "ja", label: "日本語" },
  { id: "en", label: "English" },
  { id: "ko", label: "한국어" },
  { id: "es", label: "Español" },
  { id: "fr", label: "Français" },
  { id: "de", label: "Deutsch" },
];

function DramaCreatorBar({
  project,
  locale,
  onLocaleChange,
  profiles,
  providers,
}: {
  project: RuntimeProject;
  locale: string;
  onLocaleChange: (locale: string) => void;
  profiles: AgentProfile[];
  providers: ProjectProvider[];
}) {
  const source = useGameAuthoring((state) => state.source);
  const [panel, setPanel] = useState<"cast" | "localization" | null>(null);
  return (
    <>
      <div className="drama-creator-bar">
        <div><Clapperboard aria-hidden="true" /><strong>{source.metadata.name}</strong><span>{source.characters.length} characters</span></div>
        <div>
          <button type="button" onClick={() => setPanel("cast")}><Users aria-hidden="true" />Cast</button>
          <label className="drama-locale-select"><Languages aria-hidden="true" /><span className="sr-only">Editing language</span><select value={locale} onChange={(event) => onLocaleChange(event.target.value)}>{supportedLocales(source).map((item) => <option value={item} key={item}>{localeLabel(item)}</option>)}</select></label>
          <button type="button" onClick={() => setPanel("localization")}><Languages aria-hidden="true" />Translate</button>
        </div>
      </div>
      {panel === "cast" ? <CastDialog locale={locale} profiles={profiles} onClose={() => setPanel(null)} /> : null}
      {panel === "localization" ? <LocalizationDialog projectSlug={project.slug} providers={providers} activeLocale={locale} onLocaleChange={onLocaleChange} onClose={() => setPanel(null)} /> : null}
    </>
  );
}

function CastDialog({ locale, profiles, onClose }: { locale: string; profiles: AgentProfile[]; onClose: () => void }) {
  const source = useGameAuthoring((state) => state.source);
  const setSource = useGameAuthoring((state) => state.setSource);
  const player = source.characters.find((character) => character.player);
  const availableProfiles = profiles.filter((profile) => (
    !profile.archivedAt
    && profile.activeVersionId
    && !source.characters.some((character) => character.agentId === `agent.${profile.slug}`)
  ));
  const [playerName, setPlayerName] = useState(player ? localizedMessage(source, player.nameMessageId, locale) : "");
  const [playerRole, setPlayerRole] = useState(player?.roleMessageId ? localizedMessage(source, player.roleMessageId, locale) : "Player character");
  return (
    <div className="game-modal-overlay" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <section className="creator-dialog" role="dialog" aria-modal="true" aria-labelledby="cast-dialog-title">
        <header><div><span>Story cast</span><h2 id="cast-dialog-title">Characters</h2></div><button type="button" className="icon-button" onClick={onClose} aria-label="Close cast"><X aria-hidden="true" /></button></header>
        <div className="creator-dialog-body cast-dialog-body">
          <section className="player-character-editor">
            <header><div><strong>Player character</strong><span>The role controlled by the player, not an Agent.</span></div>{player ? <Check aria-hidden="true" /> : <Plus aria-hidden="true" />}</header>
            <div><label><span>Name</span><input value={playerName} onChange={(event) => setPlayerName(event.target.value)} placeholder="Player name" /></label><label><span>Role</span><input value={playerRole} onChange={(event) => setPlayerRole(event.target.value)} placeholder="Story role" /></label></div>
            <button type="button" className="secondary-button compact" disabled={!playerName.trim()} onClick={() => setSource(addPlayerCharacter(source, playerName, playerRole))}>{player ? "Update player" : "Add player"}</button>
          </section>
          <div className="cast-roster">
            {source.characters.filter((character) => !character.player).map((character) => (
              <article key={character.id}>
                <span><Bot aria-hidden="true" /></span>
                <div><strong>{localizedMessage(source, character.nameMessageId, locale) || character.id}</strong><small>{character.roleMessageId ? localizedMessage(source, character.roleMessageId, locale) : "Agent character"}</small></div>
                <button type="button" className="icon-button" onClick={() => setSource(removeCharacter(source, character.id))} aria-label={`Remove ${character.id} from cast`}><Trash2 aria-hidden="true" /></button>
              </article>
            ))}
            {source.characters.every((character) => character.player) ? <p>Add the Agents who appear in this story.</p> : null}
          </div>
          {availableProfiles.length > 0 ? (
            <section className="available-cast">
              <header><strong>Available Agents</strong><span>Add a Live Agent to the story cast.</span></header>
              <div className="cast-roster">
                {availableProfiles.map((profile) => (
                  <article key={profile.id}>
                    <span><Bot aria-hidden="true" /></span>
                    <div><strong>{profile.name}</strong><small>{profile.description?.trim() || "Game character"}</small></div>
                    <button type="button" className="icon-button" onClick={() => setSource(addAgentCharacter(source, profile))} aria-label={`Add ${profile.name} to cast`}><Plus aria-hidden="true" /></button>
                  </article>
                ))}
              </div>
            </section>
          ) : null}
        </div>
        <footer><span>{source.characters.length} characters in this draft</span><button type="button" className="primary-button" onClick={onClose}>Done</button></footer>
      </section>
    </div>
  );
}

function LocalizationDialog({
  projectSlug,
  providers,
  activeLocale,
  onLocaleChange,
  onClose,
}: {
  projectSlug: string;
  providers: ProjectProvider[];
  activeLocale: string;
  onLocaleChange: (locale: string) => void;
  onClose: () => void;
}) {
  const source = useGameAuthoring((state) => state.source);
  const setSource = useGameAuthoring((state) => state.setSource);
  const store = useGameAuthoringStore();
  const compatibleProviders = providers.filter((provider) => provider.providerType === "openai-compatible");
  const [sourceLocale, setSourceLocale] = useState(source.localization.sourceLocale);
  const [targets, setTargets] = useState(source.localization.targetLocales);
  const [defaultLocale, setDefaultLocale] = useState(source.localization.defaultLocale);
  const [providerKey, setProviderKey] = useState(compatibleProviders[0]?.providerKey ?? "");
  const selectedProvider = compatibleProviders.find((provider) => provider.providerKey === providerKey);
  const [model, setModel] = useState(providerDefaultModel(selectedProvider));
  const [reviewLocale, setReviewLocale] = useState(activeLocale === sourceLocale ? targets[0] ?? "" : activeLocale);
  const [pending, setPending] = useState<"translate" | "review" | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  function toggleTarget(locale: string) {
    setTargets((current) => current.includes(locale) ? current.filter((item) => item !== locale) : [...current, locale]);
  }

  function applyLanguageSettings() {
    const current = store.getState().source;
    const nextTargets = targets.filter((locale) => locale !== sourceLocale);
    const nextPacks = Object.fromEntries(Object.entries(current.localization.packs).filter(([locale]) => nextTargets.includes(locale)));
    const supported = [sourceLocale, ...nextTargets];
    const nextDefault = supported.includes(defaultLocale) ? defaultLocale : sourceLocale;
    setSource({
      ...current,
      localization: {
        ...current.localization,
        sourceLocale,
        defaultLocale: nextDefault,
        targetLocales: nextTargets,
        packs: nextPacks,
      },
    });
    setDefaultLocale(nextDefault);
    if (!supported.includes(activeLocale)) onLocaleChange(nextDefault);
    setMessage("Language settings saved.");
  }

  async function translate() {
    if (!providerKey || !model.trim() || targets.length === 0) return;
    setPending("translate");
    setMessage(null);
    try {
      const current = store.getState().source;
      const nextTargets = targets.filter((locale) => locale !== sourceLocale);
      const result = await runtimeBrowserRequest<{ sourceHash: string; packs: Record<string, GameTranslationPack> }>(
        `project/${encodeURIComponent(projectSlug)}/game/localization/translate`,
        "POST",
        {
          providerKey,
          model: model.trim(),
          sourceLocale,
          targetLocales: nextTargets,
          messages: current.localization.sourceMessages,
        },
      );
      const next = store.getState().source;
      setSource({
        ...next,
        localization: {
          ...next.localization,
          sourceLocale,
          defaultLocale: [sourceLocale, ...nextTargets].includes(defaultLocale) ? defaultLocale : sourceLocale,
          targetLocales: nextTargets,
          packs: { ...next.localization.packs, ...result.packs },
        },
      });
      const first = nextTargets[0] ?? "";
      setReviewLocale(first);
      if (first) onLocaleChange(first);
      setMessage("Translation drafts are ready for review.");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "Translation failed.");
    } finally {
      setPending(null);
    }
  }

  async function markReviewed() {
    if (!reviewLocale) return;
    setPending("review");
    const current = store.getState().source;
    const pack = current.localization.packs[reviewLocale];
    const complete = pack && Object.keys(current.localization.sourceMessages).every((key) => pack.messages[key]?.trim());
    if (!complete) {
      setMessage("Complete every translated message before marking this language reviewed.");
      setPending(null);
      return;
    }
    const sourceHash = await localizationSourceHash(current.localization.sourceMessages);
    setSource({ ...current, localization: { ...current.localization, packs: { ...current.localization.packs, [reviewLocale]: { ...pack, sourceHash, status: "reviewed" } } } });
    setMessage(`${localeLabel(reviewLocale)} is reviewed and ready to publish.`);
    setPending(null);
  }

  const reviewPack = source.localization.packs[reviewLocale];
  return (
    <div className="game-modal-overlay" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <section className="creator-dialog localization-dialog" role="dialog" aria-modal="true" aria-labelledby="localization-dialog-title">
        <header><div><span>Localization</span><h2 id="localization-dialog-title">Translate the story</h2></div><button type="button" className="icon-button" onClick={onClose} aria-label="Close localization"><X aria-hidden="true" /></button></header>
        <div className="creator-dialog-body localization-dialog-body">
          <section className="language-settings">
            <div><label><span>Source language</span><select value={sourceLocale} onChange={(event) => { setSourceLocale(event.target.value); setTargets((current) => current.filter((locale) => locale !== event.target.value)); }}>{CREATOR_LOCALES.map((item) => <option value={item.id} key={item.id}>{item.label}</option>)}</select></label><label><span>Default player language</span><select value={defaultLocale} onChange={(event) => setDefaultLocale(event.target.value)}>{[sourceLocale, ...targets.filter((target) => target !== sourceLocale)].map((item) => <option value={item} key={item}>{localeLabel(item)}</option>)}</select></label></div>
            <fieldset><legend>Languages to publish</legend>{CREATOR_LOCALES.filter((item) => item.id !== sourceLocale).map((item) => <label key={item.id}><input type="checkbox" checked={targets.includes(item.id)} onChange={() => toggleTarget(item.id)} /><span>{item.label}</span></label>)}</fieldset>
            <button type="button" className="secondary-button compact" onClick={applyLanguageSettings}>Save languages</button>
          </section>
          <section className="translation-provider-settings">
            <header><div><strong>Generate translation drafts</strong><span>Uses a Provider configured for this project.</span></div></header>
            {compatibleProviders.length > 0 ? <div><label><span>Provider</span><select value={providerKey} onChange={(event) => { const key = event.target.value; setProviderKey(key); setModel(providerDefaultModel(compatibleProviders.find((provider) => provider.providerKey === key))); }}>{compatibleProviders.map((provider) => <option value={provider.providerKey} key={provider.id}>{provider.name}</option>)}</select></label><label><span>Model</span><input value={model} onChange={(event) => setModel(event.target.value)} placeholder="Provider model ID" /></label><button type="button" className="primary-button" disabled={pending !== null || !model.trim() || targets.length === 0} onClick={() => void translate()}>{pending === "translate" ? <LoaderCircle className="spin" aria-hidden="true" /> : <Languages aria-hidden="true" />}{pending === "translate" ? "Translating..." : "Translate"}</button></div> : <p>Add an OpenAI-compatible Provider to generate translation drafts. You can still write each language manually from the language menu.</p>}
          </section>
          {targets.length > 0 ? (
            <section className="translation-review">
              <header><div><strong>Review</strong><span>Compare every line before publishing.</span></div><select value={reviewLocale} onChange={(event) => { setReviewLocale(event.target.value); onLocaleChange(event.target.value); }}>{targets.map((target) => <option value={target} key={target}>{localeLabel(target)}</option>)}</select></header>
              {reviewLocale && reviewPack ? <div className="translation-review-list">{Object.entries(source.localization.sourceMessages).map(([messageId, sourceText]) => <article key={messageId}><div><small>{source.localization.sourceLocale}</small><p>{sourceText}</p></div><label><small>{reviewLocale}</small><textarea value={reviewPack.messages[messageId] ?? ""} onChange={(event) => setSource(setLocalizedMessage(source, messageId, reviewLocale, event.target.value))} rows={2} /></label></article>)}</div> : <div className="translation-review-empty">Generate a draft or select the language in the editor to translate it manually.</div>}
              {reviewLocale && reviewPack ? <button type="button" className="secondary-button compact" disabled={pending !== null} onClick={() => void markReviewed()}>{reviewPack.status === "reviewed" ? <Check aria-hidden="true" /> : null}{reviewPack.status === "reviewed" ? "Reviewed" : "Mark reviewed"}</button> : null}
            </section>
          ) : null}
          {message ? <p className="creator-dialog-message" role="status">{message}</p> : null}
        </div>
        <footer><span>{Object.keys(source.localization.sourceMessages).length} translatable lines</span><button type="button" className="primary-button" onClick={onClose}>Done</button></footer>
      </section>
    </div>
  );
}

function providerDefaultModel(provider?: ProjectProvider): string {
  if (!provider) return "";
  for (const key of ["model", "defaultModel", "chatModel"]) {
    const value = provider.config[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return "";
}

function localeLabel(locale: string): string {
  return CREATOR_LOCALES.find((item) => item.id === locale)?.label ?? locale;
}

function DramaLibrary({ definitions, profiles, resources, assets }: { definitions: GameNodeDefinition[]; profiles: AgentProfile[]; resources: GameResource[]; assets: GameAsset[] }) {
  const source = useGameAuthoring((state) => state.source);
  const store = useGameAuthoringStore();
  const [query, setQuery] = useState("");
  const [tab, setTab] = useState<"story" | "agents" | "resources">("story");
  const normalized = query.trim().toLowerCase();
  const timelineDefinitions = definitions.filter((definition) => (
    isShortDramaStoryDefinition(definition, normalized)
  ));
  const agentDefinition = definitions.find((definition) => definition.type === "agent");

  function add(definition: GameNodeDefinition, profile?: AgentProfile, config?: Record<string, unknown>, label?: string) {
    const before = new Set(store.getState().source.graph.nodes.map((node) => node.id));
    store.getState().addNode(
      definition,
      nextCanvasNodePosition(store.getState().source.graph.nodes.length),
      profile,
    );
    const added = store.getState().source.graph.nodes.find((node) => !before.has(node.id));
    if (!added) return;
    if (config || label) {
      store.getState().updateNode({ ...added, label: label ?? added.label, config: { ...added.config, ...config } });
    }
    const current = store.getState().source.graph.nodes.find((node) => node.id === added.id) ?? added;
    const trackId = shortDramaTrack(store.getState().source, current);
    const endMs = nextShortDramaStart(store.getState().source, trackId, current.id);
    store.getState().placeTimelineNode(current.id, trackId, endMs);
    store.getState().setSelectedNode(current.id);
  }

  function addAsset(asset: GameAsset, imageType: "background" | "character_visual" = "background") {
    const version = asset.versions.find((item) => item.approvalStatus === "approved");
    if (!version) return;
    const preferredType = asset.kind === "video"
      ? "video"
      : asset.kind === "audio"
        ? "audio"
        : asset.kind === "subtitle"
          ? "subtitle"
          : asset.kind === "image"
            ? imageType
            : "asset";
    const definition = definitions.find((item) => item.type === preferredType)
      ?? definitions.find((item) => item.type === "asset");
    if (!definition) return;
    add(definition, undefined, {
      logicalResourceId: asset.assetKey,
      kind: asset.kind,
      fit: preferredType === "character_visual" ? "contain" : "cover",
      inMs: 0,
      volume: 1,
      muted: false,
    }, asset.name);
    store.getState().setSource(bindManagedPresentationAsset(store.getState().source, asset, version));
  }

  return (
    <aside className="drama-library">
      <header><Library aria-hidden="true" /><strong>Library</strong></header>
      <div className="drama-library-tabs" role="tablist">
        <button type="button" role="tab" aria-selected={tab === "story"} onClick={() => setTab("story")}>Story</button>
        <button type="button" role="tab" aria-selected={tab === "agents"} onClick={() => setTab("agents")}>Cast</button>
        <button type="button" role="tab" aria-selected={tab === "resources"} onClick={() => setTab("resources")}>Media</button>
      </div>
      <label className="drama-library-search"><Search aria-hidden="true" /><input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder={`Search ${tab}`} /></label>
      <div className="drama-library-list">
        {tab === "story" ? timelineDefinitions.map((definition) => (
          <DramaLibraryItem key={definition.type} icon={libraryIcon(definition.type)} title={definition.title} detail={definition.category} onAdd={() => add(definition)} />
        )) : null}
        {tab === "agents" && agentDefinition ? profiles.filter((profile) => !profile.archivedAt && (!normalized || `${profile.name} ${profile.slug}`.toLowerCase().includes(normalized))).map((profile) => (
          <DramaLibraryItem key={profile.id} icon={Bot} title={profile.name} detail={profile.slug} onAdd={() => add(agentDefinition, profile)} />
        )) : null}
        {tab === "resources" ? (
          <>
            {assets.filter((asset) => asset.versions.some((version) => version.approvalStatus === "approved") && (!normalized || `${asset.name} ${asset.assetKey} ${asset.kind}`.toLowerCase().includes(normalized))).map((asset) => asset.kind === "image" ? (
              <DramaImageLibraryItem
                key={asset.id}
                title={asset.name}
                onAddBackground={() => addAsset(asset, "background")}
                onAddCharacter={() => addAsset(asset, "character_visual")}
              />
            ) : (
              <DramaLibraryItem key={asset.id} icon={libraryIcon(asset.kind)} title={asset.name} detail={asset.kind} onAdd={() => addAsset(asset)} />
            ))}
            {source.presentationResources.map((resource) => <DramaLibraryItem key={resource.id} icon={ImageIcon} title={resource.id} detail="Logical slot" />)}
            {resources.filter((resource) => source.resources.some((reference) => reference.id === resource.resourceKey)).map((resource) => <DramaLibraryItem key={resource.id} icon={Boxes} title={resource.name} detail="Game data" />)}
            {assets.length + source.presentationResources.length + source.resources.length === 0 ? <div className="drama-library-empty"><ImageIcon aria-hidden="true" /><strong>No media yet</strong><span>Import media from the Resources page.</span></div> : null}
          </>
        ) : null}
      </div>
    </aside>
  );
}

function DramaImageLibraryItem({
  title,
  onAddBackground,
  onAddCharacter,
}: {
  title: string;
  onAddBackground: () => void;
  onAddCharacter: () => void;
}) {
  return (
    <div className="drama-library-item drama-image-library-item">
      <span><ImageIcon aria-hidden="true" /></span>
      <div><strong>{title}</strong><small>image</small></div>
      <div className="drama-image-library-actions">
        <button type="button" title="Add as background" aria-label={`Add ${title} as background`} onClick={onAddBackground}><ImageIcon aria-hidden="true" /></button>
        <button type="button" title="Add as character" aria-label={`Add ${title} as character`} onClick={onAddCharacter}><Users aria-hidden="true" /></button>
      </div>
    </div>
  );
}

function DramaLibraryItem({ icon: Icon, title, detail, onAdd }: { icon: LucideIcon; title: string; detail: string; onAdd?: () => void }) {
  return (
    <button type="button" className="drama-library-item" onClick={onAdd} disabled={!onAdd}>
      <span><Icon aria-hidden="true" /></span><div><strong>{title}</strong><small>{detail}</small></div>{onAdd ? <Plus aria-hidden="true" /> : null}
    </button>
  );
}

function DramaStage({ projectSlug, locale, assets }: { projectSlug: string; locale: string; assets: GameAsset[] }) {
  const router = useRouter();
  const source = useGameAuthoring((state) => state.source);
  const selectedNodeId = useGameAuthoring((state) => state.selectedNodeId);
  const selected = source.graph.nodes.find((node) => node.id === selectedNodeId)
    ?? source.graph.nodes.find((node) => node.type === "scene")
    ?? source.graph.nodes[0];
  const choiceCount = source.graph.nodes.filter((node) => node.type === "choice").length;
  const endingCount = source.graph.nodes.filter((node) => node.type === "ending").length;
  const viewport = presentationViewport(source, DEFAULT_SHORT_DRAMA_VIEWPORT);
  const orientation = viewport.width === viewport.height
    ? "square"
    : viewport.width < viewport.height
      ? "portrait"
      : "landscape";
  const frameStyle = {
    "--game-frame-ratio": `${viewport.width} / ${viewport.height}`,
    "--game-frame-ratio-value": viewport.width / viewport.height,
  } as CSSProperties;
  const selectedStartMs = selected ? timelineStart(selected) : 0;
  const durationMs = Math.max(1, dramaDuration(source.graph.nodes));
  const [playheadMs, setPlayheadMs] = useState(selectedStartMs);
  const [isPlaying, setIsPlaying] = useState(false);
  const playbackFrame = useRef<number | null>(null);
  const playbackOrigin = useRef(selectedStartMs);
  const [subtitleCues, setSubtitleCues] = useState<ReturnType<typeof parseSubtitleCues>>([]);

  useEffect(() => {
    setIsPlaying(false);
    setPlayheadMs(selectedStartMs);
    playbackOrigin.current = selectedStartMs;
  }, [selectedStartMs]);

  useEffect(() => {
    if (!isPlaying) return;
    const originMs = playbackOrigin.current;
    const startedAt = performance.now();
    setPlayheadMs(originMs);

    const tick = (now: number) => {
      const next = Math.min(durationMs, originMs + now - startedAt);
      setPlayheadMs(next);
      if (next >= durationMs) {
        setIsPlaying(false);
        playbackFrame.current = null;
        return;
      }
      playbackFrame.current = window.requestAnimationFrame(tick);
    };
    playbackFrame.current = window.requestAnimationFrame(tick);
    return () => {
      if (playbackFrame.current !== null) window.cancelAnimationFrame(playbackFrame.current);
      playbackFrame.current = null;
    };
  }, [durationMs, isPlaying]);

  const activeBackground = activeNodesAt(source, playheadMs, STAGE_BACKGROUND_TYPES).at(-1);
  const activeCharacters = activeNodesAt(source, playheadMs, STAGE_CHARACTER_TYPES);
  const activeAudio = isPlaying ? activeNodesAt(source, playheadMs, STAGE_AUDIO_TYPES) : [];
  const activeSubtitle = activeNodesAt(source, playheadMs, STAGE_SUBTITLE_TYPES).at(-1);
  const activeStory = activeNodesAt(source, playheadMs, STAGE_STORY_TYPES).at(-1);
  const selectedStory = selected && STAGE_STORY_TYPES.has(selected.type) ? selected : undefined;
  const stageNode = isPlaying ? activeStory ?? selectedStory : selectedStory;
  const backgroundUrl = activeBackground ? stageAssetUrl(projectSlug, source, assets, activeBackground) : null;
  const characterUrls = activeCharacters.flatMap((node) => {
    const url = stageAssetUrl(projectSlug, source, assets, node);
    return url ? [{ id: node.id, url }] : [];
  });
  const audioUrls = activeAudio.flatMap((node) => {
    const url = stageAssetUrl(projectSlug, source, assets, node);
    return url ? [{ node, url }] : [];
  });
  const subtitleUrl = activeSubtitle ? stageAssetUrl(projectSlug, source, assets, activeSubtitle) : null;
  const subtitleOffsetMs = activeSubtitle ? playheadMs - timelineStart(activeSubtitle) : 0;
  const subtitleText = subtitleCues.find((cue) => cue.startMs <= subtitleOffsetMs && subtitleOffsetMs < cue.endMs)?.text
    ?? (activeSubtitle ? inlineSubtitle(source, activeSubtitle, locale) : null);

  useEffect(() => {
    if (!subtitleUrl) {
      setSubtitleCues([]);
      return;
    }
    let cancelled = false;
    void fetch(subtitleUrl)
      .then((response) => {
        if (!response.ok) throw new Error(`Subtitle request failed with ${response.status}`);
        return response.text();
      })
      .then((content) => {
        if (!cancelled) setSubtitleCues(parseSubtitleCues(content));
      })
      .catch(() => {
        if (!cancelled) setSubtitleCues([]);
      });
    return () => {
      cancelled = true;
    };
  }, [subtitleUrl]);

  function seek(event: MouseEvent<HTMLDivElement>) {
    const rect = event.currentTarget.getBoundingClientRect();
    const ratio = rect.width > 0 ? (event.clientX - rect.left) / rect.width : 0;
    setIsPlaying(false);
    const next = Math.max(0, Math.min(durationMs, ratio * durationMs));
    setPlayheadMs(next);
    playbackOrigin.current = next;
  }

  function togglePlayback() {
    if (isPlaying) {
      playbackOrigin.current = playheadMs;
      setIsPlaying(false);
      return;
    }
    playbackOrigin.current = playheadMs >= durationMs ? 0 : playheadMs;
    setIsPlaying(true);
  }

  return (
    <main className="drama-stage-area">
      <div className="drama-stage-meta"><span>Sequence <strong>main</strong></span><span><strong>{viewport.aspectRatio}</strong> · {viewport.width} × {viewport.height}</span><span>{choiceCount} branches</span><span>{endingCount} endings</span></div>
      <div className="drama-player-viewport">
        <div className={`drama-player-frame ${orientation}`} style={frameStyle}>
          <div className="drama-player-scene">
            <div className="drama-stage-media" aria-hidden="true">
              {backgroundUrl && activeBackground?.type === "video" ? (
                <TimelineVideo
                  key={activeBackground.id}
                  node={activeBackground}
                  playheadMs={playheadMs}
                  playing={isPlaying}
                  url={backgroundUrl}
                />
              ) : backgroundUrl ? (
                <img
                  className="drama-stage-background"
                  src={backgroundUrl}
                  alt=""
                  style={{ objectFit: activeBackground?.config.fit === "contain" ? "contain" : "cover" }}
                />
              ) : <div className="drama-stage-fallback" />}
              <div className="drama-stage-characters">
                {characterUrls.map((character, index) => (
                  <img
                    className="drama-stage-character"
                    src={character.url}
                    alt=""
                    key={character.id}
                    style={{
                      "--character-index": index,
                      "--character-count": characterUrls.length,
                    } as CSSProperties}
                  />
                ))}
              </div>
              <div className="drama-stage-scrim" />
              {audioUrls.map(({ node, url }) => <TimelineAudio key={node.id} node={node} playheadMs={playheadMs} url={url} />)}
            </div>
            {stageNode ? <div className="drama-stage-copy">
              <span className="drama-scene-kicker">{stageNode?.type ?? "scene"}</span>
              <strong>{stageNode?.label || source.metadata.name}</strong>
              <p>{stageNode ? stageDescription(source, stageNode, locale) : "Add a scene or Agent beat to begin."}</p>
              {stageNode?.type === "choice" ? <ChoicePreview source={source} node={stageNode} locale={locale} /> : null}
            </div> : null}
            {subtitleText ? <div className="drama-stage-subtitle"><span>{subtitleText}</span></div> : null}
          </div>
          <div className="drama-player-controls">
            <button type="button" className="primary" aria-label={isPlaying ? "Pause timeline" : "Play timeline"} title={isPlaying ? "Pause timeline" : "Play timeline"} onClick={togglePlayback}>{isPlaying ? <Pause aria-hidden="true" /> : <Play aria-hidden="true" />}</button>
            <button type="button" aria-label="Open live preview" title="Open live preview" onClick={() => router.push(`/project/${projectSlug}/preview`)}><ExternalLink aria-hidden="true" /></button>
            <span>{formatTime(playheadMs)}</span><div role="slider" aria-label="Timeline playhead" aria-valuemin={0} aria-valuemax={durationMs} aria-valuenow={Math.round(playheadMs)} onClick={seek}><i style={{ width: `${Math.min(100, (playheadMs / durationMs) * 100)}%` }} /></div><span>{formatTime(durationMs)}</span>
          </div>
        </div>
      </div>
      <BranchNavigator nodes={source.graph.nodes} />
    </main>
  );
}

function TimelineVideo({
  node,
  playheadMs,
  playing,
  url,
}: {
  node: GameSourceNode;
  playheadMs: number;
  playing: boolean;
  url: string;
}) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const loop = node.config.loop === true;
  const muted = node.config.muted === true;
  const volume = typeof node.config.volume === "number" ? node.config.volume : 1;
  const fit = node.config.fit === "contain" ? "contain" : "cover";

  function mediaTimeSeconds() {
    const inMs = typeof node.config.inMs === "number" ? node.config.inMs : 0;
    return Math.max(0, inMs + playheadMs - timelineStart(node)) / 1000;
  }

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    video.volume = Math.max(0, Math.min(1, volume));
  }, [volume]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const target = mediaTimeSeconds();
    if (Math.abs(video.currentTime - target) > 0.25) video.currentTime = target;
    if (playing) void video.play().catch(() => undefined);
    else video.pause();
    return () => video.pause();
  }, [playing, url]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || playing) return;
    const target = mediaTimeSeconds();
    if (Math.abs(video.currentTime - target) > 0.04) video.currentTime = target;
  }, [playheadMs, playing]);

  return (
    <video
      className="drama-stage-background"
      ref={videoRef}
      src={url}
      loop={loop}
      muted={muted}
      playsInline
      preload="auto"
      style={{ objectFit: fit }}
    />
  );
}

function TimelineAudio({ node, playheadMs, url }: { node: GameSourceNode; playheadMs: number; url: string }) {
  const audioRef = useRef<HTMLAudioElement>(null);
  const initialOffsetMs = useRef(Math.max(0, playheadMs - timelineStart(node)));
  const loop = node.config.loop === true;
  const muted = node.config.muted === true;
  const volume = typeof node.config.volume === "number" ? node.config.volume : 1;

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.volume = Math.max(0, Math.min(1, volume));
    audio.currentTime = initialOffsetMs.current / 1000;
    void audio.play().catch(() => undefined);
    return () => audio.pause();
  }, [url, volume]);

  return <audio className="drama-stage-audio" ref={audioRef} loop={loop} muted={muted} src={url} />;
}

function activeNodesAt(source: GameSource, playheadMs: number, types: ReadonlySet<string>): GameSourceNode[] {
  return source.graph.nodes
    .filter((node) => types.has(node.type))
    .filter((node) => timelineStart(node) <= playheadMs && playheadMs < timelineStart(node) + timelineDuration(node))
    .sort((left, right) => timelineStart(left) - timelineStart(right));
}

function stageAssetUrl(projectSlug: string, source: GameSource, assets: GameAsset[], node: GameSourceNode): string | null {
  const logicalResourceId = typeof node.config.logicalResourceId === "string" ? node.config.logicalResourceId : "";
  if (!logicalResourceId) return null;
  const presentation = source.views.presentation;
  if (!presentation || typeof presentation !== "object" || Array.isArray(presentation)) return null;
  const bindings = (presentation as Record<string, unknown>).bindings;
  if (!bindings || typeof bindings !== "object" || Array.isArray(bindings)) return null;
  const binding = (bindings as Record<string, unknown>)[logicalResourceId];
  if (!binding || typeof binding !== "object" || Array.isArray(binding)) return null;
  const value = binding as Record<string, unknown>;
  const versionId = value.kind === "managed-asset-version" && typeof value.value === "string" ? value.value : "";
  if (!versionId) return null;
  const asset = assets.find((item) => item.versions.some((version) => version.id === versionId));
  if (!asset) return null;
  return `/api/runtime/project/${encodeURIComponent(projectSlug)}/game/assets/${encodeURIComponent(asset.id)}/versions/${encodeURIComponent(versionId)}/content`;
}

function DramaTimeline({ definitions, profiles }: { definitions: GameNodeDefinition[]; profiles: AgentProfile[] }) {
  const source = useGameAuthoring((state) => state.source);
  const reorderTimeline = useGameAuthoring((state) => state.reorderTimeline);
  const placeTimelineNode = useGameAuthoring((state) => state.placeTimelineNode);
  const setSelectedNode = useGameAuthoring((state) => state.setSelectedNode);
  const selectedNodeId = useGameAuthoring((state) => state.selectedNodeId);
  const splitTimelineNode = useGameAuthoring((state) => state.splitTimelineNode);
  const timelineNodes = source.graph.nodes.filter((node) => definitionForNode(definitions, node)?.timelineCompatible);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  const maxDuration = Math.max(30_000, dramaDuration(timelineNodes));
  const timelineWidth = Math.max(760, (maxDuration / 1000) * PIXELS_PER_SECOND + 80);
  const selected = timelineNodes.find((node) => node.id === selectedNodeId);
  const selectedDefinition = selected ? definitionForNode(definitions, selected) : undefined;

  function splitSelected() {
    if (!selected) return;
    const outputPort = nodeOutputPorts(selectedDefinition, selected)[0];
    const inputPort = nodeInputPorts(selectedDefinition)[0];
    if (!outputPort || !inputPort) return;
    splitTimelineNode(selected.id, outputPort, inputPort);
  }

  function onDragEnd(event: DragEndEvent) {
    const activeId = String(event.active.id);
    const overId = event.over ? String(event.over.id) : null;
    if (!overId || activeId === overId) return;
    const active = timelineNodes.find((node) => node.id === activeId);
    const over = timelineNodes.find((node) => node.id === overId);
    if (!active || !over) return;
    const activeTrack = shortDramaTrack(source, active);
    const targetTrack = shortDramaTrack(source, over);
    const targetNodes = timelineNodes
      .filter((node) => shortDramaTrack(source, node) === targetTrack)
      .sort((left, right) => timelineStart(left) - timelineStart(right));
    const oldIndex = targetNodes.findIndex((node) => node.id === activeId);
    const overIndex = targetNodes.findIndex((node) => node.id === overId);
    if (activeTrack === targetTrack && oldIndex >= 0 && overIndex >= 0) {
      reorderTimeline(targetTrack, arrayMove(targetNodes.map((node) => node.id), oldIndex, overIndex));
      return;
    }
    const insertionIndex = Math.max(0, overIndex);
    const nextTargetIds = targetNodes.map((node) => node.id);
    nextTargetIds.splice(insertionIndex, 0, activeId);
    placeTimelineNode(activeId, targetTrack, timelineStart(over));
    reorderTimeline(targetTrack, nextTargetIds);
    const oldTrackIds = timelineNodes
      .filter((node) => node.id !== activeId && shortDramaTrack(source, node) === activeTrack)
      .sort((left, right) => timelineStart(left) - timelineStart(right))
      .map((node) => node.id);
    reorderTimeline(activeTrack, oldTrackIds);
  }

  return (
    <section className="drama-timeline">
      <header><div><Scissors aria-hidden="true" /><strong>Timeline</strong><span>{timelineNodes.length} items</span></div><div><button type="button" title="Split selected" aria-label="Split selected" disabled={!selected || timelineDuration(selected) < 500} onClick={splitSelected}><Scissors aria-hidden="true" /></button><button type="button" title="Timeline settings" aria-label="Timeline settings"><Sparkles aria-hidden="true" /></button></div></header>
      <div className="timeline-scroll">
        <div className="timeline-ruler-row"><span className="timeline-track-label">Tracks</span><div className="timeline-ruler" style={{ width: timelineWidth }}>{timelineTicks(maxDuration).map((tick) => <span key={tick} style={{ left: (tick / 1000) * PIXELS_PER_SECOND }}>{formatTime(tick)}</span>)}</div></div>
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
          {SHORT_DRAMA_TRACKS.map((track) => {
            const nodes = timelineNodes.filter((node) => shortDramaTrack(source, node) === track.id).sort((left, right) => timelineStart(left) - timelineStart(right));
            const laneLayout = timelineLaneLayout(nodes);
            const trackHeight = Math.max(38, laneLayout.count * 35 + 3);
            return (
              <div className="timeline-track" key={track.id} style={{ minHeight: trackHeight }}>
                <div className="timeline-track-label"><TrackIcon kind={track.kind} /><span>{track.label}</span></div>
                <SortableContext items={nodes.map((node) => node.id)} strategy={horizontalListSortingStrategy}>
                  <div className="timeline-track-lane" style={{ width: timelineWidth, minHeight: trackHeight }}>
                    {nodes.map((node) => <SortableTimelineClip key={node.id} lane={laneLayout.byNode.get(node.id) ?? 0} node={node} selected={selectedNodeId === node.id} profile={profileForNode(node, source.agents, profiles)} onSelect={() => setSelectedNode(node.id)} />)}
                  </div>
                </SortableContext>
              </div>
            );
          })}
        </DndContext>
      </div>
    </section>
  );
}

function SortableTimelineClip({ node, lane, selected, profile, onSelect }: { node: GameSourceNode; lane: number; selected: boolean; profile?: AgentProfile; onSelect: () => void }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: node.id });
  const width = Math.max(74, (timelineDuration(node) / 1000) * PIXELS_PER_SECOND);
  const left = (timelineStart(node) / 1000) * PIXELS_PER_SECOND;
  return (
    <button
      type="button"
      ref={setNodeRef}
      className={`timeline-clip clip-${clipKind(node.type)} ${selected ? "selected" : ""} ${isDragging ? "dragging" : ""}`}
      style={{ width, left, top: 3 + lane * 35, transform: CSS.Transform.toString(transform), transition }}
      {...attributes}
      {...listeners}
      onClick={onSelect}
      onPointerUp={onSelect}
    >
      <span>{profile ? <Bot aria-hidden="true" /> : <ClipIcon type={node.type} />}</span>
      <div><strong>{node.label || profile?.name || node.type}</strong><small>{formatTime(timelineDuration(node))}</small></div>
    </button>
  );
}

function timelineLaneLayout(nodes: GameSourceNode[]): { byNode: Map<string, number>; count: number } {
  const laneEnds: number[] = [];
  const byNode = new Map<string, number>();
  for (const node of nodes) {
    const start = timelineStart(node);
    let lane = laneEnds.findIndex((end) => end <= start);
    if (lane < 0) lane = laneEnds.length;
    laneEnds[lane] = start + timelineDuration(node);
    byNode.set(node.id, lane);
  }
  return { byNode, count: Math.max(1, laneEnds.length) };
}

function BranchNavigator({ nodes }: { nodes: GameSourceNode[] }) {
  const choices = nodes.filter((node) => node.type === "choice");
  const setSelectedNode = useGameAuthoring((state) => state.setSelectedNode);
  return (
    <section className="drama-branch-navigator">
      <header><ListTree aria-hidden="true" /><strong>Branches</strong></header>
      {choices.length > 0 ? choices.map((choice) => (
        <button type="button" key={choice.id} onClick={() => setSelectedNode(choice.id)}><GitBranch aria-hidden="true" /><span><strong>{choice.label || "Choice"}</strong><small>{Array.isArray(choice.config.options) ? `${choice.config.options.length} paths` : "Configure paths"}</small></span><ChevronRight aria-hidden="true" /></button>
      )) : <p>No branches in this sequence.</p>}
    </section>
  );
}

function ChoicePreview({ source, node, locale }: { source: GameSource; node: GameSourceNode; locale: string }) {
  const options = Array.isArray(node.config.options) ? node.config.options : [];
  return <div className="drama-choice-preview">{options.map((option, index) => {
    const value = option && typeof option === "object" ? option as Record<string, unknown> : {};
    const reference = value.label && typeof value.label === "object" && !Array.isArray(value.label) && "$message" in value.label
      ? String((value.label as { $message: unknown }).$message)
      : null;
    return <span key={String(value.id ?? index)}>{reference ? localizedMessage(source, reference, locale) : String(value.label ?? value.id ?? `Option ${index + 1}`)}</span>;
  })}</div>;
}

function TrackIcon({ kind }: { kind: string }) {
  const Icon = kind === "agent" ? Bot : kind === "interaction" ? GitBranch : kind === "media" ? ImageIcon : kind === "scene" ? Clapperboard : Sparkles;
  return <Icon aria-hidden="true" />;
}

function ClipIcon({ type }: { type: string }) {
  const Icon = libraryIcon(type);
  return <Icon aria-hidden="true" />;
}

function libraryIcon(type: string): LucideIcon {
  if (type === "agent") return Bot;
  if (["choice", "condition", "input", "event"].includes(type)) return GitBranch;
  if (["image", "video", "background", "character_visual", "asset"].includes(type)) return ImageIcon;
  if (["audio", "voice", "subtitle"].includes(type)) return Volume2;
  if (["scene", "episode", "dialogue", "ending"].includes(type)) return Clapperboard;
  return Sparkles;
}

function clipKind(type: string): string {
  if (type === "agent") return "agent";
  if (["choice", "condition", "input", "event"].includes(type)) return "interaction";
  if (["video", "background", "character_visual", "asset"].includes(type)) return "visual";
  if (["audio", "voice", "subtitle"].includes(type)) return "audio";
  return "story";
}

function profileForNode(
  node: GameSourceNode,
  agentReferences: Array<{ id: string; profileId: string }>,
  profiles: AgentProfile[],
): AgentProfile | undefined {
  const agentId = typeof node.config.agentId === "string" ? node.config.agentId : "";
  const reference = agentReferences.find((agent) => agent.id === agentId);
  return reference ? profiles.find((profile) => profile.id === reference.profileId) : undefined;
}

function stageDescription(source: GameSource, node: GameSourceNode, locale: string): string {
  for (const key of ["prompt", "text", "description", "title", "action"]) {
    const value = node.config[key];
    if (typeof value === "string" && value) return value;
    if (value && typeof value === "object" && !Array.isArray(value) && typeof (value as { $message?: unknown }).$message === "string") {
      return localizedMessage(source, String((value as { $message: string }).$message), locale);
    }
  }
  return `Configure this ${node.type.replace(/_/g, " ")} in the inspector.`;
}

function inlineSubtitle(source: GameSource, node: GameSourceNode, locale: string): string | null {
  const value = node.config.text;
  if (typeof value === "string") return value || null;
  if (value && typeof value === "object" && !Array.isArray(value) && typeof (value as { $message?: unknown }).$message === "string") {
    return localizedMessage(source, String((value as { $message: string }).$message), locale) || null;
  }
  return null;
}

function dramaDuration(nodes: GameSourceNode[]): number {
  return nodes.reduce((maximum, node) => Math.max(maximum, timelineStart(node) + timelineDuration(node)), 0);
}

function timelineTicks(durationMs: number): number[] {
  const ticks: number[] = [];
  for (let tick = 0; tick <= durationMs; tick += 5000) ticks.push(tick);
  return ticks;
}

function formatTime(valueMs: number): string {
  const seconds = Math.max(0, valueMs) / 1000;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${remainder.toFixed(remainder % 1 ? 1 : 0).padStart(2, "0")}`;
}
