export function isAllowedProjectGamePath(path: string[]): boolean {
  if (path[0] !== "project" || path[2] !== "game") return false;
  if (path.length === 3) return true;
  switch (path[3]) {
    case "source":
      return path.length === 4
        || (path.length === 5 && ["export", "import"].includes(path[4] ?? ""));
    case "releases":
      return path.length === 4
        || (path.length === 6 && path[5] === "activate");
    case "resources":
    case "sessions":
      return path.length === 4 || path.length === 5;
    case "assets":
      return path.length === 4
        || path.length === 5
        || (path.length === 6 && path[5] === "versions")
        || (path.length === 8 && path[5] === "versions" && path[7] === "approve");
    case "builds":
      return path.length === 4
        || path.length === 5
        || (path.length === 6 && path[5] === "cancel");
    case "presentations":
      return path.length === 4
        || (path.length === 6 && path[5] === "activate");
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
