import { redirect } from "next/navigation";

export default async function ProjectHomePage({ params }: {
  params: Promise<{ projectSlug: string }>;
}) {
  const { projectSlug } = await params;
  redirect(`/apps/${encodeURIComponent(projectSlug)}`);
}
