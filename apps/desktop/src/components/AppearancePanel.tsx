import {
  Check,
  Clock,
  Home,
  Layers,
  Monitor,
  Moon,
  Palette,
  PanelLeft,
  RotateCcw,
  Sliders,
  Square,
  Sun,
  TerminalSquare,
  X,
} from 'lucide-react'
import { formatExactTime, relativeTime } from '@/lib/format'
import {
  ACCENT_PRESETS,
  SURFACE_PRESETS,
  usePreferences,
  type CornerRadius,
  type DefaultSection,
  type LogFontSize,
  type SidebarLayout,
  type ThemeMode,
  type TimeStyle,
  type TimeZoneMode,
  type UiDensity,
} from '@/lib/preferences'

function Segmented<T extends string | number>({
  value,
  options,
  onChange,
}: {
  value: T
  options: { value: T; label: string; icon?: React.ReactNode }[]
  onChange: (next: T) => void
}) {
  return (
    <div className="inline-flex rounded-lg border border-line bg-bg p-0.5 text-xs">
      {options.map((opt) => {
        const active = value === opt.value
        return (
          <button
            key={String(opt.value)}
            type="button"
            onClick={() => onChange(opt.value)}
            className={`inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 font-medium transition-colors ${
              active ? 'bg-surface-2 text-fg ring-1 ring-line' : 'text-muted hover:text-fg'
            }`}
          >
            {opt.icon}
            {opt.label}
          </button>
        )
      })}
    </div>
  )
}

