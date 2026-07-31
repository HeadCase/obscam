const SCHEMA_VERSION = 1;
const CLIENT_STORAGE_KEY = "obscam.service-quality.client.v1";
const EVIDENCE_REFRESH_MS = 500;
const REPORT_INTERVAL_MS = 250;
const MAX_REPORT_BATCH = 32;

export interface QualityObservation {
  correlation: "exact" | "unknown";
  streamEpoch: number | null;
  rtpTimestamp?: number;
  presentedFrames: number;
  visibility: "visible" | "hidden";
}

interface TimedQualityObservation extends QualityObservation {
  presentedAtUnixUs: number;
}

export interface Distribution {
  min: number;
  p50: number;
  p95: number;
  p99: number;
  max: number;
}

export interface QualityPartition {
  runtimeEpoch: string;
  streamEpoch: number | null;
  connectionGeneration: number;
  settingsGeneration: number | null;
  treatment: "monochrome" | "colour" | null;
  width: number | null;
  height: number | null;
  visibility: "visible" | "hidden";
  sampleCount: number;
  exactCorrelation: number;
  unknownCorrelation: number;
  uniquePresentedFrames: number;
  uniquePresentedCadenceHz: number | null;
  latencyUs: Distribution | null;
  clockUncertaintyUs: Distribution | null;
}

export interface QualityAggregate {
  sampleCount: number;
  exactCorrelation: number;
  unknownCorrelation: number;
  reconnects: number;
  presentationSkips: number;
  partitions: QualityPartition[];
}

export interface QualityClientReport extends QualityAggregate {
  clientId: string;
  connectionGeneration: number;
  samples: unknown[];
}

export interface ServiceQualityResponse {
  schemaVersion: typeof SCHEMA_VERSION;
  limits: { clients: 16; samplesPerClient: 512 };
  clients: QualityClientReport[];
  combined: QualityAggregate;
}

interface ClockCalibration {
  offsetUs: number;
  uncertaintyUs: number;
}

export class ServiceQualityClient {
  readonly clientId: string;
  private connectionGeneration = 0;
  private pending: TimedQualityObservation[] = [];
  private reporting = false;
  private reportTimer: number | null = null;
  private lastEvidenceAtMs = 0;

  private constructor(
    private readonly runtimeEpoch: string,
    private readonly clock: ClockCalibration,
    private readonly evidence: (response: ServiceQualityResponse) => void
  ) {
    this.clientId = clientId();
  }

  static async connect(
    runtimeEpoch: string,
    evidence: (response: ServiceQualityResponse) => void
  ): Promise<ServiceQualityClient> {
    const clock = await calibrateClock();
    const client = new ServiceQualityClient(runtimeEpoch, clock, evidence);
    await client.beginConnection();
    return client;
  }

  report(observation: QualityObservation): void {
    if (this.pending.length === MAX_REPORT_BATCH) {
      this.pending.shift();
    }
    this.pending.push({
      ...observation,
      presentedAtUnixUs: Date.now() * 1_000 + this.clock.offsetUs
    });
    this.scheduleReport();
  }

  /** Begins a new server-fenced media connection generation for this tab. */
  async reconnect(): Promise<void> {
    await this.beginConnection();
  }

  /** Returns Unix microseconds calibrated to the Rust service clock. */
  nowUnixUs(): number {
    return Date.now() * 1_000 + this.clock.offsetUs;
  }

  private async beginConnection(): Promise<void> {
    const response = await fetch("/api/v1/service-quality/connections", {
      method: "POST",
      cache: "no-store",
      headers: { Accept: "application/json", "Content-Type": "application/json" },
      body: JSON.stringify({
        schemaVersion: SCHEMA_VERSION,
        clientId: this.clientId,
        runtimeEpoch: this.runtimeEpoch
      })
    });
    if (!response.ok) {
      throw new Error(`quality connection request failed with ${response.status}`);
    }
    const value: unknown = await response.json();
    if (
      !isRecord(value) ||
      value.schemaVersion !== SCHEMA_VERSION ||
      value.clientId !== this.clientId ||
      !positiveInteger(value.connectionGeneration)
    ) {
      throw new Error("invalid quality connection response");
    }
    this.connectionGeneration = value.connectionGeneration;
  }

