import { useQuery } from '@tanstack/react-query'
import { Activity } from 'lucide-react'
import { useId, useMemo, useState, type PointerEvent } from 'react'
import { Panel } from '@rig/ui/app/page'
import { Tabs, TabsItem } from '@rig/ui/tabs'
import { formatBytes } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import type { MetricPoint, MetricRange } from '@/lib/types'

const RANGES: { value: MetricRange; label: string; name: string }[] = [
  { value: '1h', label: '1h', name: 'Last hour' },
  { value: '6h', label: '6h', name: 'Last 6 hours' },
  { value: '24h', label: '24h', name: 'Last 24 hours' },
]

const INTERVALS = 120
const W = 600
const H = 96
const TIME = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' })

const formatCpu = (n: number) => `${n >= 10 ? n.toFixed(0) : n.toFixed(1)}%`
const formatRate = (n: number) => `${formatBytes(Math.round(n))}/s`

const cpuValue = (p: MetricPoint) => p.cpu_percent
const memoryValue = (p: MetricPoint) => p.memory_bytes
const rxValue = (p: MetricPoint) => p.net_rx_bps ?? 0
const txValue = (p: MetricPoint) => p.net_tx_bps ?? 0

function MetricChart({
  label,
  points,
  step,
  value,
  format,
  ceiling,
  color,
}: {
  label: string
  points: MetricPoint[]
  step: number
  value: (p: MetricPoint) => number
  format: (n: number) => string
  ceiling?: number
  color: string
}) {
  const gradientId = useId()
  const [hover, setHover] = useState<number | null>(null)

  const shape = useMemo(() => {
    if (points.length === 0) return null
    const safeStep = Math.max(1, step || 30)
    const end = points[points.length - 1]!.ts + safeStep
    const start = end - INTERVALS * safeStep
    const peak = points.reduce((m, p) => Math.max(m, value(p)), 0)
    const top = ceiling ?? (peak > 0 ? peak * 1.25 : 1)
    const x = (ts: number) => ((ts - start) / (end - start)) * W
    const y = (v: number) => H - Math.min(1, Math.max(0, v) / top) * (H - 6) - 2

    let line = ''
    let area = ''
    points.forEach((p, i) => {
      const px = x(p.ts).toFixed(1)
      const py = y(value(p)).toFixed(1)
      const fresh = i === 0 || p.ts - points[i - 1]!.ts > safeStep * 1.5
      if (fresh) {
        if (i > 0) area += `L${x(points[i - 1]!.ts).toFixed(1)},${H}Z`
        line += `M${px},${py}`
        area += `M${px},${H}L${px},${py}`
      } else {
        line += `L${px},${py}`
        area += `L${px},${py}`
      }
      if (fresh && (i === points.length - 1 || points[i + 1]!.ts - p.ts > safeStep * 1.5)) {
        line += 'h0.01'
      }
    })
    area += `L${x(points[points.length - 1]!.ts).toFixed(1)},${H}Z`
    return { line, area, x, y, start, end, peak, top }
  }, [points, step, value, ceiling])

  const latest = points[points.length - 1]
  const shown = hover !== null ? points[hover] : latest

  const onMove = (e: PointerEvent<SVGSVGElement>) => {
    if (!shape || points.length === 0) return
    const box = e.currentTarget.getBoundingClientRect()
    if (box.width <= 0) return
    const ts = shape.start + ((e.clientX - box.left) / box.width) * (shape.end - shape.start)
    let best = 0
    for (let i = 1; i < points.length; i++) {
      if (Math.abs(points[i]!.ts - ts) < Math.abs(points[best]!.ts - ts)) best = i
    }
    setHover(best)
  }

  return (
    <figure className="flex min-w-0 flex-col gap-1.5">
      <figcaption className="flex items-baseline justify-between gap-3">
        <span className="text-[12.5px] font-medium text-muted-foreground">{label}</span>
        <span className="flex items-baseline gap-2 tabular-nums">
          {hover !== null && shown ? (
            <time className="font-mono text-[11.5px] text-muted-foreground">
              {TIME.format(new Date(shown.ts * 1000))}
            </time>
          ) : null}
          <span className="font-mono text-[13px] font-semibold text-foreground">
            {shown ? format(value(shown)) : '–'}
          </span>
        </span>
      </figcaption>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        role="img"
        aria-label={
          latest && shape
            ? `${label}: now ${format(value(latest))}, peak ${format(shape.peak)}`
            : `${label}: no data`
        }
        className="h-24 w-full touch-none overflow-visible rounded-sm border-b border-border/80"
        style={{ color }}
        onPointerMove={onMove}
        onPointerLeave={() => setHover(null)}
      >
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="currentColor" stopOpacity="0.22" />
            <stop offset="1" stopColor="currentColor" stopOpacity="0" />
          </linearGradient>
        </defs>
        <line
          x1="0"
          x2={W}
          y1={H * 0.5}
          y2={H * 0.5}
          stroke="var(--border)"
          strokeOpacity="0.55"
          strokeDasharray="3 3"
          strokeWidth="1"
          vectorEffect="non-scaling-stroke"
        />
        {shape ? (
          <>
            <path d={shape.area} fill={`url(#${gradientId})`} />
            <path
              d={shape.line}
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinejoin="round"
              strokeLinecap="round"
              vectorEffect="non-scaling-stroke"
            />
            {hover !== null && shown ? (
              <line
                x1={shape.x(shown.ts)}
                x2={shape.x(shown.ts)}
                y1="0"
                y2={H}
                stroke="currentColor"
                strokeOpacity="0.55"
                strokeWidth="1"
                vectorEffect="non-scaling-stroke"
              />
            ) : null}
          </>
        ) : null}
      </svg>
      {shape ? (
        <div className="flex items-center justify-between font-mono text-[11px] text-muted-foreground/80 tabular-nums">
          <span>{TIME.format(new Date(shape.start * 1000))}</span>
          <span>peak {format(shape.peak)}</span>
          <span>{TIME.format(new Date(shape.end * 1000))}</span>
        </div>
      ) : null}
    </figure>
  )
}

