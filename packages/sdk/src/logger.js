export function createLogger(rawLogger) {
  const fallback = () => {};
  return {
    debug: typeof rawLogger?.debug === "function" ? rawLogger.debug.bind(rawLogger) : fallback,
    info: typeof rawLogger?.info === "function" ? rawLogger.info.bind(rawLogger) : fallback,
    warn: typeof rawLogger?.warn === "function" ? rawLogger.warn.bind(rawLogger) : fallback,
    error: typeof rawLogger?.error === "function" ? rawLogger.error.bind(rawLogger) : fallback,
  };
}

export function logSdk(logger, level, message, details) {
  try {
    logger[level]?.(message, details);
  } catch {
    // Logging must never break the runtime.
  }
}
