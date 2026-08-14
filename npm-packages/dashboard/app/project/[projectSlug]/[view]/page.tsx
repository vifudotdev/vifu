import { redirect } from "next/navigation";

export default async function ProjectViewPage({ params }: {
  params: Promise<{ projectSlug: string; view: string }>;
}) {
  const { projectSlug, view } = await params;
  redirect(`/apps/${encodeURIComponent(projectSlug)}/${encodeURIComponent(view)}`);
}
