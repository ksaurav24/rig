import {
  ExternalLink,
  HardDrive,
  KeyRound,
  LayoutGrid,
  Rocket,
  Search,
  Sparkles,
} from 'lucide-react'
import { useDeferredValue, useEffect, useMemo, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { Page } from '@rig/ui/app/page'
import { EmptyState } from '@rig/ui/app/state'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { Card, CardContent, CardFooter, CardGroup, CardHeader, CardTitle } from '@rig/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@rig/ui/dialog'
import { Field, FieldDescription, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@rig/ui/select'
import { Tabs, TabsItem } from '@rig/ui/tabs'
import {
  CATALOG,
  CATALOG_CATEGORIES,
  generateSecret,
  type CatalogCategory,
  type CatalogTemplate,
} from '@/lib/catalog'
import { desktopApi } from '@/lib/ipc'
import type { App, Project, SetEnvVarRequest } from '@/lib/types'

interface CatalogViewProps {
  serverId: string
  projects: Project[]
  onDeployed: (app: App) => void
}

function monogram(name: string): string {
  const words = name.split(/[\s-]+/).filter(Boolean)
  if (words.length >= 2) return `${words[0]![0]}${words[1]![0]}`.toUpperCase()
  return name.slice(0, 2).toUpperCase()
}

export function CatalogView({ serverId, projects, onDeployed }: CatalogViewProps) {
  const [category, setCategory] = useState<'All' | CatalogCategory>('All')
  const [search, setSearch] = useState('')
  const deferredSearch = useDeferredValue(search)
  const [selected, setSelected] = useState<CatalogTemplate | null>(null)

  const categories = useMemo(
    () =>
      [
        { id: 'All' as const, label: `All (${CATALOG.length})` },
        ...CATALOG_CATEGORIES.map((c) => ({
          id: c,
          label: `${c} (${CATALOG.filter((t) => t.category === c).length})`,
        })),
      ],
    [],
  )
  const categoryIdx = Math.max(
    0,
    categories.findIndex((c) => c.id === category),
  )

  const filtered = useMemo(() => {
    const q = deferredSearch.trim().toLowerCase()
    return CATALOG.filter((t) => {
      if (category !== 'All' && t.category !== category) return false
      if (!q) return true
      return (
        t.name.toLowerCase().includes(q) ||
        t.description.toLowerCase().includes(q) ||
        t.category.toLowerCase().includes(q) ||
        t.image.toLowerCase().includes(q)
      )
    })
  }, [category, deferredSearch])

  return (
    <Page
      title="App Catalog"
      description="Pre-configured self-hosted Docker templates with persistent storage and auto-generated secrets."
    >
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/80 pb-3">
        <Tabs
          size="compact"
          selectedIndex={categoryIdx}
          onSelect={(idx) => setCategory(categories[idx]?.id ?? 'All')}
        >
          {categories.map((c, idx) => (
            <TabsItem key={c.id} index={idx} label={c.label} />
          ))}
        </Tabs>

        <div className="relative w-full sm:w-64">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            size="compact"
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search templates…"
            className="pl-8"
          />
        </div>
      </div>

      {filtered.length === 0 ? (
        <EmptyState
          icon={LayoutGrid}
          title="No templates matched"
          action={
            <Button
              type="button"
              variant="tertiary"
              onClick={() => {
                setCategory('All')
                setSearch('')
              }}
            >
              Clear filters
            </Button>
          }
        >
          Try another search term or switch back to all categories.
        </EmptyState>
      ) : (
        <CardGroup
          columns={3}
          border="outlined"
          separated
          className="grid-cols-1 gap-3.5 sm:grid-cols-2 lg:grid-cols-3"
        >
          {filtered.map((tpl) => {
            const generatedCount = tpl.variables.filter((v) => v.generate === 'password').length
            return (
              <Card
                key={tpl.id}
                onClick={() => setSelected(tpl)}
                label={`Deploy ${tpl.name}`}
                className="bg-card shadow-[var(--shadow-2)]"
              >
                <CardHeader>
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex min-w-0 items-center gap-3">
                      <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-border/80 bg-surface font-mono text-[12px] font-semibold text-foreground">
                        {monogram(tpl.name)}
                      </span>
                      <div className="min-w-0">
                        <CardTitle className="truncate text-[15px] font-semibold">
                          {tpl.name}
                        </CardTitle>
                        <div className="text-[12px] text-muted-foreground">{tpl.category}</div>
                      </div>
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      aria-label={`Open ${tpl.name} website`}
                      title={tpl.website}
                      onClick={(e) => {
                        e.stopPropagation()
                        void desktopApi.openExternalUrl(tpl.website)
                      }}
                    >
                      <ExternalLink />
                    </Button>
                  </div>
                </CardHeader>

                <CardContent className="space-y-3">
                  <p className="line-clamp-2 text-[13px] leading-relaxed text-muted-foreground">
                    {tpl.description}
                  </p>
                  <div className="flex flex-wrap items-center gap-1.5">
                    <Badge variant="solid" color="gray" size="compact" className="font-mono">
                      {tpl.memoryMb} MB
                    </Badge>
                    {tpl.volumes.length > 0 && (
                      <Badge variant="solid" color="emerald" size="compact">
                        <HardDrive className="size-3" />
                        Storage ({tpl.volumes.length})
                      </Badge>
                    )}
                    {generatedCount > 0 && (
                      <Badge variant="solid" color="amber" size="compact">
                        <Sparkles className="size-3" />
                        Auto-secrets
                      </Badge>
                    )}
                  </div>
                </CardContent>

                <CardFooter className="mt-auto border-t border-border/70 pt-3">
                  <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-muted-foreground">
                    {tpl.image}
                  </span>
                  <Button
                    type="button"
                    variant="primary"
                    size="compact"
                    leadingIcon={Rocket}
                    onClick={(e) => {
                      e.stopPropagation()
                      setSelected(tpl)
                    }}
                  >
                    Deploy
                  </Button>
                </CardFooter>
              </Card>
            )
          })}
        </CardGroup>
      )}

      <DeployTemplateDialog
        serverId={serverId}
        template={selected}
        projects={projects}
        onClose={() => setSelected(null)}
        onDeployed={onDeployed}
      />
    </Page>
  )
}

function DeployTemplateDialog({
  serverId,
  template,
  projects,
  onClose,
  onDeployed,
}: {
  serverId: string
  template: CatalogTemplate | null
  projects: Project[]
  onClose: () => void
  onDeployed: (app: App) => void
}) {
  const [projectId, setProjectId] = useState(projects[0]?.id ?? '__new__')
  const [newProjectName, setNewProjectName] = useState('Apps')
  const [appName, setAppName] = useState('')
  const [memoryMb, setMemoryMb] = useState(512)
  const [varValues, setVarValues] = useState<Record<string, string>>({})
  const [deploying, setDeploying] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!template) return
    setProjectId(projects[0]?.id ?? '__new__')
    setAppName(template.id)
    setMemoryMb(template.memoryMb)
    const defaults: Record<string, string> = {}
    for (const v of template.variables) {
      if (v.value !== undefined) defaults[v.key] = v.value
    }
    setVarValues(defaults)
    setError(null)
  }, [template, projects])

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    if (!template || !appName.trim()) return
    setDeploying(true)
    setError(null)
    try {
      let targetProjectId = projectId
      if (targetProjectId === '__new__') {
        const createdProj = await desktopApi.createProject(
          serverId,
          newProjectName.trim() || 'Apps',
          '',
        )
        targetProjectId = createdProj.id
      }

      const env: SetEnvVarRequest[] = template.variables.map((v) => {
        const supplied = (varValues[v.key] ?? '').trim()
        const value =
          supplied || (v.generate === 'password' ? generateSecret(32) : v.value ?? '')
        return {
          key: v.key,
          value,
          is_secret: Boolean(v.secret || v.generate),
        }
      })

      const created = await desktopApi.createApp(serverId, {
        project_id: targetProjectId,
        name: appName.trim(),
        source_kind: 'image',
        image: template.image,
        port: template.port,
        memory_mb: Number(memoryMb) || template.memoryMb,
        health_path: template.healthPath,
        volumes: template.volumes,
        env,
      })

      await desktopApi.appAction(serverId, created.id, 'deploy')
      toast.success(`Deploying ${created.name}`, {
        description:
          env.some((eVar) => eVar.is_secret)
            ? 'Auto-generated secrets are saved in the app Env tab.'
            : undefined,
      })
      onDeployed(created)
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setDeploying(false)
    }
  }

  const editableVars = template?.variables.filter((v) => !v.generate || v.description) ?? []

  return (
    <Dialog open={Boolean(template)} onOpenChange={(open) => !open && onClose()}>
      <DialogContent size="lg">
        {template && (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2.5">
                <span className="flex size-7 items-center justify-center rounded-md border border-border/80 bg-surface font-mono text-[11px] font-semibold">
                  {monogram(template.name)}
                </span>
                Deploy {template.name}
              </DialogTitle>
              <DialogDescription>{template.description}</DialogDescription>
            </DialogHeader>

            <form onSubmit={handleSubmit} className="space-y-4">
              {error && (
                <Banner role="alert" status="error" contrast="high" size="compact">
                  <BannerDescription>{error}</BannerDescription>
                </Banner>
              )}

              <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
                <Field>
                  <FieldLabel>Project</FieldLabel>
                  <Select value={projectId} onValueChange={setProjectId}>
                    <SelectTrigger className="w-full" placeholder="Select project" />
                    <SelectContent>
                      {projects.map((p, idx) => (
                        <SelectItem key={p.id} index={idx} value={p.id}>
                          {p.name}
                        </SelectItem>
                      ))}
                      <SelectItem index={projects.length} value="__new__">
                        + Create new project…
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </Field>

                <Field>
                  <FieldLabel htmlFor="cat-app-name">App Name</FieldLabel>
                  <Input
                    id="cat-app-name"
                    type="text"
                    required
                    value={appName}
                    onChange={(e) => setAppName(e.target.value)}
                    className="font-mono"
                  />
                </Field>
              </div>

              {projectId === '__new__' && (
                <Field>
                  <FieldLabel htmlFor="cat-new-proj">New Project Name</FieldLabel>
                  <Input
                    id="cat-new-proj"
                    type="text"
                    required
                    value={newProjectName}
                    onChange={(e) => setNewProjectName(e.target.value)}
                    placeholder="Apps, Homelab…"
                  />
                </Field>
              )}

              <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
                <Field>
                  <FieldLabel>Container Image</FieldLabel>
                  <Input readOnly value={template.image} className="bg-surface/60 font-mono text-muted-foreground" />
                </Field>
                <Field>
                  <FieldLabel htmlFor="cat-mem">Memory Limit (MB)</FieldLabel>
                  <Input
                    id="cat-mem"
                    type="number"
                    min={64}
                    max={65536}
                    value={memoryMb}
                    onChange={(e) => setMemoryMb(Number(e.target.value))}
                    className="font-mono"
                  />
                </Field>
              </div>

              {template.volumes.length > 0 && (
                <div className="rounded-lg border border-border/80 bg-surface/60 px-3.5 py-2.5 text-[12.5px]">
                  <div className="flex items-center gap-1.5 font-medium text-foreground">
                    <HardDrive className="size-3.5 text-primary" />
                    Persistent storage attached automatically
                  </div>
                  <div className="mt-1.5 flex flex-wrap gap-1.5 font-mono text-[11.5px]">
                    {template.volumes.map((vol) => (
                      <Badge key={vol} variant="solid" color="gray" size="compact">
                        {vol}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}

              {editableVars.length > 0 && (
                <div className="space-y-3 rounded-lg border border-border/80 bg-surface/60 p-3.5">
                  <div className="flex items-center gap-1.5 text-[12.5px] font-medium text-foreground">
                    <KeyRound className="size-3.5 text-primary" />
                    Environment &amp; Generated Secrets
                  </div>
                  {editableVars.map((v) => (
                    <Field key={v.key}>
                      <FieldLabel htmlFor={`cat-var-${v.key}`} className="font-mono text-[12px]">
                        {v.key}
                        {v.generate === 'password' && (
                          <span className="ml-2 font-sans text-[11px] font-normal text-muted-foreground">
                            (leave blank to auto-generate)
                          </span>
                        )}
                      </FieldLabel>
                      <Input
                        id={`cat-var-${v.key}`}
                        type={v.secret ? 'password' : 'text'}
                        required={Boolean(v.required && !v.generate)}
                        value={varValues[v.key] ?? ''}
                        onChange={(e) =>
                          setVarValues((prev) => ({ ...prev, [v.key]: e.target.value }))
                        }
                        placeholder={
                          v.generate === 'password' ? 'Auto-generated 32-char secret' : v.value ?? ''
                        }
                        className="font-mono"
                      />
                      {v.description && <FieldDescription>{v.description}</FieldDescription>}
                    </Field>
                  ))}
                </div>
              )}

              <DialogFooter>
                <Button type="button" variant="ghost" onClick={onClose}>
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="primary"
                  leadingIcon={Rocket}
                  loading={deploying}
                  disabled={!appName.trim()}
                >
                  Deploy {template.name}
                </Button>
              </DialogFooter>
            </form>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
