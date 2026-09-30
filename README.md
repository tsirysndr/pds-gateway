# pds-gateway

A virtual AT Protocol PDS. It presents one hostname to the network and routes
every XRPC call to whichever Personal Data Server actually hosts the account —
the way `bsky.social` fronts a fleet of PDS instances.

Put it behind `rocksky.social` and it will route to the nodes behind it:

```
   clients, relays, appviews
              │
              ▼
   ┌─────────────────────────┐
   │ rocksky.social          │   owns the wildcard, answers handle
   │ pds-gateway :2583       │   resolution for the whole namespace
   └────────────┬────────────┘
                │
     ┌──────────┼───────────────┬────────────────────┐
     ▼          ▼               ▼                    ▼
  127.0.0.1   radxa.lan   raspberrypi4.lan   orangepi-zero-3w.lan
    :2584       :2583           :2583               :2583

  published as:
  rocksky.      radxa.      raspberrypi4.     orangepi-zero-3w.
  social        rocksky.    rocksky.social    rocksky.social
  (same host)   social
```

Built on tokio and axum. Tested against the [atoll](../atoll),
[clojure-pds](../clojure-pds) and [scala-pds](../scala-pds) implementations,
which share the same `com.atproto.*` surface.

## Contents

- [What it does](#what-it-does)
- [How routing works](#how-routing-works)
- [Handle collisions and the delegate protocol](#handle-collisions-and-the-delegate-protocol)
- [Account placement](#account-placement)
- [The firehose](#the-firehose)
- [Caching](#caching)
- [Quick start](#quick-start)
- [Configuration](#configuration)
- [Environment variables](#environment-variables)
- [Running the PDS on the same machine](#running-the-pds-on-the-same-machine)
- [Wiring the nodes to the gateway](#wiring-the-nodes-to-the-gateway)
- [Redis](#redis)
- [Admin API](#admin-api)
- [Observability](#observability)
- [Deployment](#deployment)
- [Security model](#security-model)
- [Development](#development)
- [Limitations](#limitations)

## What it does

- **Routes every XRPC method** to the node hosting the subject of the call,
  resolving handles and DIDs as needed and learning the mapping as it goes.
- **Owns the handle namespace.** It is the authority for `*.rocksky.social`, so
  two nodes can never issue the same handle.
- **Places new accounts** on a node by least-loaded, round-robin, weighted or
  pinned strategy, holding the handle for the duration of the signup.
- **Merges the firehose.** Every node's `subscribeRepos` stream becomes one
  stream under gateway-owned sequence numbers.
- **Answers as a delegate** for the PDS nodes, including the on-demand TLS
  check, so handles in the namespace can get certificates.
- **Caches aggressively** in-process, optionally shared through Redis, with
  request coalescing so a thundering herd on one handle makes one lookup.
- **Health-checks the fleet** and keeps new accounts off nodes that are down.
- **Streams blobs and CAR files** without buffering them.

## How routing works

Each XRPC method is classified once, in `src/routing/lexicon.rs`, by where its
routing subject lives. Sources are tried in order:

| Method group | Subject |
| --- | --- |
| `repo.getRecord`, `describeRepo`, `listRecords` | `?repo=`, then the token |
| `repo.createRecord`, `putRecord`, `deleteRecord`, `applyWrites` | the token, then `repo` in the body |
| `repo.uploadBlob`, `importRepo` | the token only (never buffered) |
| `sync.getRepo`, `getBlob`, `listBlobs`, … | `?did=` |
| `admin.*` | `?did=`, or `did` / `account` / `subject.did` in the body |
| `server.createSession` | `identifier` in the body, broadcast if it is an email |
| `identity.resolveHandle`, `server.describeServer`, `_health` | answered by the gateway |
| `sync.listRepos`, `listReposByCollection`, `admin.searchAccounts` | merged across every node |
| everything else | the token |

Once a subject is known, the gateway finds its node by:

1. **SQLite registry** — the mapping it already recorded. One local read.
2. **Delegate ask** — for a handle in its own namespace, the nodes are asked
   directly; each is authoritative for its own accounts. The answer is recorded.
3. **DID document** — resolve `did:plc` through the PLC directory or `did:web`
   over HTTPS, then match the `#atproto_pds` endpoint's host to a node. The
   result is recorded, so the gateway heals its own registry from the network.
4. **Token audience** — a session token names its issuing PDS in `aud`.
5. **`?node=`** — an explicit override, for debugging.
6. **Default node** — anything with no routable subject. A DID hosted outside
   the fleet also lands here, and that node federates to it as usual.

`X-Pdsgw-Node` on every proxied response names the node that served it.

## Handle collisions and the delegate protocol

One handle domain served by several PDS instances only works if something owns
the wildcard and answers for the whole namespace. That is the gateway's job, and
it is also how collisions are prevented.

atoll calls this a **delegate**: point each node at the gateway with

```sh
ATOLL_HANDLE_DELEGATES=https://rocksky.social
```

and before a node issues a hosted handle it asks the gateway
`GET /xrpc/com.atproto.identity.resolveHandle?handle=…`. A `200 {"did": …}` means
the name is taken. The gateway answers from its registry and, on a miss, asks
the other nodes, so whichever one holds the account answers.

The gateway serves the three endpoints a delegate and a TLS issuer expect:

| Endpoint | Answer |
| --- | --- |
| `GET /xrpc/com.atproto.identity.resolveHandle?handle=` | `200 {"did"}`, or `400 UnableToResolveHandle` |
| `GET /.well-known/atproto-did` (handle in `Host`) | `200 text/plain <did>`, or `404` |
| `GET /tls-check?domain=` | `200` if the name exists, `404` otherwise |

These run on the TLS handshake path, so `delegate.ask_timeout` must stay under
the caller's budget — atoll allows 2s to connect and 3s to answer, and the
default here is 1.5s. Nodes that fail or time out simply do not claim the
handle, so one node being down never makes a name look taken.

**Loop breaking.** The gateway marks its own fan-out requests with
`x-pdsgw-delegate-hop`, and holds an in-flight set of handles it is currently
asking about. A node that answers by asking the gateway back is answered from
the registry alone, so the question cannot bounce.

Account creation then goes through a two-phase reservation:

1. Validate the handle and refuse reserved labels.
2. Ask the nodes **live** — a cached answer could only ever say "taken", and
   would refuse a name that has since been given up.
3. Lock the handle in Redis (if configured), then reserve it in SQLite under a
   `UNIQUE` index. Both must succeed.
4. Forward the signup to the chosen node.
5. On success, turn the reservation into an account row in one transaction. On
   any failure, release it. Expired reservations are swept on a timer and
   cleared inline on the next attempt.

Concurrent signups for one handle therefore produce exactly one account; there
is a test for that.

## Account placement

`gateway.placement` picks the node for a new account among those that are
healthy, accept signups, and are under `max_accounts`:

- `least-accounts` — fewest accounts wins, counting reservations in flight so
  concurrent signups spread out. The default.
- `round-robin` — strict rotation, shared through Redis when configured.
- `weighted` — weighted draw using each node's `weight`.
- `pinned` — always `default_node`.

Set `accepts_signups = false` to let a small board keep serving the accounts it
already has without taking new ones.

## The firehose

Each node numbers its own events from 1, so the streams cannot simply be
concatenated. In `multiplex` mode (the default) the gateway assigns its own
monotonic `seq` to every frame and rewrites that one field, leaving the signed
commit blocks untouched. Per-node cursors and the gateway's high-water mark are
persisted, so a restart resumes rather than replays.

- `?cursor=` is served from an in-memory replay buffer; a cursor older than the
  buffer gets an `#info` / `OutdatedCursor` frame first, as the spec requires.
- A subscriber that falls further behind than `subscriber_queue` is
  disconnected rather than allowed to grow the buffer without bound.
- `?node=<name>` relays one node's stream verbatim, for debugging.
- `mode = "passthrough"` relays a single node; `mode = "off"` refuses with 501.

## Caching

Two tiers, with negative caching and single-flight coalescing:

| Cache | Default TTL |
| --- | --- |
| handle → DID | `5m` (`30s` for a miss) |
| DID document | `10m` |
| delegate claims | `5m`, positive only |
| handle → node | durable in SQLite |

L1 is in-process (`moka`); its `try_get_with` collapses concurrent lookups for
the same key into one upstream call. L2 is Redis when configured, so a fleet of
gateways warms one another. Misses expire sooner than hits, so a handle that has
just been published becomes visible quickly.

## Quick start

```sh
cargo build --release
cp gateway.example.toml gateway.toml   # then edit the [[nodes]] tables
./target/release/pds-gateway -c gateway.toml --check   # validate and exit
./target/release/pds-gateway -c gateway.toml
```

Without a config file it reads `GATEWAY_*` from the environment:

```sh
GATEWAY_BIND=0.0.0.0:2583 \
GATEWAY_PUBLIC_URL=https://rocksky.social \
GATEWAY_HANDLE_DOMAINS=rocksky.social \
GATEWAY_DEFAULT_NODE=local \
GATEWAY_NODES='local|http://127.0.0.1:2584|rocksky.social,radxa|http://radxa.lan:2583|radxa.rocksky.social' \
./target/release/pds-gateway
```

Check it is routing:

```sh
curl -s localhost:2583/health | jq
curl -s 'localhost:2583/xrpc/com.atproto.identity.resolveHandle?handle=alice.rocksky.social'
curl -si 'localhost:2583/xrpc/com.atproto.repo.getRecord?repo=alice.rocksky.social&collection=app.bsky.feed.post&rkey=1' | grep -i x-pdsgw-node
```

## Configuration

TOML overlaid with `GATEWAY_*` environment variables; **the environment always
wins**, so a baked-in config file can be overridden per deployment. Durations
accept `30` (seconds), `500ms`, `5m`, `1h30m`. See
[`gateway.example.toml`](gateway.example.toml) for every setting with comments.

The config is validated at startup and the process refuses to run with a fleet
that could route ambiguously — duplicate node names, two nodes sharing a
`public_host`, an unknown `default_node`, an empty `handle_domains`, or signups
enabled with no node willing to take them.

A node has two addresses, and keeping them separate is what makes the layout
above possible:

```toml
[[nodes]]
name = "radxa"
url = "http://radxa.lan:2583"          # how the gateway reaches it
public_host = "radxa.rocksky.social"   # what it publishes in DID documents
did = "did:web:radxa.rocksky.social"   # lets the gateway route on a token's aud
weight = 2
accepts_signups = true
max_accounts = 5000
```

## Environment variables

Every setting has an equivalent. The full list:

| Variable | Section |
| --- | --- |
| `GATEWAY_CONFIG` | path to the TOML file |
| `GATEWAY_BIND`, `GATEWAY_PUBLIC_URL`, `GATEWAY_DID` | `[server]` |
| `GATEWAY_TRUST_FORWARDED_HEADERS`, `GATEWAY_SHUTDOWN_GRACE` | `[server]` |
| `GATEWAY_HANDLE_DOMAINS`, `GATEWAY_RESERVED_HANDLES` | `[gateway]` |
| `GATEWAY_PLACEMENT`, `GATEWAY_DEFAULT_NODE` | `[gateway]` |
| `GATEWAY_RESERVATION_TTL`, `GATEWAY_ALLOW_SIGNUPS` | `[gateway]` |
| `GATEWAY_BROADCAST_LOGIN`, `GATEWAY_HONOR_PROXY_HEADER` | `[gateway]` |
| `GATEWAY_DELEGATE_ENABLED`, `GATEWAY_DELEGATE_FAN_OUT` | `[delegate]` |
| `GATEWAY_DELEGATE_ASK_TIMEOUT`, `GATEWAY_DELEGATE_CACHE_TTL` | `[delegate]` |
| `GATEWAY_DELEGATE_TLS_CHECK` | `[delegate]` |
| `GATEWAY_PLC_DIRECTORY_URL`, `GATEWAY_CACHE_CAPACITY` | `[identity]` |
| `GATEWAY_HANDLE_CACHE_TTL`, `GATEWAY_HANDLE_NEGATIVE_CACHE_TTL` | `[identity]` |
| `GATEWAY_DID_CACHE_TTL`, `GATEWAY_ROUTE_CACHE_TTL` | `[identity]` |
| `GATEWAY_DNS_RESOLUTION`, `GATEWAY_DNS_NAMESERVERS` | `[identity]` |
| `GATEWAY_WELL_KNOWN_RESOLUTION` | `[identity]` |
| `GATEWAY_UPSTREAM_CONNECT_TIMEOUT`, `GATEWAY_UPSTREAM_REQUEST_TIMEOUT` | `[upstream]` |
| `GATEWAY_UPSTREAM_TRANSFER_TIMEOUT`, `GATEWAY_UPSTREAM_POOL_IDLE_TIMEOUT` | `[upstream]` |
| `GATEWAY_UPSTREAM_POOL_MAX_IDLE_PER_HOST`, `GATEWAY_MAX_BUFFERED_BODY_BYTES` | `[upstream]` |
| `GATEWAY_HEALTH_INTERVAL`, `GATEWAY_HEALTH_TIMEOUT` | `[health]` |
| `GATEWAY_HEALTH_FAILURE_THRESHOLD`, `GATEWAY_HEALTH_SUCCESS_THRESHOLD` | `[health]` |
| `GATEWAY_HEALTH_PROBE_PATH`, `GATEWAY_ROUTE_TO_UNHEALTHY` | `[health]` |
| `GATEWAY_FIREHOSE_MODE`, `GATEWAY_FIREHOSE_REPLAY_BUFFER` | `[firehose]` |
| `GATEWAY_FIREHOSE_SUBSCRIBER_QUEUE` | `[firehose]` |
| `GATEWAY_FIREHOSE_RECONNECT_MIN_BACKOFF`, `..._MAX_BACKOFF` | `[firehose]` |
| `GATEWAY_STORE_PATH`, `GATEWAY_STORE_MAX_CONNECTIONS` | `[store]` |
| `GATEWAY_STORE_SWEEP_INTERVAL` | `[store]` |
| `GATEWAY_REDIS_URL`, `GATEWAY_REDIS_KEY_PREFIX` | `[redis]` |
| `GATEWAY_REDIS_FAIL_OPEN`, `GATEWAY_REDIS_CONNECT_TIMEOUT` | `[redis]` |
| `GATEWAY_REDIS_RESPONSE_TIMEOUT` | `[redis]` |
| `GATEWAY_ADMIN_TOKEN`, `GATEWAY_METRICS` | `[admin]` |
| `GATEWAY_NODES` | the fleet, see below |
| `GATEWAY_LOG`, `GATEWAY_LOG_JSON` | logging |

`GATEWAY_NODES` is a compact spelling of the `[[nodes]]` tables so a whole fleet
fits in one variable. Fields are `name|url|public_host|weight|accepts_signups|did`,
nodes separated by commas; only `name|url` is required:

```sh
GATEWAY_NODES='local|http://127.0.0.1:2584|rocksky.social|1|true|did:web:rocksky.social,
radxa|http://radxa.lan:2583|radxa.rocksky.social|2|true,
orangepi-zero-3w|http://orangepi-zero-3w.lan:2583|orangepi-zero-3w.rocksky.social|1|false'
```

List-valued variables (`GATEWAY_HANDLE_DOMAINS`, `GATEWAY_RESERVED_HANDLES`,
`GATEWAY_DNS_NAMESERVERS`) are comma-separated. An empty value is treated as
unset, so `GATEWAY_FOO=` in a compose file does not override the file.

## Running the PDS on the same machine

The gateway takes the public hostname, so the real PDS sharing that machine
moves to a loopback port. It is configured like any other node — it just has a
private `url` and the bare domain as its `public_host`:

```toml
[[nodes]]
name = "local"
url = "http://127.0.0.1:2584"     # the PDS, bound to loopback only
public_host = "rocksky.social"    # what it publishes in DID documents
did = "did:web:rocksky.social"
```

For atoll that means:

```sh
ATOLL_LISTEN_IP=127.0.0.1
PORT=2584
PHX_HOST=rocksky.social           # DID documents must name the public host
ATOLL_HANDLE_DELEGATES=http://127.0.0.1:2583
```

`public_host` is what the gateway matches DID documents against, so it must be
what the PDS actually publishes — the gateway refuses to start if two nodes
claim the same one.

## Wiring the nodes to the gateway

On every PDS in the fleet:

```sh
# The gateway owns the wildcard and answers for the whole namespace.
ATOLL_HANDLE_DELEGATES=https://rocksky.social
# DID documents must name the node's own public host.
PHX_HOST=radxa.rocksky.social
```

DNS: point `rocksky.social` and `*.rocksky.social` at the gateway. User handles
resolve through the gateway, which is what lets any node host any handle in the
namespace. The nodes' own hostnames point at the nodes.

Each node keeps its own signing keys and its own session secrets. The gateway
never holds account credentials.

## Redis

Optional. Leave `[redis].url` unset for a single gateway: SQLite is the source
of truth and caches live in-process, with no extra service to run.

Set it to run several gateway replicas, which then share:

- the handle/DID resolution cache (L2 behind each replica's in-process cache),
- handle locks, so two replicas cannot both pass the collision check,
- the round-robin placement cursor.

`fail_open = true` (the default) degrades to local-only behaviour when Redis is
unreachable rather than failing requests — the durable SQLite `UNIQUE` index is
still enforced behind the lock, so failing open costs the distributed fast path,
not correctness.

## Admin API

Enabled by setting `admin.token`; unset disables it entirely. All routes take
`Authorization: Bearer <token>`, compared in constant time.

| Route | Purpose |
| --- | --- |
| `GET /admin/nodes` | fleet state: health, latency, account counts |
| `GET /admin/accounts?node=&cursor=&limit=` | registry contents, keyset paginated |
| `DELETE /admin/accounts/{did}` | forget a mapping; the account is untouched |
| `GET /admin/reservations` | handles held by in-flight signups |
| `DELETE /admin/reservations/{handle}` | release a stuck reservation |
| `GET /admin/resolve?subject=&live=true` | explain where a subject routes, and why |
| `POST /admin/cache/purge` | drop cached entries for a `handle` or `did` |
| `POST /admin/nodes/{from}/drain/{to}` | reassign every account after a migration |

`/admin/resolve` is the one to reach for when a request lands on the wrong node:

```sh
curl -s -H "Authorization: Bearer $TOKEN" \
  'localhost:2583/admin/resolve?subject=alice.rocksky.social&live=true' | jq
```

## Observability

- `GET /health` — per-node health, latency, last error, firehose position.
- `GET /health/ready` — `503` until at least one node is up. Use for readiness.
- `GET /metrics` — Prometheus text, when `admin.metrics = true`. Counters for
  requests, proxied calls, cache hits and misses, resolutions, accounts created,
  handle collisions, upstream and Redis errors, firehose frames and lagged
  subscribers; gauges for node health and per-node account counts.

Logging is `tracing`, filtered with `GATEWAY_LOG` (or `RUST_LOG`), and
`--log-json` / `GATEWAY_LOG_JSON=1` for structured output.

```sh
GATEWAY_LOG=pds_gateway=debug ./target/release/pds-gateway -c gateway.toml
```

## Deployment

Terminate TLS in front of the gateway with a certificate covering
`rocksky.social` and `*.rocksky.social`, or use on-demand TLS pointed at
`/tls-check`. Keep `trust_forwarded_headers = true` so client addresses are
forwarded, and make sure the proxy sets `X-Forwarded-For` itself — the gateway
strips any the client sent.

```ini
# /etc/systemd/system/pds-gateway.service
[Unit]
Description=pds-gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pds
WorkingDirectory=/var/lib/pds-gateway
Environment=GATEWAY_CONFIG=/etc/pds-gateway/gateway.toml
ExecStart=/usr/local/bin/pds-gateway
Restart=on-failure
RestartSec=2
StateDirectory=pds-gateway
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

The SQLite database is small but is the registry — back it up. Losing it is
recoverable (the gateway re-learns mappings from DID documents and delegate
asks) but costs a resolution per account and loses the firehose cursors.

## Security model

The gateway **cannot verify session tokens**, and does not try to. Each PDS
signs sessions with its own secret — atoll uses HS256 with a per-node key and
its own DID as the audience — so only the issuing node can check them. The
gateway reads `sub` and `aud` from the token payload **without verifying the
signature**, uses them to pick a node, and forwards the original
`Authorization` header. Every authorization decision still happens on the
upstream PDS, which does verify. A forged token can at worst choose which node
rejects it.

Other boundaries:

- Hop-by-hop headers and any client-supplied `X-Forwarded-*` are stripped before
  forwarding.
- Upstream redirects are not followed, so a node cannot move a request somewhere
  the gateway did not choose.
- Only bodies that must be read for routing are buffered, capped by
  `max_buffered_body_bytes`; blobs and CAR imports stream.
- `atproto-proxy` is honoured only when it names one of the configured nodes;
  foreign targets are left to the upstream PDS.
- Internal errors are logged in full and reported generically, so upstream
  internals are not echoed to clients.
- The admin token is compared in constant time, and an empty token is rejected
  at startup rather than accepting every request.

## Development

```sh
cargo build --release
cargo test --release            # 93 unit + 29 integration tests
cargo clippy --all-targets
cargo fmt
```

The integration suite in `tests/` starts stub PDS nodes on real sockets and
drives the full gateway, covering delegate resolution, handle collisions,
concurrent signups for one handle, placement, login broadcast, fan-out merging,
and the admin API.

Layout:

```
src/
  config.rs          TOML + GATEWAY_* env, validation
  state.rs           shared state and startup wiring
  error.rs           errors and their XRPC wire form
  identity/
    handle.rs did.rs syntax per the atproto specs
    resolver.rs      handle/DID resolution, two-tier cache
    delegate.rs      the gateway as a handle-resolution delegate
  registry/
    store.rs         SQLite: accounts, reservations, cursors
    coord.rs         Redis or in-process coordination
  routing/
    lexicon.rs       which part of a request identifies the account
    auth.rs          unverified token claims, for routing only
    router.rs        subject -> node, and placement
  proxy/
    forward.rs       streaming HTTP forwarding, fan-out
    ws.rs            websocket relay
  firehose.rs        merging subscribeRepos, seq rewriting
  health.rs          node probes, rotation counter
  api/               xrpc, wellknown, subscribe, admin, describe
```

## Limitations

- **Firehose replay is bounded by memory.** Cursors and the sequence high-water
  mark survive a restart, but the replay buffer does not, so a subscriber
  reconnecting after a gateway restart gets `OutdatedCursor` and resumes from
  the oldest frame then available. Point relays at the gateway, not at
  individual nodes, so sequence numbers stay consistent.
- **Fan-out results are not paginated.** `listRepos` and friends merge one page
  per node and drop the per-node cursors rather than returning a merged cursor
  that would be a lie. Fine for a handful of nodes; not a relay.
- **The gateway holds no accounts.** It does not issue tokens, run OAuth, or
  sign anything; it routes. Account-holding endpoints are served by the nodes.
  A full entryway that owns accounts is a different, much larger thing.
- **Account migration between nodes** is not automated. Move the repository with
  the PDS's own import/export, then `POST /admin/nodes/{from}/drain/{to}` or
  `DELETE /admin/accounts/{did}` to correct the registry.

## License

MIT
