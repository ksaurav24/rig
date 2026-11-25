export type CatalogCategory =
  | 'Analytics'
  | 'Automation'
  | 'Developer tools'
  | 'Media'
  | 'Monitoring'
  | 'Productivity'
  | 'Web'

export interface CatalogVariable {
  key: string
  /** Default value. Omit when `generate` is set or the user must supply it. */
  value?: string
  /** Fill with a random 32-char URL-safe string when the app is created. */
  generate?: 'password'
  secret?: boolean
  /** Ask the user for a value when true and no value/generate is given. */
  required?: boolean
  /** One short sentence shown under the input. */
  description?: string
}

export interface CatalogTemplate {
  id: string
  name: string
  description: string
  category: CatalogCategory
  image: string
  port: number
  healthPath: string | null
  memoryMb: number
  volumes: string[]
  variables: CatalogVariable[]
  website: string
}

export const CATALOG: readonly CatalogTemplate[] = [
  {
    id: 'uptime-kuma',
    name: 'Uptime Kuma',
    description: 'Self-hosted uptime monitor with status pages and notifications.',
    category: 'Monitoring',
    image: 'louislam/uptime-kuma:2',
    port: 3001,
    healthPath: '/',
    memoryMb: 512,
    volumes: ['/app/data'],
    variables: [],
    website: 'https://uptime.kuma.pet',
  },
  {
    id: 'gotify',
    name: 'Gotify',
    description: 'Simple server for sending and receiving push notifications over HTTP and WebSocket.',
    category: 'Monitoring',
    image: 'gotify/server:3',
    port: 80,
    healthPath: '/health',
    memoryMb: 128,
    volumes: ['/app/data'],
    variables: [
      {
        key: 'GOTIFY_DEFAULTUSER_PASS',
        generate: 'password',
        secret: true,
        description: 'Password for the initial admin user (name: admin).',
      },
    ],
    website: 'https://gotify.net',
  },
  {
    id: 'vaultwarden',
    name: 'Vaultwarden',
    description: 'Lightweight server compatible with the Bitwarden client apps.',
    category: 'Productivity',
    image: 'vaultwarden/server:1.37.5',
    port: 80,
    healthPath: '/alive',
    memoryMb: 256,
    volumes: ['/data'],
    variables: [
      {
        key: 'ADMIN_TOKEN',
        generate: 'password',
        secret: true,
        description: 'Token that unlocks the /admin page.',
      },
    ],
    website: 'https://github.com/dani-garcia/vaultwarden',
  },
  {
    id: 'n8n',
    name: 'n8n',
    description: 'Workflow automation tool with a visual editor and many integrations.',
    category: 'Automation',
    image: 'n8nio/n8n:1',
    port: 5678,
    healthPath: '/healthz',
    memoryMb: 1024,
    volumes: ['/home/node/.n8n'],
    variables: [
      {
        key: 'N8N_ENCRYPTION_KEY',
        generate: 'password',
        secret: true,
        description: 'Encrypts stored credentials. Keep it; changing it breaks saved credentials.',
      },
    ],
    website: 'https://n8n.io',
  },
  {
    id: 'gitea',
    name: 'Gitea',
    description: 'Self-hosted Git service with issues, pull requests and a package registry.',
    category: 'Developer tools',
    image: 'gitea/gitea:28',
    port: 3000,
    healthPath: '/api/healthz',
    memoryMb: 512,
    volumes: ['/data'],
    variables: [{ key: 'GITEA__database__DB_TYPE', value: 'sqlite3' }],
    website: 'https://about.gitea.com',
  },
  {
    id: 'code-server',
    name: 'code-server',
    description: 'VS Code running on a server and accessed from the browser.',
    category: 'Developer tools',
    image: 'codercom/code-server:4.145.2',
    port: 8080,
    healthPath: '/healthz',
    memoryMb: 1024,
    volumes: ['/home/coder'],
    variables: [
      {
        key: 'PASSWORD',
        generate: 'password',
        secret: true,
        description: 'Password for the login page.',
      },
    ],
    website: 'https://coder.com/docs/code-server',
  },
  {
    id: 'memos',
    name: 'Memos',
    description: 'Note-taking service for short text notes, stored in SQLite.',
    category: 'Productivity',
    image: 'neosmemo/memos:stable',
    port: 5230,
    healthPath: '/healthz',
    memoryMb: 256,
    volumes: ['/var/opt/memos'],
    variables: [],
    website: 'https://usememos.com',
  },
  {
    id: 'linkding',
    name: 'linkding',
    description: 'Bookmark manager with tagging and full-text search, stored in SQLite.',
    category: 'Productivity',
    image: 'sissbruecker/linkding:1.47.0',
    port: 9090,
    healthPath: '/health',
    memoryMb: 256,
    volumes: ['/etc/linkding/data'],
    variables: [
      { key: 'LD_SUPERUSER_NAME', value: 'admin', description: 'Username of the initial admin user.' },
      {
        key: 'LD_SUPERUSER_PASSWORD',
        generate: 'password',
        secret: true,
        description: 'Password of the initial admin user.',
      },
    ],
    website: 'https://linkding.link',
  },
  {
    id: 'actual-budget',
    name: 'Actual Budget',
    description: 'Local-first personal finance app with an optional sync server.',
    category: 'Productivity',
    image: 'actualbudget/actual-server:26.11.0',
    port: 5006,
    healthPath: '/health',
    memoryMb: 512,
    volumes: ['/data'],
    variables: [],
    website: 'https://actualbudget.org',
  },
  {
    id: 'freshrss',
    name: 'FreshRSS',
    description: 'Self-hosted RSS feed aggregator and reader.',
    category: 'Productivity',
    image: 'freshrss/freshrss:1',
    port: 80,
    healthPath: '/',
    memoryMb: 256,
    volumes: ['/var/www/FreshRSS/data', '/var/www/FreshRSS/extensions'],
    variables: [],
    website: 'https://freshrss.org',
  },
  {
    id: 'mealie',
    name: 'Mealie',
    description: 'Recipe manager and meal planner, stored in SQLite.',
    category: 'Productivity',
    image: 'ghcr.io/mealie-recipes/mealie:v3.31.0',
    port: 9000,
    healthPath: '/api/app/about',
    memoryMb: 1024,
    volumes: ['/app/data'],
    variables: [],
    website: 'https://mealie.io',
  },
  {
    id: 'filebrowser',
    name: 'File Browser',
    description: 'Web file manager for browsing, uploading and sharing files.',
    category: 'Productivity',
    image: 'filebrowser/filebrowser:v2',
    port: 80,
    healthPath: '/health',
    memoryMb: 128,
    volumes: ['/srv', '/database', '/config'],
    variables: [],
    website: 'https://filebrowser.org',
  },
  {
    id: 'stirling-pdf',
    name: 'Stirling-PDF',
    description: 'Web tool for merging, splitting, converting and editing PDF files.',
    category: 'Productivity',
    image: 'stirlingtools/stirling-pdf:3.4.0',
    port: 8080,
    healthPath: '/api/v1/info/status',
    memoryMb: 1024,
    volumes: ['/configs'],
    variables: [],
    website: 'https://www.stirlingpdf.com',
  },
  {
    id: 'jellyfin',
    name: 'Jellyfin',
    description: 'Media server for streaming movies, shows, music and photos.',
    category: 'Media',
    image: 'jellyfin/jellyfin:12',
    port: 8096,
    healthPath: '/health',
    memoryMb: 1024,
    volumes: ['/config', '/cache'],
    variables: [],
    website: 'https://jellyfin.org',
  },
  {
    id: 'navidrome',
    name: 'Navidrome',
    description: 'Music streaming server compatible with Subsonic clients.',
    category: 'Media',
    image: 'deluan/navidrome:0.64.2',
    port: 4533,
    healthPath: '/ping',
    memoryMb: 256,
    volumes: ['/data', '/music'],
    variables: [],
    website: 'https://www.navidrome.org',
  },
  {
    id: 'excalidraw',
    name: 'Excalidraw',
    description: 'Virtual whiteboard for sketching hand-drawn style diagrams.',
    category: 'Web',
    image: 'excalidraw/excalidraw:latest',
    port: 80,
    healthPath: '/',
    memoryMb: 128,
    volumes: [],
    variables: [],
    website: 'https://excalidraw.com',
  },
  {
    id: 'it-tools',
    name: 'IT-Tools',
    description: 'Collection of browser-based utilities for developers and sysadmins.',
    category: 'Developer tools',
    image: 'corentinth/it-tools:latest',
    port: 80,
    healthPath: '/',
    memoryMb: 128,
    volumes: [],
    variables: [],
    website: 'https://it-tools.tech',
  },
]

export const CATALOG_CATEGORIES: readonly CatalogCategory[] = [
  'Productivity',
  'Developer tools',
  'Monitoring',
  'Automation',
  'Media',
  'Web',
]

export function generateSecret(length = 32): string {
  const alphabet = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'
  const buf = new Uint8Array(length * 2)
  crypto.getRandomValues(buf)
  let out = ''
  for (let i = 0; i < buf.length && out.length < length; i++) {
    const v = buf[i]!
    if (v < 248) out += alphabet[v % alphabet.length]
  }
  while (out.length < length) {
    out += alphabet[Math.floor(Math.random() * alphabet.length)]
  }
  return out
}
