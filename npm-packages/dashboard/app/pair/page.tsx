import { resolveAuthority } from "../../lib/authority";
import { PairGateway } from "../../components/pair-gateway";

export const dynamic = "force-dynamic";

export default async function PairGatewayPage({ searchParams }: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const requestId = readParam((await searchParams).request);
  const returnTo = requestId ? `/pair?request=${encodeURIComponent(requestId)}` : "/pair";
  await resolveAuthority({ returnTo });
  return <PairGateway requestId={requestId} />;
}

function readParam(value: string | string[] | undefined): string | null {
  return Array.isArray(value) ? value[0] ?? null : value ?? null;
}
