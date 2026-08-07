export interface ExactSnapshotFacts {
  exposureCompletedAtUnixUs: number;
  sourceGeneration: number;
}

export function snapshotFilename(
  facts: ExactSnapshotFacts | null,
  savedAt: Date = new Date()
): string {
  if (facts === null) {
    return `obscam-saved-${compactTimestamp(savedAt)}-correlation-unknown.png`;
  }
  const capturedAt = new Date(facts.exposureCompletedAtUnixUs / 1_000);
  return `obscam-captured-${compactTimestamp(capturedAt)}-generation-${facts.sourceGeneration}.png`;
}

export function isSnapshotCancellation(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

function compactTimestamp(value: Date): string {
  return value.toISOString().replaceAll("-", "").replaceAll(":", "");
}
