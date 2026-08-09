"use client";

import { createContext, useContext, type AnchorHTMLAttributes, type ComponentType, type ImgHTMLAttributes, type ReactNode } from "react";
import {
  runtimeBrowserRequest,
  runtimeBrowserUpload,
  type RuntimeBrowserRequest,
  type RuntimeBrowserUpload,
} from "./browser-client";

export type RuntimeConsoleLinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string;
  prefetch?: boolean;
};

export type RuntimeConsoleImageProps = ImgHTMLAttributes<HTMLImageElement> & {
  src: string;
  width?: number;
  height?: number;
  priority?: boolean;
};

export type RuntimeConsoleRouter = {
  push: (href: string) => void;
  refresh: () => void;
};

export type RuntimeConsoleHost = {
  Link: ComponentType<RuntimeConsoleLinkProps>;
  Image: ComponentType<RuntimeConsoleImageProps>;
  router: RuntimeConsoleRouter;
  request: RuntimeBrowserRequest;
  upload: RuntimeBrowserUpload;
  projectRootHref: () => string;
  projectHref: (projectSlug: string) => string;
  projectSectionHref: (projectSlug: string, section: string) => string;
  logoutAction?: string;
  brand?: {
    label: string;
    lockupSrc?: string;
    iconSrc?: string;
  };
};

export type RuntimeConsoleHostProviderProps = {
  value: Partial<RuntimeConsoleHost>;
  children: ReactNode;
};

const defaultHost: RuntimeConsoleHost = {
  Link: DefaultLink,
  Image: DefaultImage,
  router: {
    push(href) {
      if (typeof window !== "undefined") window.location.assign(href);
    },
    refresh() {
      if (typeof window !== "undefined") window.location.reload();
    },
  },
  request: runtimeBrowserRequest,
  upload: runtimeBrowserUpload,
  projectRootHref: () => "/project",
  projectHref: (projectSlug) => `/project/${encodeURIComponent(projectSlug)}`,
  projectSectionHref: (projectSlug, section) => `/project/${encodeURIComponent(projectSlug)}/${encodeURIComponent(section)}`,
  logoutAction: "/auth/logout",
  brand: {
    label: "Vifu Console",
    lockupSrc: "/brand/vifu-lockup.png",
    iconSrc: "/brand/vifu-icon-512.png",
  },
};

const HostContext = createContext<RuntimeConsoleHost>(defaultHost);

export function RuntimeConsoleHostProvider({ value, children }: RuntimeConsoleHostProviderProps) {
  return (
    <HostContext.Provider value={{ ...defaultHost, ...value }}>
      {children}
    </HostContext.Provider>
  );
}

export function useRuntimeConsoleHost(): RuntimeConsoleHost {
  return useContext(HostContext);
}

export function useRuntimeConsoleRouter(): RuntimeConsoleRouter {
  return useRuntimeConsoleHost().router;
}

export function RuntimeLink(props: RuntimeConsoleLinkProps) {
  const Link = useRuntimeConsoleHost().Link;
  return <Link {...props} />;
}

export function RuntimeImage(props: RuntimeConsoleImageProps) {
  const Image = useRuntimeConsoleHost().Image;
  return <Image {...props} />;
}

function DefaultLink({ prefetch: _prefetch, ...props }: RuntimeConsoleLinkProps) {
  return <a {...props} />;
}

function DefaultImage({ priority: _priority, ...props }: RuntimeConsoleImageProps) {
  return <img {...props} />;
}