export type MetricTarget =
  | { kind: 'host' }
  | { kind: 'app'; id: string }
  | { kind: 'database'; id: string }

export function UsagePanel({
  serverId,
  target,
  title = 'Usage History',
  className,
}: {
  serverId: string
  target: MetricTarget
  title?: string
  className?: string
}) {
  const [range, setRange] = useState<MetricRange>('1h')
  const targetKey = target.kind === 'host' ? 'host' : `${target.kind}:${target.id}`

  const { data, isPending, isError } = useQuery({
    queryKey: ['desktop', serverId, 'metrics', targetKey, range],
    queryFn: () => desktopApi.getMetrics(serverId, target, range),
    refetchInterval: 30_000,
    staleTime: 15_000,
  })

  const host = target.kind === 'host'
  const empty = !data || data.points.length === 0
  const network = useMemo(
    () => (data?.points ?? []).filter((p) => p.net_rx_bps != null && p.net_tx_bps != null),
    [data],
  )
  const rangeIndex = Math.max(
    0,
    RANGES.findIndex((r) => r.value === range),
  )

  return (
    <Panel
      title={
        <>
          <Activity className="size-4 text-primary" />
          {title}
        </>
      }
      className={className}
      actions={
        <Tabs
          size="compact"
          selectedIndex={rangeIndex}
          onSelect={(i) => setRange(RANGES[i]?.value ?? '1h')}
          aria-label="Time range"
        >
          {RANGES.map((r, i) => (
            <TabsItem
              key={r.value}
              index={i}
              label={r.label}
              aria-label={r.name}
              className="tabular-nums"
            />
          ))}
        </Tabs>
      }
    >
      {isPending ? (
        <div className="grid gap-6 sm:grid-cols-2">
          <div className="h-32 animate-pulse rounded-lg bg-muted/60" />
          <div className="h-32 animate-pulse rounded-lg bg-muted/60" />
        </div>
      ) : isError ? (
        <p className="text-[13px] text-muted-foreground">
          Metrics could not be loaded right now. Retrying automatically every 30s.
        </p>
      ) : empty ? (
        <p className="text-[13px] text-muted-foreground">
          {host
            ? 'No samples yet. The first host sample arrives within 30 seconds.'
            : 'No samples recorded in this time range yet.'}
        </p>
      ) : (
        <div className="grid gap-x-8 gap-y-6 sm:grid-cols-2">
          <MetricChart
            label="CPU"
            points={data.points}
            step={data.step_seconds}
            value={cpuValue}
            format={formatCpu}
            ceiling={host ? 100 : undefined}
            color="var(--chart-1)"
          />
          <MetricChart
            label={
              data.memory_total > 0
                ? `Memory · ${formatBytes(data.memory_total)}${host ? '' : ' limit'}`
                : 'Memory'
            }
            points={data.points}
            step={data.step_seconds}
            value={memoryValue}
            format={formatBytes}
            ceiling={host && data.memory_total > 0 ? data.memory_total : undefined}
            color="var(--chart-2)"
          />
          <MetricChart
            label="Network in"
            points={network}
            step={data.step_seconds}
            value={rxValue}
            format={formatRate}
            color="var(--chart-3)"
          />
          <MetricChart
            label="Network out"
            points={network}
            step={data.step_seconds}
            value={txValue}
            format={formatRate}
            color="var(--chart-4)"
          />
        </div>
      )}
    </Panel>
  )
}
