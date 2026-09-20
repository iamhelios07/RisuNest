# Endpoint Registry

Endpoint Registry is a private Cloudflare Worker and D1 service that stores an encrypted RisuNest Sync server URL under a UUID. It does not store accounts, device tokens, decryption keys, or synchronization data.

## Local development

Requires Node.js 22.12 or newer and pnpm 10.34.1. Run from this directory:

```powershell
pnpm install --frozen-lockfile
pnpm run db:migrate
pnpm run dev
```

The local service listens on `http://localhost:8787` and stores its synthetic D1 state under `.wrangler/state/`. It does not require a Cloudflare login or remote database.

Verify the package with:

```powershell
pnpm run check
pnpm test
pnpm run format:check
pnpm run build
```

`build` is a Wrangler dry run and does not deploy.

## Service contract

The only route is `/endpoints/{uuid}`:

| Request                  | Result                                                   |
| ------------------------ | -------------------------------------------------------- |
| `POST /endpoints/{uuid}` | Stores one unpadded base64url envelope and returns `204` |
| `GET /endpoints/{uuid}`  | Returns the exact envelope as `text/plain`               |

UUIDs are normalized lowercase UUIDv4 values. Responses disable caching, browser CORS access is not provided, and records expire after 30 days without a successful POST. The Worker validates only envelope encoding and size. Native clients perform AES-256-GCM authentication and HTTPS URL validation.

The complete HTTP, encryption, storage, quota, and deployment contracts are maintained in:

- `docs/superpowers/specs/2026-09-09-sync-endpoint-registry-design.md`
- `docs/superpowers/plans/2026-09-09-sync-endpoint-registry.md`
- `docs/superpowers/plans/2026-09-13-endpoint-registry-server.md`

Deployment creates remote resources and must be separately authorized. Use `wrangler.example.jsonc` as the template, keep real resource IDs in ignored `wrangler.jsonc`, and authenticate Wrangler without committing credentials.
