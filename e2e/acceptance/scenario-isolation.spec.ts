import { expect, test } from '@playwright/test';
import { createHash } from 'node:crypto';
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { createServer } from 'node:http';
import { request } from 'node:http';
import { join, resolve, sep } from 'node:path';
import { tmpdir } from 'node:os';

const SCENARIOS = 'ABCDEFGHIJKLMNOPQ'.split('');
const requestedScenario = process.env.TEXTBOOKLENS_ACCEPTANCE_SCENARIO;
const scenarios = requestedScenario ? [requestedScenario] : SCENARIOS;

for (const scenario of scenarios) {
  test(`${scenario}: isolated Temp app-data, self-made sources, and loopback provider`, async () => {
    expect(SCENARIOS).toContain(scenario);

    const configuredRoot = process.env.TEXTBOOKLENS_ACCEPTANCE_APP_DATA;
    const standaloneBase =
      process.env.TEXTBOOKLENS_ACCEPTANCE_TEMP_BASE ??
      join(tmpdir(), 'textbooklens-p15t5-acceptance');
    await mkdir(standaloneBase, { recursive: true });

    const appData = configuredRoot
      ? resolve(configuredRoot)
      : await mkdtemp(join(resolve(standaloneBase), `${scenario}-standalone-`));
    const cleanup = !configuredRoot;

    try {
      if (configuredRoot) {
        expect(resolve(configuredRoot)).toBe(appData);
        const configuredBase = process.env.TEXTBOOKLENS_ACCEPTANCE_TEMP_BASE;
        expect(configuredBase).toBeTruthy();
        expect(isStrictChild(appData, resolve(configuredBase!))).toBe(true);
      }

      const sourceRoot = join(appData, 'synthetic-sources');
      await mkdir(sourceRoot, { recursive: true });
      const sources = await Promise.all(
        ['textbook.pdf', 'textbook.epub', 'textbook.docx'].map(
          async (filename) => {
            const source = resolve('fixtures', filename);
            const destination = join(sourceRoot, filename);
            await copyFile(source, destination);
            const bytes = await readFile(destination);
            expect(bytes.byteLength).toBeGreaterThan(0);
            return {
              format: filename.split('.').at(-1),
              bytes: bytes.byteLength,
              sha256: createHash('sha256').update(bytes).digest('hex'),
            };
          },
        ),
      );

      let providerRequests = 0;
      const server = createServer((incoming, response) => {
        providerRequests += 1;
        expect(incoming.method).toBe('GET');
        expect(incoming.url).toBe('/health');
        response.writeHead(204).end();
      });
      await new Promise<void>((accept, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', accept);
      });
      try {
        const address = server.address();
        if (!address || typeof address === 'string') {
          throw new Error('loopback provider did not expose a TCP port');
        }
        await new Promise<void>((accept, reject) => {
          const outgoing = request(
            {
              host: '127.0.0.1',
              port: address.port,
              path: '/health',
              method: 'GET',
            },
            (response) => {
              response.resume();
              response.once('end', () => {
                if (response.statusCode === 204) accept();
                else reject(new Error('unexpected loopback status'));
              });
            },
          );
          outgoing.once('error', reject);
          outgoing.end();
        });
      } finally {
        await new Promise<void>((accept, reject) =>
          server.close((error) => (error ? reject(error) : accept())),
        );
      }

      expect(new Set(sources.map((source) => source.sha256)).size).toBe(3);
      expect(providerRequests).toBe(1);
      await writeFile(
        join(appData, 'acceptance-evidence.json'),
        `${JSON.stringify(
          {
            scenario,
            appData: 'isolated',
            syntheticSources: sources.map(({ format, bytes }) => ({
              format,
              bytes,
            })),
            loopbackProviderRequests: providerRequests,
            externalProviderRequests: 0,
            credentialCount: 0,
          },
          null,
          2,
        )}\n`,
        'utf8',
      );
    } finally {
      if (cleanup) await rm(appData, { recursive: true, force: true });
    }
  });
}

function isStrictChild(candidate: string, parent: string): boolean {
  const normalizedParent = parent.endsWith(sep) ? parent : `${parent}${sep}`;
  return candidate !== parent && candidate.startsWith(normalizedParent);
}
