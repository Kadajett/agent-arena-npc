/**
 * A read-only view onto what the aggregator has found, for a human or a
 * future dashboard to query directly instead of reaching into the
 * container's own volume with `docker exec`. No auth: there is nothing
 * here that is not already public somewhere in the pipeline that produced
 * it - see the header of worldledger.ts for what "public" means for every
 * input this table is built from.
 *
 * Bound to localhost on the host by default (see the worldledger service's
 * `ports:` entry in docker-compose.yml) - queryable from this machine, not
 * exposed to the internet, until that is deliberately widened.
 */

import { createServer } from 'node:http';
import type { Server } from 'node:http';
import { loadClaims } from './store.js';
import type { Authenticity, Tier } from './graph.js';

const DEFAULT_LIMIT = 100;
const MAX_LIMIT = 500;

const TIERS: Tier[] = ['read', 'possibly-true', 'probably-seeded', 'overheard', 'unknown'];
const AUTHENTICITIES: Authenticity[] = ['stable', 'drifting', 'contradicted', 'unexamined'];

function json(res: import('node:http').ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { 'content-type': 'application/json' });
  res.end(JSON.stringify(body));
}

export function startServer(memoryDir: string, port: number): Server {
  const server = createServer((req, res) => {
    if ('GET' !== req.method) {
      json(res, 405, { error: 'METHOD_NOT_ALLOWED' });
      return;
    }
    const url = new URL(req.url ?? '/', 'http://localhost');

    if ('/health' === url.pathname) {
      json(res, 200, { ok: true, claims: loadClaims(memoryDir).length });
      return;
    }

    if ('/claims' === url.pathname) {
      let claims = loadClaims(memoryDir);

      const tier = url.searchParams.get('tier');
      if (null !== tier) {
        if (!(TIERS as string[]).includes(tier)) {
          json(res, 400, { error: 'UNKNOWN_TIER', allowed: TIERS });
          return;
        }
        claims = claims.filter((claim) => claim.tier === tier);
      }

      const authenticity = url.searchParams.get('authenticity');
      if (null !== authenticity) {
        if (!(AUTHENTICITIES as string[]).includes(authenticity)) {
          json(res, 400, { error: 'UNKNOWN_AUTHENTICITY', allowed: AUTHENTICITIES });
          return;
        }
        claims = claims.filter((claim) => claim.authenticity === authenticity);
      }

      const requested = Number(url.searchParams.get('limit'));
      const limit = Math.max(1, Math.min(Number.isFinite(requested) && requested > 0 ? requested : DEFAULT_LIMIT, MAX_LIMIT));
      const page = [...claims].sort((left, right) => right.lastSeen.localeCompare(left.lastSeen)).slice(0, limit);

      json(res, 200, { generatedAt: new Date().toISOString(), total: claims.length, count: page.length, claims: page });
      return;
    }

    json(res, 404, { error: 'NOT_FOUND' });
  });
  server.listen(port);
  return server;
}
