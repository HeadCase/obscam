export function snapshotFilename(facts, savedAt = new Date()) {
    if (facts === null) {
        return `obscam-saved-${compactTimestamp(savedAt)}-correlation-unknown.png`;
    }
    const capturedAt = new Date(facts.exposureCompletedAtUnixUs / 1_000);
    return `obscam-captured-${compactTimestamp(capturedAt)}-generation-${facts.sourceGeneration}.png`;
}
export function isSnapshotCancellation(error) {
    return error instanceof DOMException && error.name === "AbortError";
}
function compactTimestamp(value) {
    return value.toISOString().replaceAll("-", "").replaceAll(":", "");
}
