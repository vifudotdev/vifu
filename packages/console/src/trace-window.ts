export type TraceDateWindow = Readonly<{
  from: string | null;
  to: string | null;
}>;

export function traceDateWindowChanged(
  previous: TraceDateWindow,
  next: TraceDateWindow,
): boolean {
  return previous.from !== next.from || previous.to !== next.to;
}
