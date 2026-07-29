import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const html = fs.readFileSync(
  new URL('../crates/gre200-probe/src/client.html', import.meta.url),
  'utf8',
);
const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
assert(script, 'inline client script is present');
assert.match(
  html,
  /#empty\[hidden\]\{display:none!important\}/,
  'the connected-state overlay must be forcibly hidden',
);
assert.doesNotMatch(
  script,
  /result\.status\s*!==\s*localStatus/,
  'the local telemetry cache must not veto authoritative correlation',
);
assert.match(
  script,
  /capture\.capture_started_unix_ns.*capture\.exposure_us\/1000\+500\+identityGraceMs/,
  'presentation liveness must use the authoritative exposure-aware deadline',
);
assert.match(
  script,
  /else if\(now-model\.lastCorrelatedAt>identityGraceMs\)/,
  'an isolated unknown identity must retain the last proven frame during the grace period',
);
assert.match(
  script,
  /now-model\.lastPresentationAt<identityGraceMs/,
  'an isolated presentation pause must not trigger media recovery',
);
assert.match(
  script,
  /peer\.getStats\(\)/,
  'media recovery must inspect WebRTC transport progress',
);
assert.match(
  script,
  /bytesReceived/,
  'media recovery must use inbound RTP byte progress',
);
assert.doesNotMatch(
  script,
  /peer\.connectionState==='disconnected'.*connect\(\)/,
  'a transient WebRTC disconnected state must not replace progressing media',
);
assert.match(
  script,
  /model\.awaitingFirstFrame/,
  'startup state must remain reconnecting until the first exact frame',
);
const presentationTimer = script.match(
  /setInterval\(\(\)=>\{.*lastPresentationAt.*?\},250\);/,
)?.[0];
assert.ok(presentationTimer, 'presentation-liveness timer must exist');
assert.doesNotMatch(
  presentationTimer,
  /connect\(\)/,
  'presentation callback timing must not tear down healthy transport',
);
assert.match(
  script,
  /setState\('Capturing'/,
  'a progressing long exposure must not masquerade as media failure',
);
assert.match(
  script,
  /if\(model\.connecting\)return/,
  'media recovery attempts must not overlap',
);
assert.match(
  script,
  /const peer=model\.peer;model\.peer=null;if\(peer\)peer\.close\(\)/,
  'intentional peer closure must detach callbacks before closing',
);
assert.match(
  script,
  /now-model\.mediaAttemptStartedAt>4000/,
  'watchdog must allow a bounded keyframe establishment grace',
);
assert.match(
  script,
  /setTimeout\(resolve,750\)/,
  'bounded host-candidate gathering must leave recovery headroom',
);
assert.match(
  script,
  /model\.runtime\.stream_epoch=frame\.stream_epoch/,
  'a newer telemetry stream epoch must advance browser correlation context',
);
assert.doesNotMatch(
  script,
  /Math\.min\(10000,/,
  'media reconnect backoff must remain within the five-second recovery gate',
);
assert.match(
  script,
  /Correlation service unavailable/,
  'presentation-report failures must not masquerade as video reconnects',
);
assert.match(
  script,
  /Frame identity unavailable/,
  'authoritative unknown results must describe correlation, not connectivity',
);
assert.doesNotMatch(
  script,
  /whep_url|192\.168\.1\.200|10\.164\.190\.1/,
  'the browser contract must not contain a fixed-interface media URL',
);
assert.match(
  script,
  /endpoint\(runtime\.whep\)/,
  'the browser must consume the hostless WHEP descriptor',
);
assert.match(
  script,
  /fetch\(`\/api\/evidence\/clients\/\$\{clientId\}`,\{cache:'no-store'\}\)/,
  'browser-visible service quality must come from the agent-readable evidence contract',
);
assert.match(
  script,
  /fetch\('\/api\/connections',\{method:'POST'/,
  'each browser media connection must advance authoritative reconnect evidence',
);
for (const field of [
  'sample_count',
  'exact_correlation_count',
  'unknown_correlation_count',
  'unique_presented_cadence_hz',
  'latency_ms.p50',
  'latency_ms.p95',
  'latency_ms.p99',
  'clock_uncertainty_ms.p95',
  'reconnect_count',
]) {
  assert.ok(
    script.includes(field),
    `browser-visible evidence must display authoritative ${field}`,
  );
}
assert.match(
  script,
  /JSON\.stringify\(model\.evidence,null,2\)/,
  'the downloaded evidence must be the same authoritative object the browser displays',
);

const endpointStatement = script
  .split('\n')
  .find((line) => line.includes('function endpoint'));
assert(endpointStatement, 'origin-aware WHEP endpoint function is present');
for (const host of ['192.168.1.200', '10.164.190.1']) {
  const endpointContext = {
    URL,
    location: { origin: `http://${host}:8200` },
    fetch: () => Promise.resolve(),
    model: { runtime: { runtime_epoch: 'epoch', stream_epoch: 1 } },
    clientId: 'client',
  };
  vm.runInNewContext(
    `${endpointStatement};result=endpoint({port:18889,path:'/obscam/whep'})`,
    endpointContext,
  );
  assert.equal(endpointContext.result, `http://${host}:18889/obscam/whep`);
}

const clientIdStatements = script
  .split('\n')
  .filter((line) => line.includes('createClientId') || line.includes('const clientId'))
  .join('\n');
const context = {
  crypto: {},
  Date,
  Math,
  sessionStorage: {},
};

vm.runInNewContext(clientIdStatements, context);
assert.match(context.sessionStorage.gre200ClientId, /^[a-z0-9]+$/);
