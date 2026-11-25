import { useQuery } from '@tanstack/react-query'
import { useId, useMemo, useState, type PointerEvent } from 'react'

import { metricsQuery, type MetricRange, type MetricTarget } from '@/api/queries'
import type { Schemas } from '@/api/client'
import { Panel } from '@rig/ui/app/page'
import { Tabs, TabsItem } from '@rig/ui/tabs'
import { Skeleton } from '@/components/ui/skeleton'
import { bytes } from '@/lib/format'

type Point = Schemas['MetricPoint']

const RANGES: { value: MetricRange; label: string; name: string }[] = [
  { value: '1h', label: '1h', name: 'Last hour' },
  { value: '6h', label: '6h', name: 'Last 6 hours' },
  { value: '24h', label: '24h', name: 'Last 24 hours' },
]
/** Every range comes back as this many intervals. */
const INTERVALS = 120
const W = 600
const H = 96
const TIME = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' })

const cpu = (n: number) => `${n >= 10 ? n.toFixed(0) : n.toFixed(1)}%`

/**
 * A small area chart drawn as one SVG path: no chart library, no layout work
 * beyond the browser scaling the viewBox.
 */
function Chart({
  label,
  points,
  step,
  value,
  format,
  ceiling,
  color,
}: {
  label: string
  points: Point[]
  step: number
  value: (p: Point) => number
  format: (n: number) => string
  /** Fixed top of the scale. Without it the chart fits the data. */
  ceiling?: number
  color: string
}) {
  const gradient = useId()
  const [hover, setHover] = useState<number | null>(null)

  const shape = useMemo(() => {
    if (points.length === 0) return null
    // The window ends at the newest sample, so a young series fills from the right.
    const end = points[points.length - 1].ts + step
    const start = end - INTERVALS * step
    const peak = points.reduce((m, p) => Math.max(m, value(p)), 0)
    const top = ceiling ?? (peak > 0 ? peak * 1.25 : 1)
    const x = (ts: number) => ((ts - start) / (end - start)) * W
    const y = (v: number) => H - Math.min(1, v / top) * (H - 6) - 2

    // Break the line where samples are missing instead of drawing through the gap.
    let line = ''
    let area = ''
    points.forEach((p, i) => {
      const px = x(p.ts).toFixed(1)
      const py = y(value(p)).toFixed(1)
      const fresh = i === 0 || p.ts - points[i - 1].ts > step * 1.5
      if (fresh) {
        if (i > 0) area += `L${x(points[i - 1].ts).toFixed(1)},${H}Z`
        line += `M${px},${py}`
        area += `M${px},${H}L${px},${py}`
      } else {
        line += `L${px},${py}`
        area += `L${px},${py}`
      }
      // A lone sample has no width; give it a short dash so it is visible.
      if (fresh && (i === points.length - 1 || points[i + 1].ts - p.ts > step * 1.5)) line += `h0.01`
    })
    area += `L${x(points[points.length - 1].ts).toFixed(1)},${H}Z`
    return { line, area, x, y, start, end, peak, top }
  }, [points, step, value, ceiling])

  const latest = points.at(-1)
  const shown = hover !== null ? points[hover] : latest

  const onMove = (e: PointerEvent<SVGSVGElement>) => {
    if (!shape) return
    const box = e.currentTarget.getBoundingClientRect()
    const ts = shape.start + ((e.clientX - box.left) / box.width) * (shape.end - shape.start)
    let best = 0
    for (let i = 1; i < points.length; i++) {
      if (Math.abs(points[i].ts - ts) < Math.abs(points[best].ts - ts)) best = i
    }
    setHover(best)
  }

  return (
    <figure className="flex min-w-0 flex-col gap-1.5">
      <figcaption className="flex items-baseline justify-between gap-3">
        <span className="text-[12.5px] font-medium text-muted-foreground">{label}</span>
        <span className="flex items-baseline gap-2 tabular-nums">
          {hover !== null && shown ? (
            <time className="text-[11.5px] text-muted-foreground">{TIME.format(new Date(shown.ts * 1000))}</time>
          ) : null}
          <span className="text-[13px] font-semibold text-foreground">{shown ? format(value(shown)) : '–'}</span>
        </span>
      </figcaption>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        role="img"
        aria-label={
          latest && shape ? `${label}: now ${format(value(latest))}, peak ${format(shape.peak)}` : `${label}: no data`
        }
        className="h-24 w-full touch-none overflow-visible rounded-sm border-b border-border/80"
        style={{ color }}
        onPointerMove={onMove}
        onPointerLeave={() => setHover(null)}
      >
        <defs>
          <linearGradient id={gradient} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="currentColor" stopOpacity="0.2" />
            <stop offset="1" stopColor="currentColor" stopOpacity="0" />
          </linearGradient>
        </defs>
        {/* Subtle horizontal reference lines so sparse histories still show scale */}
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
            <path d={shape.area} fill={`url(#${gradient})`} />
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
                strokeOpacity="0.5"
                strokeWidth="1"
                vectorEffect="non-scaling-stroke"
              />
            ) : null}
          </>
        ) : null}
      </svg>
      {shape ? (
        <div className="flex items-center justify-between text-[11px] text-muted-foreground/80 tabular-nums">
          <span>{TIME.format(new Date(shape.start * 1000))}</span>
          <span>peak {format(shape.peak)}</span>
          <span>{TIME.format(new Date(shape.end * 1000))}</span>
        </div>
      ) : null}
    </figure>
  )
}

