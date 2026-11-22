import type { ReactNode } from 'react';

export function RoleBadge({
  role,
}: {
  role: 'Viewer' | 'Developer' | 'Admin' | 'Owner' | 'Public' | 'Session Only';
}) {
  const styles: Record<string, string> = {
    Viewer: 'border-border bg-muted text-muted-foreground',
    Developer: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400',
    Admin: 'border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400',
    Owner: 'border-rose-500/30 bg-rose-500/10 text-rose-600 dark:text-rose-400',
    Public: 'border-sky-500/30 bg-sky-500/10 text-sky-600 dark:text-sky-400',
    'Session Only':
      'border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-400',
  };
  return (
    <span
      className={`inline-flex items-center rounded-md border px-2 py-0.5 font-mono text-[11px] font-medium leading-4 ${
        styles[role] ?? styles.Viewer
      }`}
    >
      {role}
    </span>
  );
}

export function Endpoint({
  method,
  path,
  role,
  children,
}: {
  method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE' | 'WS' | 'SSE';
  path: string;
  role?: 'Viewer' | 'Developer' | 'Admin' | 'Owner' | 'Public' | 'Session Only';
  children?: ReactNode;
}) {
  const methodColors: Record<string, string> = {
    GET: 'border-emerald-500/30 bg-emerald-500/12 text-emerald-700 dark:text-emerald-300',
    POST: 'border-sky-500/30 bg-sky-500/12 text-sky-700 dark:text-sky-300',
    PUT: 'border-amber-500/30 bg-amber-500/12 text-amber-700 dark:text-amber-300',
    PATCH: 'border-amber-500/30 bg-amber-500/12 text-amber-700 dark:text-amber-300',
    DELETE: 'border-rose-500/30 bg-rose-500/12 text-rose-700 dark:text-rose-300',
    WS: 'border-purple-500/30 bg-purple-500/12 text-purple-700 dark:text-purple-300',
    SSE: 'border-indigo-500/30 bg-indigo-500/12 text-indigo-700 dark:text-indigo-300',
  };
  return (
    <div className="not-prose my-4 overflow-hidden rounded-lg border border-border bg-card">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/80 bg-muted/50 px-3.5 py-2.5">
        <div className="flex items-center gap-2.5">
          <span
            className={`inline-flex items-center rounded border px-2 py-0.5 font-mono text-xs font-semibold tracking-wide ${
              methodColors[method] ?? methodColors.GET
            }`}
          >
            {method}
          </span>
          <code className="font-mono text-[13px] font-medium text-foreground">
            {path}
          </code>
        </div>
        {role && <RoleBadge role={role} />}
      </div>
      {children && (
        <div className="px-3.5 py-2.5 text-[13.5px] leading-relaxed text-muted-foreground">
          {children}
        </div>
      )}
    </div>
  );
}

export function StatGrid({
  items,
}: {
  items: { label: string; value: string; detail?: string }[];
}) {
  return (
    <div className="not-prose my-6 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
      {items.map((item) => (
        <div
          key={item.label}
          className="rounded-lg border border-border bg-card p-4"
        >
          <div className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {item.label}
          </div>
          <div className="mt-1.5 font-mono text-xl font-semibold tracking-tight text-foreground">
            {item.value}
          </div>
          {item.detail && (
            <div className="mt-1 text-xs leading-normal text-muted-foreground">
              {item.detail}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
