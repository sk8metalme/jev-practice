import { fileURLToPath } from 'node:url';

import { createApp } from './app.js';
import { createGatewayEvaluator } from './gateway.js';

export function resolvePort(value = process.env.PORT) {
  return Number(value) || 3000;
}

export function startServer(port = resolvePort(), log = console.log) {
  const server = createApp({ evaluator: createGatewayEvaluator() });
  server.listen(port, () => {
    const actualPort = listeningPort(server.address(), port);
    log(`Jev Triage is running at http://localhost:${actualPort}`);
  });
  return server;
}

export function listeningPort(address, fallbackPort) {
  return typeof address === 'object' && address !== null ? address.port : fallbackPort;
}

export function runIfMain({ argv = process.argv, moduleUrl = import.meta.url, start = startServer } = {}) {
  if (argv[1] && fileURLToPath(moduleUrl) === argv[1]) {
    start();
  }
}

runIfMain();