export function AppearancePanel() {
  const { prefs, resolvedTheme, updatePrefs, resetPrefs } = usePreferences()
  const sampleIso = new Date(Date.now() - 4 * 60_000).toISOString()

  return (
    <div className="space-y-4 text-xs">
      {/* Theme & Surface */}
      <div className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-surface">
        <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3.5">
          <div className="flex items-start gap-2.5">
            <Sun className="mt-0.5 size-4 shrink-0 text-emerald-400" />
            <div>
              <div className="font-semibold text-fg">Theme</div>
              <div className="text-[11px] text-muted">
                Choose dark, light, or follow your operating system.
              </div>
            </div>
          </div>
          <Segmented<ThemeMode>
            value={prefs.theme}
            onChange={(theme) => updatePrefs({ theme })}
            options={[
              { value: 'dark', label: 'Dark', icon: <Moon className="size-3" /> },
              { value: 'light', label: 'Light', icon: <Sun className="size-3" /> },
              { value: 'system', label: 'System', icon: <Monitor className="size-3" /> },
            ]}
          />
        </div>

        {resolvedTheme === 'dark' && (
          <div className="px-4 py-3.5">
            <div className="mb-2.5 flex items-start gap-2.5">
              <Layers className="mt-0.5 size-4 shrink-0 text-emerald-400" />
              <div>
                <div className="font-semibold text-fg">Dark Surface Tone</div>
                <div className="text-[11px] text-muted">
                  Adjust background contrast for dark mode.
                </div>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-2 pl-6 sm:grid-cols-4">
              {SURFACE_PRESETS.map((preset) => {
                const active = prefs.surface === preset.value
                return (
                  <button
                    key={preset.value}
                    type="button"
                    onClick={() => updatePrefs({ surface: preset.value })}
                    className={`flex items-center gap-2 rounded-lg border px-2.5 py-2 text-left transition-colors ${
                      active
                        ? 'border-emerald-500 bg-emerald-500/10 text-fg'
                        : 'border-line bg-bg text-muted hover:text-fg'
                    }`}
                  >
                    <span
                      className="flex size-5 shrink-0 items-end justify-end rounded border border-white/15 p-0.5"
                      style={{ backgroundColor: preset.bg }}
                    >
                      <span
                        className="size-2.5 rounded-xs"
                        style={{ backgroundColor: preset.surface }}
                      />
                    </span>
                    <span className="truncate font-medium">{preset.label}</span>
                  </button>
                )
              })}
            </div>
          </div>
        )}

        {/* Accent color */}
        <div className="px-4 py-3.5">
          <div className="mb-2.5 flex items-start gap-2.5">
            <Palette className="mt-0.5 size-4 shrink-0 text-emerald-400" />
            <div>
              <div className="font-semibold text-fg">Accent Color</div>
              <div className="text-[11px] text-muted">
                Applies across primary buttons, active tabs, focus rings, and highlights.
              </div>
            </div>
          </div>
          <div className="flex flex-wrap gap-1.5 pl-6">
            {ACCENT_PRESETS.map((preset) => {
              const active = prefs.accent === preset.value
              return (
                <button
                  key={preset.value}
                  type="button"
                  onClick={() => updatePrefs({ accent: preset.value })}
                  className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1.5 font-medium transition-colors ${
                    active
                      ? 'border-emerald-500 bg-emerald-500/15 text-fg'
                      : 'border-line bg-bg text-muted hover:text-fg'
                  }`}
                >
                  <span
                    className="flex size-3.5 items-center justify-center rounded-full"
                    style={{ backgroundColor: preset.swatch }}
                  >
                    {active && <Check className="size-2.5 text-white" />}
                  </span>
                  {preset.label}
                </button>
              )
            })}
          </div>
        </div>

        {/* UI Density */}
        <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3.5">
          <div className="flex items-start gap-2.5">
            <Sliders className="mt-0.5 size-4 shrink-0 text-emerald-400" />
            <div>
              <div className="font-semibold text-fg">UI Density</div>
              <div className="text-[11px] text-muted">
                Tighten spacing to fit more data or increase legibility on HiDPI displays.
              </div>
            </div>
          </div>
          <Segmented<UiDensity>
            value={prefs.density}
            onChange={(density) => updatePrefs({ density })}
            options={[
              { value: 'compact', label: 'Compact' },
              { value: 'default', label: 'Default' },
              { value: 'comfortable', label: 'Comfortable' },
            ]}
          />
        </div>

        {/* Corner Style */}
        <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3.5">
          <div className="flex items-start gap-2.5">
            <Square className="mt-0.5 size-4 shrink-0 text-emerald-400" />
            <div>
              <div className="font-semibold text-fg">Corner Style</div>
              <div className="text-[11px] text-muted">
                Adjust border radius across panels, inputs, and buttons.
              </div>
            </div>
          </div>
          <Segmented<CornerRadius>
            value={prefs.radius}
            onChange={(radius) => updatePrefs({ radius })}
            options={[
              { value: 'sharp', label: 'Sharp' },
              { value: 'rounded', label: 'Rounded' },
              { value: 'soft', label: 'Soft' },
            ]}
          />
        </div>

        {/* Sidebar Layout */}
        <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3.5">
          <div className="flex items-start gap-2.5">
            <PanelLeft className="mt-0.5 size-4 shrink-0 text-emerald-400" />
            <div>
              <div className="font-semibold text-fg">Navigation Sidebar</div>
              <div className="text-[11px] text-muted">
                Collapse the server navigation to icons or hide shortcut footer hints.
              </div>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <Segmented<SidebarLayout>
              value={prefs.sidebarLayout}
              onChange={(sidebarLayout) => updatePrefs({ sidebarLayout })}
              options={[
                { value: 'expanded', label: 'Expanded' },
                { value: 'compact', label: 'Icon only' },
              ]}
            />
            <label className="inline-flex cursor-pointer items-center gap-1.5 text-muted hover:text-fg">
              <input
                type="checkbox"
                checked={prefs.showShortcutHints}
                onChange={(e) => updatePrefs({ showShortcutHints: e.target.checked })}
                className="accent-emerald-500"
              />
              Shortcut hints
            </label>
          </div>
        </div>
      </div>

      {/* Workspace, Logs & Time */}
      <div className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-surface">
        <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3.5">
          <div className="flex items-start gap-2.5">
            <TerminalSquare className="mt-0.5 size-4 shrink-0 text-emerald-400" />
            <div>
              <div className="font-semibold text-fg">Logs & Terminal</div>
              <div className="text-[11px] text-muted">
                Monospace font size and default line wrapping in log viewers and shells.
              </div>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <Segmented<LogFontSize>
              value={prefs.logFontSize}
              onChange={(logFontSize) => updatePrefs({ logFontSize })}
              options={[
                { value: 11, label: '11px' },
                { value: 12, label: '12px' },
                { value: 13, label: '13px' },
                { value: 14, label: '14px' },
              ]}
            />
            <label className="inline-flex cursor-pointer items-center gap-1.5 text-muted hover:text-fg">
              <input
                type="checkbox"
                checked={prefs.logWrap}
                onChange={(e) => updatePrefs({ logWrap: e.target.checked })}
                className="accent-emerald-500"
              />
              Wrap lines
            </label>
          </div>
        </div>

        <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3.5">
          <div className="flex items-start gap-2.5">
            <Clock className="mt-0.5 size-4 shrink-0 text-emerald-400" />
            <div>
              <div className="font-semibold text-fg">Timestamps</div>
              <div className="text-[11px] text-muted">
                Preview:{' '}
                <span className="font-mono text-fg">{relativeTime(sampleIso)}</span> ·{' '}
                <span className="font-mono">{formatExactTime(sampleIso)}</span>
              </div>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Segmented<TimeStyle>
              value={prefs.timeStyle}
              onChange={(timeStyle) => updatePrefs({ timeStyle })}
              options={[
                { value: 'relative', label: 'Relative' },
                { value: 'exact', label: 'Exact' },
              ]}
            />
            <Segmented<TimeZoneMode>
              value={prefs.timeZone}
              onChange={(timeZone) => updatePrefs({ timeZone })}
              options={[
                { value: 'local', label: 'Local' },
                { value: 'utc', label: 'UTC' },
              ]}
            />
          </div>
        </div>

        <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3.5">
          <div className="flex items-start gap-2.5">
            <Home className="mt-0.5 size-4 shrink-0 text-emerald-400" />
            <div>
              <div className="font-semibold text-fg">Default Landing View</div>
              <div className="text-[11px] text-muted">
                Which screen opens first when launching Rig Desktop.
              </div>
            </div>
          </div>
          <Segmented<DefaultSection>
            value={prefs.defaultSection}
            onChange={(defaultSection) => updatePrefs({ defaultSection })}
            options={[
              { value: 'overview', label: 'Overview' },
              { value: 'apps', label: 'Apps' },
              { value: 'local', label: 'Local Repos' },
              { value: 'databases', label: 'Databases' },
            ]}
          />
        </div>
      </div>

      <div className="flex justify-end">
        <button
          type="button"
          onClick={resetPrefs}
          className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-muted hover:bg-surface-2 hover:text-fg"
        >
          <RotateCcw className="size-3.5" />
          Reset UI Defaults
        </button>
      </div>
    </div>
  )
}

export function AppearanceModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  if (!open) return null
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4"
      onClick={onClose}
    >
      <div
        className="flex max-h-[88vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-line bg-bg shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-line bg-surface px-5 py-3.5">
          <div className="flex items-center gap-2.5">
            <div className="flex size-8 items-center justify-center rounded-lg bg-emerald-500/15 text-emerald-400">
              <Palette className="size-4" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-fg">Appearance & UI Preferences</h2>
              <p className="text-[11px] text-muted">
                Saved locally on this machine — changes apply live across all servers
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg p-1.5 text-muted hover:bg-surface-2 hover:text-fg"
          >
            <X className="size-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          <AppearancePanel />
        </div>
      </div>
    </div>
  )
}
