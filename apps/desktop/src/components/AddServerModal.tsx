import {
  AlertTriangle,
  ArrowRight,
  KeyRound,
  Lock,
  Server,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  X,
} from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { desktopApi } from '@/lib/ipc'
import type { ProbeServerResult, ServerView } from '@/lib/types'

interface AddServerModalProps {
  open: boolean
  onClose: () => void
  onAdded: (server: ServerView) => void
}

type AuthTab = 'pair' | 'password' | 'setup'

export function AddServerModal({ open, onClose, onAdded }: AddServerModalProps) {
  const [step, setStep] = useState<'url' | 'tls' | 'auth'>('url')
  const [url, setUrl] = useState('')
  const [serverName, setServerName] = useState('')
  const [probe, setProbe] = useState<ProbeServerResult | null>(null)
  const [pinnedFp, setPinnedFp] = useState<string | null>(null)
  const [tab, setTab] = useState<AuthTab>('pair')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Pair code state
  const [pairCode, setPairCode] = useState('')

  // Password + TOTP state
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [totpCode, setTotpCode] = useState('')
  const [challengeToken, setChallengeToken] = useState<string | null>(null)

  // First-run headless setup state
  const [setupToken, setSetupToken] = useState('')
  const [orgName, setOrgName] = useState('')
  const [ownerName, setOwnerName] = useState('')

  if (!open) return null

  function reset() {
    setStep('url')
    setProbe(null)
    setPinnedFp(null)
    setError(null)
    setPairCode('')
    setTotpCode('')
    setChallengeToken(null)
  }

  async function runProbe(acceptFingerprint?: string) {
    setBusy(true)
    setError(null)
    try {
      const res = await desktopApi.probeServer(url, acceptFingerprint)
      setProbe(res)
      setUrl(res.normalized_url)
      if (res.needs_tls_pin_confirmation) {
        setStep('tls')
        return
      }
      if (!res.api_compatible) {
        setError(res.compatibility_note ?? 'Server API version is incompatible.')
        return
      }
      if (acceptFingerprint) {
        setPinnedFp(acceptFingerprint)
      }
      if (res.setup && !res.setup.setup_complete) {
        setTab('setup')
      } else {
        setTab('pair')
      }
      setStep('auth')
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  async function handleUrlSubmit(e: FormEvent) {
    e.preventDefault()
    await runProbe()
  }

  async function handleTrustFingerprint() {
    const fp = probe?.tls.fingerprint_sha256
    if (!fp) return
    await runProbe(fp)
  }

  async function handlePairSubmit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const server = await desktopApi.pairServer({
        url,
        code: pairCode,
        serverName: serverName.trim() || undefined,
        pinnedTlsSha256: pinnedFp,
      })
      toast.success(`Paired with ${server.name}`)
      reset()
      onAdded(server)
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  async function handleLoginSubmit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const res = await desktopApi.loginServer({
        url,
        email,
        password,
        totpCode: totpCode.trim() || undefined,
        challengeToken: challengeToken ?? undefined,
        serverName: serverName.trim() || undefined,
        pinnedTlsSha256: pinnedFp,
      })
      if (res.status === 'second_factor_required') {
        setChallengeToken(res.challenge_token ?? null)
        setError('Enter your 6-digit authenticator or recovery code to finish signing in.')
        return
      }
      toast.success(`Connected to ${res.server.name}`)
      reset()
      onAdded(res.server)
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  async function handleSetupSubmit(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const server = await desktopApi.initServerSetup({
        url,
        pinnedTlsSha256: pinnedFp,
        serverName: serverName.trim() || undefined,
        setupToken,
        orgName,
        name: ownerName,
        email,
        password,
      })
      toast.success(`Initialized and connected to ${server.name}`)
      reset()
      onAdded(server)
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
      <div className="w-full max-w-lg rounded-2xl border border-line bg-surface shadow-2xl">
        <div className="flex items-center justify-between border-b border-line px-5 py-4">
          <div className="flex items-center gap-2.5">
            <div className="flex size-8 items-center justify-center rounded-lg bg-emerald-500/15 text-emerald-400">
              <Server className="size-4" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-fg">Add Rig Server</h2>
              <p className="text-xs text-muted">
                Connect public HTTPS, LAN (internal CA), or Tailscale servers
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={() => {
              reset()
              onClose()
            }}
            className="rounded-lg p-1.5 text-muted hover:bg-surface-2 hover:text-fg"
          >
            <X className="size-4" />
          </button>
        </div>

        <div className="p-5">
          {error && (
            <div className="mb-4 flex items-start gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2.5 text-xs text-rose-300">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
              <span>{error}</span>
            </div>
          )}

          {step === 'url' && (
            <form onSubmit={handleUrlSubmit} className="space-y-4">
              <div>
                <label className="mb-1.5 block text-xs font-medium text-fg">Server URL</label>
                <input
                  type="text"
                  autoFocus
                  required
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://rig.example.com or http://127.0.0.1:7460"
                  className="w-full rounded-lg border border-line bg-bg px-3 py-2 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                />
                <p className="mt-1.5 text-[11px] text-muted">
                  Supports domain HTTPS, LAN internal CA (with fingerprint pinning), and Tailscale
                  MagicDNS.
                </p>
              </div>
              <div>
                <label className="mb-1.5 block text-xs font-medium text-fg">
                  Display name <span className="text-muted">(optional)</span>
                </label>
                <input
                  type="text"
                  value={serverName}
                  onChange={(e) => setServerName(e.target.value)}
                  placeholder="Production EU, Homelab, Staging…"
                  className="w-full rounded-lg border border-line bg-bg px-3 py-2 text-xs text-fg outline-none focus:border-emerald-500"
                />
              </div>
              <div className="flex justify-end gap-2 pt-2">
                <button
                  type="button"
                  onClick={onClose}
                  className="rounded-lg px-3 py-2 text-xs text-muted hover:bg-surface-2 hover:text-fg"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={busy}
                  className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-4 py-2 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
                >
                  {busy ? 'Checking server…' : 'Continue'}
                  <ArrowRight className="size-3.5" />
                </button>
              </div>
            </form>
          )}

          {step === 'tls' && probe && (
            <div className="space-y-4">
              <div className="flex items-start gap-3 rounded-xl border border-amber-500/30 bg-amber-500/10 p-3.5">
                <ShieldAlert className="mt-0.5 size-5 shrink-0 text-amber-400" />
                <div className="space-y-1 text-xs">
                  <div className="font-semibold text-amber-200">
                    Self-signed / Internal CA Certificate
                  </div>
                  <p className="text-amber-200/80">
                    Verify this SHA-256 fingerprint matches your LAN server before pinning it. All
                    future requests, SSE streams, and terminals will require this exact certificate.
                  </p>
                </div>
              </div>

              <div className="rounded-lg border border-line bg-bg p-3">
                <div className="mb-1 text-[10px] font-medium tracking-wider text-muted uppercase">
                  Leaf Certificate SHA-256
                </div>
                <code className="block break-all font-mono text-[11px] leading-relaxed text-emerald-300">
                  {probe.tls.fingerprint_sha256 ?? 'Unavailable'}
                </code>
              </div>

              <div className="flex justify-between pt-2">
                <button
                  type="button"
                  onClick={() => setStep('url')}
                  className="rounded-lg px-3 py-2 text-xs text-muted hover:bg-surface-2 hover:text-fg"
                >
                  Back
                </button>
                <button
                  type="button"
                  disabled={busy}
                  onClick={handleTrustFingerprint}
                  className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-4 py-2 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
                >
                  <ShieldCheck className="size-3.5" />
                  {busy ? 'Pinning…' : 'Trust & Pin Certificate'}
                </button>
              </div>
            </div>
          )}

          {step === 'auth' && (
            <div className="space-y-4">
              <div className="flex items-center justify-between rounded-lg border border-line bg-bg px-3 py-2 text-xs">
                <div className="min-w-0 truncate font-mono text-fg">{url}</div>
                <div className="flex shrink-0 items-center gap-2">
                  {pinnedFp && (
                    <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-300">
                      TLS Pinned
                    </span>
                  )}
                  {probe?.meta && (
                    <span className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-muted">
                      v{probe.meta.server_version}
                    </span>
                  )}
                </div>
              </div>

              <div className="grid grid-cols-3 gap-1 rounded-lg bg-bg p-1 text-xs">
                <button
                  type="button"
                  onClick={() => {
                    setTab('pair')
                    setError(null)
                  }}
                  className={`flex items-center justify-center gap-1.5 rounded-md py-1.5 font-medium transition-colors ${
                    tab === 'pair' ? 'bg-surface-2 text-fg' : 'text-muted hover:text-fg'
                  }`}
                >
                  <KeyRound className="size-3.5" />
                  Pair Code
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setTab('password')
                    setError(null)
                  }}
                  className={`flex items-center justify-center gap-1.5 rounded-md py-1.5 font-medium transition-colors ${
                    tab === 'password' ? 'bg-surface-2 text-fg' : 'text-muted hover:text-fg'
                  }`}
                >
                  <Lock className="size-3.5" />
                  Password
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setTab('setup')
                    setError(null)
                  }}
                  className={`flex items-center justify-center gap-1.5 rounded-md py-1.5 font-medium transition-colors ${
                    tab === 'setup' ? 'bg-surface-2 text-fg' : 'text-muted hover:text-fg'
                  }`}
                >
                  <Sparkles className="size-3.5" />
                  Initialize
                </button>
              </div>

              {tab === 'pair' && (
                <form onSubmit={handlePairSubmit} className="space-y-4">
                  <div>
                    <label className="mb-1.5 block text-xs font-medium text-fg">
                      One-time pairing code
                    </label>
                    <input
                      type="text"
                      autoFocus
                      required
                      value={pairCode}
                      onChange={(e) => setPairCode(e.target.value.toUpperCase())}
                      placeholder="ABCD-EFGH"
                      className="w-full rounded-lg border border-line bg-bg px-3 py-2.5 text-center font-mono text-base tracking-[0.25em] text-fg uppercase outline-none focus:border-emerald-500"
                    />
                    <p className="mt-1.5 text-[11px] text-muted">
                      Generate on the server with{' '}
                      <code className="rounded bg-surface-2 px-1 py-0.5 font-mono text-fg">
                        rig devices pair
                      </code>{' '}
                      or{' '}
                      <code className="rounded bg-surface-2 px-1 py-0.5 font-mono text-fg">
                        rig-server devices pair
                      </code>
                      . Token is stored in your OS keychain.
                    </p>
                  </div>
                  <div className="flex justify-between pt-1">
                    <button
                      type="button"
                      onClick={() => setStep('url')}
                      className="rounded-lg px-3 py-2 text-xs text-muted hover:bg-surface-2 hover:text-fg"
                    >
                      Back
                    </button>
                    <button
                      type="submit"
                      disabled={busy}
                      className="rounded-lg bg-emerald-600 px-4 py-2 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
                    >
                      {busy ? 'Pairing…' : 'Pair Device'}
                    </button>
                  </div>
                </form>
              )}

              {tab === 'password' && (
                <form onSubmit={handleLoginSubmit} className="space-y-3">
                  <div>
                    <label className="mb-1 block text-xs font-medium text-fg">Email</label>
                    <input
                      type="email"
                      required
                      value={email}
                      onChange={(e) => setEmail(e.target.value)}
                      className="w-full rounded-lg border border-line bg-bg px-3 py-2 text-xs text-fg outline-none focus:border-emerald-500"
                    />
                  </div>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-fg">Password</label>
                    <input
                      type="password"
                      required
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      className="w-full rounded-lg border border-line bg-bg px-3 py-2 text-xs text-fg outline-none focus:border-emerald-500"
                    />
                  </div>
                  {challengeToken && (
                    <div>
                      <label className="mb-1 block text-xs font-medium text-fg">
                        Authenticator / Recovery Code
                      </label>
                      <input
                        type="text"
                        autoFocus
                        required
                        value={totpCode}
                        onChange={(e) => setTotpCode(e.target.value)}
                        placeholder="123456"
                        className="w-full rounded-lg border border-line bg-bg px-3 py-2 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                      />
                    </div>
                  )}
                  <div className="flex justify-between pt-2">
                    <button
                      type="button"
                      onClick={() => setStep('url')}
                      className="rounded-lg px-3 py-2 text-xs text-muted hover:bg-surface-2 hover:text-fg"
                    >
                      Back
                    </button>
                    <button
                      type="submit"
                      disabled={busy}
                      className="rounded-lg bg-emerald-600 px-4 py-2 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
                    >
                      {busy ? 'Signing in…' : 'Sign In & Pair'}
                    </button>
                  </div>
                </form>
              )}

              {tab === 'setup' && (
                <form onSubmit={handleSetupSubmit} className="space-y-2.5">
                  <p className="text-[11px] text-muted">
                    Initialize a brand-new or headless Rig server directly from Rig Desktop.
                  </p>
                  <div className="grid grid-cols-2 gap-2.5">
                    <div>
                      <label className="mb-1 block text-xs font-medium text-fg">Setup token</label>
                      <input
                        type="text"
                        required
                        value={setupToken}
                        onChange={(e) => setSetupToken(e.target.value)}
                        placeholder="from rig-server setup-token"
                        className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                      />
                    </div>
                    <div>
                      <label className="mb-1 block text-xs font-medium text-fg">Organization</label>
                      <input
                        type="text"
                        required
                        value={orgName}
                        onChange={(e) => setOrgName(e.target.value)}
                        placeholder="Acme Corp"
                        className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
                      />
                    </div>
                  </div>
                  <div className="grid grid-cols-2 gap-2.5">
                    <div>
                      <label className="mb-1 block text-xs font-medium text-fg">Owner name</label>
                      <input
                        type="text"
                        required
                        value={ownerName}
                        onChange={(e) => setOwnerName(e.target.value)}
                        placeholder="Ada Lovelace"
                        className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
                      />
                    </div>
                    <div>
                      <label className="mb-1 block text-xs font-medium text-fg">Owner email</label>
                      <input
                        type="email"
                        required
                        value={email}
                        onChange={(e) => setEmail(e.target.value)}
                        className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
                      />
                    </div>
                  </div>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-fg">
                      Owner password <span className="text-muted">(min 10 chars)</span>
                    </label>
                    <input
                      type="password"
                      required
                      minLength={10}
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
                    />
                  </div>
                  <div className="flex justify-between pt-2">
                    <button
                      type="button"
                      onClick={() => setStep('url')}
                      className="rounded-lg px-3 py-2 text-xs text-muted hover:bg-surface-2 hover:text-fg"
                    >
                      Back
                    </button>
                    <button
                      type="submit"
                      disabled={busy}
                      className="rounded-lg bg-emerald-600 px-4 py-2 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
                    >
                      {busy ? 'Initializing…' : 'Initialize & Connect'}
                    </button>
                  </div>
                </form>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
