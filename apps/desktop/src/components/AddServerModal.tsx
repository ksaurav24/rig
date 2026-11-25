import {
  ArrowRight,
  KeyRound,
  Lock,
  Server,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
} from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription, BannerTitle } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
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
import { Tabs, TabsItem } from '@rig/ui/tabs'
import { desktopApi } from '@/lib/ipc'
import type { ProbeServerResult, ServerView } from '@/lib/types'

interface AddServerModalProps {
  open: boolean
  onClose: () => void
  onAdded: (server: ServerView) => void
}

type AuthTab = 'pair' | 'password' | 'setup'
const AUTH_TABS: AuthTab[] = ['pair', 'password', 'setup']

export function AddServerModal({ open, onClose, onAdded }: AddServerModalProps) {
  const [step, setStep] = useState<'url' | 'tls' | 'auth'>('url')
  const [url, setUrl] = useState('')
  const [serverName, setServerName] = useState('')
  const [probe, setProbe] = useState<ProbeServerResult | null>(null)
  const [pinnedFp, setPinnedFp] = useState<string | null>(null)
  const [tab, setTab] = useState<AuthTab>('pair')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const [pairCode, setPairCode] = useState('')
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [totpCode, setTotpCode] = useState('')
  const [challengeToken, setChallengeToken] = useState<string | null>(null)

  const [setupToken, setSetupToken] = useState('')
  const [orgName, setOrgName] = useState('')
  const [ownerName, setOwnerName] = useState('')

  function reset() {
    setStep('url')
    setProbe(null)
    setPinnedFp(null)
    setError(null)
    setPairCode('')
    setTotpCode('')
    setChallengeToken(null)
  }

  function handleClose() {
    reset()
    onClose()
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
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) handleClose()
      }}
    >
      <DialogContent size="lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2.5">
            <Server className="size-4.5 text-primary" />
            Add Rig Server
          </DialogTitle>
          <DialogDescription>
            Connect public HTTPS, LAN (internal CA), or Tailscale servers.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {error && (
            <Banner role="alert" status="error" contrast="high" size="compact">
              <BannerDescription>{error}</BannerDescription>
            </Banner>
          )}

          {step === 'url' && (
            <form onSubmit={handleUrlSubmit} className="space-y-4">
              <Field>
                <FieldLabel htmlFor="srv-url">Server URL</FieldLabel>
                <Input
                  id="srv-url"
                  type="text"
                  autoFocus
                  required
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://rig.example.com or http://127.0.0.1:7460"
                  className="font-mono"
                />
                <FieldDescription>
                  Supports domain HTTPS, LAN internal CA (with fingerprint pinning), and Tailscale
                  MagicDNS.
                </FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="srv-name">
                  Display Name <span className="opacity-70">(optional)</span>
                </FieldLabel>
                <Input
                  id="srv-name"
                  type="text"
                  value={serverName}
                  onChange={(e) => setServerName(e.target.value)}
                  placeholder="Production EU, Homelab, Staging…"
                />
              </Field>
              <DialogFooter>
                <Button type="button" variant="ghost" onClick={handleClose}>
                  Cancel
                </Button>
                <Button type="submit" variant="primary" trailingIcon={ArrowRight} loading={busy}>
                  Continue
                </Button>
              </DialogFooter>
            </form>
          )}

          {step === 'tls' && probe && (
            <div className="space-y-4">
              <Banner status="warning" contrast="high" icon={ShieldAlert}>
                <BannerTitle>Self-signed / Internal CA Certificate</BannerTitle>
                <BannerDescription>
                  Verify this SHA-256 fingerprint matches your LAN server before pinning it. All
                  future requests, SSE streams, and terminals will require this exact certificate.
                </BannerDescription>
              </Banner>

              <div className="rounded-lg border border-border bg-surface p-3.5">
                <div className="mb-1.5 text-[11.5px] font-medium tracking-wider text-muted-foreground uppercase">
                  Leaf Certificate SHA-256
                </div>
                <code className="block break-all font-mono text-[12.5px] leading-relaxed text-ok">
                  {probe.tls.fingerprint_sha256 ?? 'Unavailable'}
                </code>
              </div>

              <DialogFooter className="justify-between">
                <Button type="button" variant="ghost" onClick={() => setStep('url')}>
                  Back
                </Button>
                <Button
                  type="button"
                  variant="primary"
                  leadingIcon={ShieldCheck}
                  loading={busy}
                  onClick={handleTrustFingerprint}
                >
                  Trust & Pin Certificate
                </Button>
              </DialogFooter>
            </div>
          )}

          {step === 'auth' && (
            <div className="space-y-4">
              <div className="flex items-center justify-between gap-2 rounded-lg border border-border bg-surface px-3.5 py-2.5 text-[13px]">
                <div className="min-w-0 truncate font-mono text-foreground">{url}</div>
                <div className="flex shrink-0 items-center gap-1.5">
                  {pinnedFp && (
                    <Badge variant="solid" color="emerald" size="compact">
                      TLS Pinned
                    </Badge>
                  )}
                  {probe?.meta && (
                    <Badge variant="solid" color="gray" size="compact" className="font-mono">
                      v{probe.meta.server_version}
                    </Badge>
                  )}
                </div>
              </div>

              <div className="rounded-xl border border-border/80 bg-surface p-1">
                <Tabs
                  className="mx-0 my-0 w-full max-w-full px-0 py-0"
                  selectedIndex={AUTH_TABS.indexOf(tab)}
                  onSelect={(idx) => {
                    setTab(AUTH_TABS[idx] ?? 'pair')
                    setError(null)
                  }}
                >
                  <TabsItem index={0} icon={KeyRound} label="Pair Code" className="flex-1 justify-center" />
                  <TabsItem index={1} icon={Lock} label="Password" className="flex-1 justify-center" />
                  <TabsItem index={2} icon={Sparkles} label="Initialize" className="flex-1 justify-center" />
                </Tabs>
              </div>

              {tab === 'pair' && (
                <form onSubmit={handlePairSubmit} className="space-y-4">
                  <Field>
                    <FieldLabel htmlFor="pair-code">One-time Pairing Code</FieldLabel>
                    <Input
                      id="pair-code"
                      type="text"
                      autoFocus
                      required
                      value={pairCode}
                      onChange={(e) => setPairCode(e.target.value.toUpperCase())}
                      placeholder="ABCD-EFGH"
                      className="h-11 text-center font-mono text-base tracking-[0.25em] uppercase"
                    />
                    <FieldDescription>
                      Generate on the server with{' '}
                      <code className="rounded bg-surface px-1.5 py-0.5 font-mono text-foreground">
                        rig devices pair
                      </code>{' '}
                      or{' '}
                      <code className="rounded bg-surface px-1.5 py-0.5 font-mono text-foreground">
                        rig-server devices pair
                      </code>
                      . Token is stored in your OS keychain.
                    </FieldDescription>
                  </Field>
                  <DialogFooter className="justify-between">
                    <Button type="button" variant="ghost" onClick={() => setStep('url')}>
                      Back
                    </Button>
                    <Button type="submit" variant="primary" loading={busy}>
                      Pair Device
                    </Button>
                  </DialogFooter>
                </form>
              )}

              {tab === 'password' && (
                <form onSubmit={handleLoginSubmit} className="space-y-3.5">
                  <Field>
                    <FieldLabel htmlFor="login-email">Email</FieldLabel>
                    <Input
                      id="login-email"
                      type="email"
                      required
                      value={email}
                      onChange={(e) => setEmail(e.target.value)}
                    />
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="login-pass">Password</FieldLabel>
                    <Input
                      id="login-pass"
                      type="password"
                      required
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                    />
                  </Field>
                  {challengeToken && (
                    <Field>
                      <FieldLabel htmlFor="login-totp">Authenticator / Recovery Code</FieldLabel>
                      <Input
                        id="login-totp"
                        type="text"
                        autoFocus
                        required
                        value={totpCode}
                        onChange={(e) => setTotpCode(e.target.value)}
                        placeholder="123456"
                        className="font-mono"
                      />
                    </Field>
                  )}
                  <DialogFooter className="justify-between">
                    <Button type="button" variant="ghost" onClick={() => setStep('url')}>
                      Back
                    </Button>
                    <Button type="submit" variant="primary" loading={busy}>
                      Sign In & Pair
                    </Button>
                  </DialogFooter>
                </form>
              )}

              {tab === 'setup' && (
                <form onSubmit={handleSetupSubmit} className="space-y-3.5">
                  <p className="text-[13px] text-muted-foreground">
                    Initialize a brand-new or headless Rig server directly from Rig Desktop.
                  </p>
                  <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                    <Field>
                      <FieldLabel htmlFor="setup-tok">Setup Token</FieldLabel>
                      <Input
                        id="setup-tok"
                        type="text"
                        required
                        value={setupToken}
                        onChange={(e) => setSetupToken(e.target.value)}
                        placeholder="from rig-server setup-token"
                        className="font-mono"
                      />
                    </Field>
                    <Field>
                      <FieldLabel htmlFor="setup-org">Organization</FieldLabel>
                      <Input
                        id="setup-org"
                        type="text"
                        required
                        value={orgName}
                        onChange={(e) => setOrgName(e.target.value)}
                        placeholder="Acme Corp"
                      />
                    </Field>
                    <Field>
                      <FieldLabel htmlFor="setup-owner">Owner Name</FieldLabel>
                      <Input
                        id="setup-owner"
                        type="text"
                        required
                        value={ownerName}
                        onChange={(e) => setOwnerName(e.target.value)}
                        placeholder="Ada Lovelace"
                      />
                    </Field>
                    <Field>
                      <FieldLabel htmlFor="setup-email">Owner Email</FieldLabel>
                      <Input
                        id="setup-email"
                        type="email"
                        required
                        value={email}
                        onChange={(e) => setEmail(e.target.value)}
                      />
                    </Field>
                  </div>
                  <Field>
                    <FieldLabel htmlFor="setup-pass">
                      Owner Password <span className="opacity-70">(min 10 chars)</span>
                    </FieldLabel>
                    <Input
                      id="setup-pass"
                      type="password"
                      required
                      minLength={10}
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                    />
                  </Field>
                  <DialogFooter className="justify-between">
                    <Button type="button" variant="ghost" onClick={() => setStep('url')}>
                      Back
                    </Button>
                    <Button type="submit" variant="primary" loading={busy}>
                      Initialize & Connect
                    </Button>
                  </DialogFooter>
                </form>
              )}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
