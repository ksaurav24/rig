import type { ReactNode } from 'react';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { source } from '@/lib/source';
import { RigLogo } from '@/components/rig-logo';
import { AnimatedThemeToggler } from '@/components/theme-toggler';
import { site } from '@/lib/site';

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <DocsLayout
      tree={source.getPageTree()}
      nav={{
        title: (
          <span className="flex items-center gap-2.5">
            <RigLogo className="size-6 shrink-0" />
            <span className="text-[15px] font-semibold tracking-tight text-foreground">
              Rig
            </span>
            <span className="rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-[11px] font-medium text-muted-foreground">
              v{site.version}
            </span>
          </span>
        ),
        url: '/docs/',
      }}
      githubUrl={site.repository}
      sidebar={{ defaultOpenLevel: 1 }}
      slots={{ themeSwitch: AnimatedThemeToggler }}
    >
      {children}
    </DocsLayout>
  );
}