  private scheduleReport(): void {
    if (this.reporting || this.reportTimer !== null || this.pending.length === 0) {
      return;
    }
    this.reportTimer = window.setTimeout(() => {
      this.reportTimer = null;
      void this.flush();
    }, REPORT_INTERVAL_MS);
  }

  private async flush(): Promise<void> {
    if (this.reporting || this.pending.length === 0) {
      return;
    }
    this.reporting = true;
    const observations = this.pending.splice(0, MAX_REPORT_BATCH);
    try {
      await this.send(observations);
      const now = Date.now();
      if (now - this.lastEvidenceAtMs >= EVIDENCE_REFRESH_MS) {
        this.lastEvidenceAtMs = now;
        this.evidence(await this.fetchEvidence());
      }
    } catch (error: unknown) {
      console.error("ObsCam service-quality reporting failed", error);
    } finally {
      this.reporting = false;
      this.scheduleReport();
    }
  }

  private async send(observations: TimedQualityObservation[]): Promise<void> {
    const response = await fetch("/api/v1/service-quality", {
      method: "POST",
      cache: "no-store",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        schemaVersion: SCHEMA_VERSION,
        clientId: this.clientId,
        runtimeEpoch: this.runtimeEpoch,
        connectionGeneration: this.connectionGeneration,
        samples: observations.map((observation) => ({
          streamEpoch: observation.streamEpoch,
          ...(observation.rtpTimestamp === undefined
            ? {}
            : { rtpTimestamp: observation.rtpTimestamp }),
          presentedFrames: observation.presentedFrames,
          presentedAtUnixUs: observation.presentedAtUnixUs,
          clockUncertaintyUs: this.clock.uncertaintyUs,
          visibility: observation.visibility,
          correlation: observation.correlation
        }))
      })
    });
    if (!response.ok) {
      throw new Error(`quality report failed with ${response.status}`);
    }
  }

  private async fetchEvidence(): Promise<ServiceQualityResponse> {
    const response = await fetch(
      `/api/v1/service-quality?clientId=${encodeURIComponent(this.clientId)}`,
      { cache: "no-store", headers: { Accept: "application/json" } }
    );
    if (!response.ok) {
      throw new Error(`quality evidence request failed with ${response.status}`);
    }
    return parseServiceQualityResponse(await response.json(), this.clientId);
  }
}

export function serviceQualityText(response: ServiceQualityResponse): string {
  const client = response.clients[0];
  if (client === undefined || client.sampleCount === 0) {
    return "No quality samples";
  }
  const latestExact = [...client.partitions]
    .reverse()
    .find(
      (partition) =>
        partition.connectionGeneration === client.connectionGeneration &&
        partition.visibility === "visible" &&
        partition.exactCorrelation > 0
    );
  const measured = latestExact === undefined
    ? ""
    : ` · ${formatCadence(latestExact.uniquePresentedCadenceHz)} · p95 ${formatLatency(latestExact.latencyUs?.p95 ?? null)}`;
  return `${client.sampleCount} samples · ${client.exactCorrelation} exact · ${client.unknownCorrelation} unknown${measured}`;
}

export function downloadServiceQuality(
  response: ServiceQualityResponse,
  clientId: string
): void {
  const url = URL.createObjectURL(
    new Blob([`${JSON.stringify(response, null, 2)}\n`], { type: "application/json" })
  );
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `obscam-service-quality-${clientId}.json`;
  anchor.click();
  URL.revokeObjectURL(url);
}

export function parseServiceQualityResponse(
  value: unknown,
  expectedClientId: string
): ServiceQualityResponse {
  if (
    !isRecord(value) ||
    value.schemaVersion !== SCHEMA_VERSION ||
    !isRecord(value.limits) ||
    value.limits.clients !== 16 ||
    value.limits.samplesPerClient !== 512 ||
    !Array.isArray(value.clients) ||
    value.clients.length > 1 ||
    !validAggregate(value.combined)
  ) {
    throw new Error("invalid service-quality response");
  }
  for (const client of value.clients) {
    if (
      !isRecord(client) ||
      client.clientId !== expectedClientId ||
      !positiveInteger(client.connectionGeneration) ||
      !Array.isArray(client.samples) ||
      client.samples.length > 512 ||
      !validAggregate(client)
    ) {
      throw new Error("invalid service-quality client response");
    }
  }
  return value as unknown as ServiceQualityResponse;
}

