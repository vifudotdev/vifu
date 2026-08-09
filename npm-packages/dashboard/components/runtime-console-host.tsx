"use client";

import Image from "next/image";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMemo, type ReactNode } from "react";
import {
  RuntimeConsole as SharedRuntimeConsole,
  RuntimeConsoleHostProvider,
  type DashboardSection,
  type RuntimeConsoleHost,
  type RuntimeConsoleProps,
} from "@vifu/console/react";

export type { DashboardSection };

export function RuntimeConsole(props: RuntimeConsoleProps) {
  return (
    <NextRuntimeConsoleHost>
      <SharedRuntimeConsole {...props} />
    </NextRuntimeConsoleHost>
  );
}

function NextRuntimeConsoleHost({ children }: { children: ReactNode }) {
  const router = useRouter();
  const host = useMemo<Partial<RuntimeConsoleHost>>(() => ({
    Link: Link as unknown as RuntimeConsoleHost["Link"],
    Image: Image as unknown as RuntimeConsoleHost["Image"],
    router: {
      push: (href) => router.push(href),
      refresh: () => router.refresh(),
    },
    projectRootHref: () => "/project",
    projectHref: (projectSlug) => `/project/${encodeURIComponent(projectSlug)}`,
    projectSectionHref: (projectSlug, section) => `/project/${encodeURIComponent(projectSlug)}/${encodeURIComponent(section)}`,
    logoutAction: "/auth/logout",
    brand: {
      label: "Vifu Console",
      lockupSrc: "/brand/vifu-lockup.png",
      iconSrc: "/brand/vifu-icon-512.png",
    },
  }), [router]);

  return <RuntimeConsoleHostProvider value={host}>{children}</RuntimeConsoleHostProvider>;
}
