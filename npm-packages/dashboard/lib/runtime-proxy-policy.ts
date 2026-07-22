export function isAllowedProjectGamePath(path: string[]): boolean {
  if (path[0] !== "project" || path[2] !== "game") return false;
  if (path.length === 3) return true;
  switch (path[3]) {
    case "source":
      return path.length === 4
        || (path.length === 5 && ["export", "import"].includes(path[4] ?? ""));
    case "releases":
      return path.length === 4
        || (path.length === 6 && ["activate", "export"].includes(path[5] ?? ""));
    case "resources":
    case "sessions":
      return path.length === 4 || path.length === 5;
    case "assets":
      return path.length === 4
        || path.length === 5
        || (path.length === 6 && path[5] === "versions")
        || (path.length === 8 && path[5] === "versions" && ["approve", "content"].includes(path[7] ?? ""));
    case "builds":
      return path.length === 4
        || path.length === 5
        || (path.length === 6 && path[5] === "cancel");
    case "presentations":
      return path.length === 4
        || (path.length === 6 && path[5] === "activate");
    case "localization":
      return path.length === 5 && path[4] === "translate";
    case "analytics":
    case "preview":
    case "publish":
    case "qa":
    case "validate":
      return path.length === 4;
    default:
      return false;
  }
}