async function calibrateClock(): Promise<ClockCalibration> {
  const startedAtUs = Date.now() * 1_000;
  const response = await fetch("/api/v1/clock", {
    cache: "no-store",
    headers: { Accept: "application/json" }
  });
  const completedAtUs = Date.now() * 1_000;
  if (!response.ok) {
    throw new Error(`clock calibration failed with ${response.status}`);
  }
  const value: unknown = await response.json();
  if (
    !isRecord(value) ||
    value.schemaVersion !== SCHEMA_VERSION ||
    !nonNegativeSafeInteger(value.serverUnixUs)
  ) {
    throw new Error("invalid clock calibration response");
  }
  const roundTripUs = completedAtUs - startedAtUs;
  return {
    offsetUs: Math.round(value.serverUnixUs - (startedAtUs + roundTripUs / 2)),
    uncertaintyUs: Math.ceil(roundTripUs / 2) + 1_000
  };
}

function clientId(): string {
  try {
    const stored = sessionStorage.getItem(CLIENT_STORAGE_KEY);
    if (stored !== null && isUuid(stored)) {
      return stored;
    }
    const created = randomUuid();
    sessionStorage.setItem(CLIENT_STORAGE_KEY, created);
    return created;
  } catch {
    return randomUuid();
  }
}

function randomUuid(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  bytes[6] = (bytes[6]! & 0x0f) | 0x40;
  bytes[8] = (bytes[8]! & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0"));
  return `${hex.slice(0, 4).join("")}-${hex.slice(4, 6).join("")}-${hex.slice(6, 8).join("")}-${hex.slice(8, 10).join("")}-${hex.slice(10).join("")}`;
}

function validAggregate(value: unknown): boolean {
  return (
    isRecord(value) &&
    nonNegativeSafeInteger(value.sampleCount) &&
    nonNegativeSafeInteger(value.exactCorrelation) &&
    nonNegativeSafeInteger(value.unknownCorrelation) &&
    value.exactCorrelation + value.unknownCorrelation === value.sampleCount &&
    nonNegativeSafeInteger(value.reconnects) &&
    nonNegativeSafeInteger(value.presentationSkips) &&
    Array.isArray(value.partitions) &&
    value.partitions.every(validPartition)
  );
}

function validPartition(value: unknown): boolean {
  return (
    isRecord(value) &&
    nonNegativeSafeInteger(value.sampleCount) &&
    nonNegativeSafeInteger(value.exactCorrelation) &&
    nonNegativeSafeInteger(value.unknownCorrelation) &&
    value.exactCorrelation + value.unknownCorrelation === value.sampleCount &&
    nonNegativeSafeInteger(value.uniquePresentedFrames) &&
    (value.uniquePresentedCadenceHz === null ||
      (typeof value.uniquePresentedCadenceHz === "number" &&
        Number.isFinite(value.uniquePresentedCadenceHz) &&
        value.uniquePresentedCadenceHz >= 0)) &&
    (value.latencyUs === null || validDistribution(value.latencyUs)) &&
    (value.clockUncertaintyUs === null || validDistribution(value.clockUncertaintyUs))
  );
}

function validDistribution(value: unknown): boolean {
  return (
    isRecord(value) &&
    [value.min, value.p50, value.p95, value.p99, value.max].every(nonNegativeSafeInteger)
  );
}

function formatCadence(value: number | null): string {
  return value === null ? "cadence unknown" : `${value.toFixed(1)} fps`;
}

function formatLatency(valueUs: number | null): string {
  return valueUs === null ? "unknown" : `${Math.round(valueUs / 1_000)} ms`;
}

function positiveInteger(value: unknown): value is number {
  return nonNegativeSafeInteger(value) && value > 0;
}

function nonNegativeSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(value);
}

function isRecord(value: unknown): value is Record<string, any> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
