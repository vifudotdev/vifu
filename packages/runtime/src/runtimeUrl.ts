export const RUNTIME_GAME_ID_PARAM = "runtime_game_id";
export const RUNTIME_DATA_VERSION_PARAM = "runtime_data_version";
export const RUNTIME_DATA_TOKEN_PARAM = "runtime_data_token";
export const RUNTIME_DATA_BASE_PARAM = "runtime_data_base";
export const RUNTIME_MEDIA_BASE_PARAM = "runtime_media_base";

export interface RuntimeUrlParamsOptions {
  autoplay?: boolean;
  cacheBust?: string | number;
  runtimeDataBaseUrl?: string | null;
  runtimeMediaBaseUrl?: string | null;
  extraParams?: Record<string, string | number | boolean | null | undefined>;
}

export const RUNTIME_IFRAME_ALLOW = "autoplay; fullscreen; cross-origin-isolated; microphone; camera";
export const RUNTIME_IFRAME_SANDBOX = "allow-scripts allow-pointer-lock allow-presentation";
export const RUNTIME_IFRAME_SCROLLING = "no";

export function buildRuntimeCacheBust(gameId: string, revision?: string | number): string {
  if (typeof revision === "undefined" || revision === "" || revision === 0) return gameId;
  return `${gameId}-${revision}`;
}

export function withRuntimeIframeParams(urlString: string, gameId: string, options: RuntimeUrlParamsOptions = {}): string {
  if (!urlString) return urlString;
  const autoplay = options.autoplay ?? true;
  const cacheBust = options.cacheBust ?? gameId;
  const params: Record<string, string | number | boolean | null | undefined> = {
    autoplay: autoplay ? "1" : undefined,
    runtime_cache_bust: cacheBust,
    ...options.extraParams,
    [RUNTIME_DATA_BASE_PARAM]: options.runtimeDataBaseUrl ?? undefined,
    [RUNTIME_MEDIA_BASE_PARAM]: options.runtimeMediaBaseUrl ?? undefined,
  };

  try {
    const url = new URL(urlString);
    for (const [key, value] of Object.entries(params)) {
      if (value === null) {
        url.searchParams.delete(key);
        continue;
      }
      if (typeof value === "undefined") continue;
      url.searchParams.set(key, String(value));
    }
    return url.toString();
  } catch {
    const hashIndex = urlString.indexOf("#");
    const withoutHash = hashIndex >= 0 ? urlString.slice(0, hashIndex) : urlString;
    const hash = hashIndex >= 0 ? urlString.slice(hashIndex) : "";
    const queryIndex = withoutHash.indexOf("?");
    const path = queryIndex >= 0 ? withoutHash.slice(0, queryIndex) : withoutHash;
    const search = queryIndex >= 0 ? withoutHash.slice(queryIndex + 1) : "";
    const extra = new URLSearchParams(search);
    for (const [key, value] of Object.entries(params)) {
      if (value === null) {
        extra.delete(key);
        continue;
      }
      if (typeof value === "undefined") continue;
      extra.set(key, String(value));
    }
    const query = extra.toString();
    return `${path}${query ? `?${query}` : ""}${hash}`;
  }
}