const cpuValue = (p: Point) => p.cpu_percent
const memoryValue = (p: Point) => p.memory_bytes
const rxValue = (p: Point) => p.net_rx_bps ?? 0
const txValue = (p: Point) => p.net_tx_bps ?? 0
const rate = (n: number) => `${bytes(Math.round(n))}/s`

/** CPU and memory over time for the host, an app or a database. */
export function UsagePanel({ target, className }: { target: MetricTarget; className?: string }) {
  const [range, setRange] = useState<MetricRange>('1h')
  const { data, isPending, isError } = useQuery(metricsQuery(target, range))
  const host = target.kind === 'host'
  const empty = data?.points.length === 0
  // Throughput needs two samples to measure between; the rest are gaps.
  const network = useMemo(() => data?.points.filter((p) => p.net_rx_bps != null && p.net_tx_bps != null) ?? [], [data])
  const rangeIndex = RANGES.findIndex((r) => r.value === range)

  return (
    <Panel
      title="Usage"
      className={className}
      actions={
        <Tabs
          size="compact"
          selectedIndex={rangeIndex}
          onSelect={(i) => setRange(RANGES[i].value)}
          aria-label="Time range"
          className="border border-border/80 bg-surface/60"
        >
          {RANGES.map((r, i) => (
            <TabsItem key={r.value} index={i} label={r.label} aria-label={r.name} className="tabular-nums" />
          ))}
        </Tabs>
      }
    >
      {isPending ? (
        <div className="grid gap-6 sm:grid-cols-2">
          <Skeleton className="h-32" />
          <Skeleton className="h-32" />
        </div>
      ) : isError || !data ? (
        <p className="text-[13px] text-muted-foreground">Usage could not be loaded. It will retry automatically.</p>
      ) : empty ? (
        <p className="text-[13px] text-muted-foreground">
          {host
            ? 'No samples yet. First sample arrives within 30 seconds.'
            : 'No samples in this range yet.'}
        </p>
      ) : (
        <div className="grid gap-x-8 gap-y-6 sm:grid-cols-2">
          <Chart
            label="CPU"
            points={data.points}
            step={data.step_seconds}
            value={cpuValue}
            format={cpu}
            ceiling={host ? 100 : undefined}
            color="var(--chart-1)"
          />
          <Chart
            label={data.memory_total > 0 ? `Memory · ${bytes(data.memory_total)}${host ? '' : ' limit'}` : 'Memory'}
            points={data.points}
            step={data.step_seconds}
            value={memoryValue}
            format={bytes}
            ceiling={host && data.memory_total > 0 ? data.memory_total : undefined}
            color="var(--chart-2)"
          />
          <Chart
            label="Network in"
            points={network}
            step={data.step_seconds}
            value={rxValue}
            format={rate}
            color="var(--chart-3)"
          />
          <Chart
            label="Network out"
            points={network}
            step={data.step_seconds}
            value={txValue}
            format={rate}
            color="var(--chart-4)"
          />
        </div>
      )}
    </Panel>
  )
}
