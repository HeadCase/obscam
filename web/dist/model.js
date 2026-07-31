/** Derives a direct WHEP endpoint without accepting a media-supplied authority. */
export function deriveWhepUrl(media, pageUrl) {
    const endpoint = new URL(pageUrl);
    endpoint.port = String(media.whepPort);
    endpoint.pathname = media.whepPath;
    endpoint.search = "";
    endpoint.hash = "";
    return endpoint.toString();
}
/** Validates the untrusted runtime payload and rejects unsupported frame claims. */
export function parseRuntimeContract(value) {
    if (!isRecord(value) || value.schemaVersion !== 1) {
        throw new Error("unsupported runtime contract");
    }
    if (typeof value.runtimeEpoch !== "string" ||
        !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(value.runtimeEpoch)) {
        throw new Error("invalid runtime epoch");
    }
    if (!isRecord(value.media) ||
        !Number.isInteger(value.media.whepPort) ||
        typeof value.media.whepPort !== "number" ||
        value.media.whepPort < 1 ||
        value.media.whepPort > 65_535 ||
        typeof value.media.whepPath !== "string" ||
        !isHostlessWhepPath(value.media.whepPath)) {
        throw new Error("invalid media descriptor");
    }
    const components = parseRuntimeComponents(value.components);
    if (value.latestFrame !== null) {
        throw new Error("untrusted frame contract");
    }
    return {
        schemaVersion: 1,
        runtimeEpoch: value.runtimeEpoch,
        media: {
            whepPort: value.media.whepPort,
            whepPath: value.media.whepPath
        },
        components,
        latestFrame: null
    };
}
/** Validates and normalizes independently reported component facts. */
export function parseRuntimeComponents(value) {
    if (!isRecord(value) ||
        !isComponentStatus(value.capture, "no_camera_source") ||
        !isComponentStatus(value.encoder, "no_frame") ||
        !isComponentStatus(value.relay, "not_observed")) {
        throw new Error("invalid component state");
    }
    return {
        capture: parseComponentStatus(value.capture, "no_camera_source"),
        encoder: parseComponentStatus(value.encoder, "no_frame"),
        relay: parseComponentStatus(value.relay, "not_observed")
    };
}
/** Derives only claims justified by the validated bootstrap contract. */
export function deriveViewerState(_runtime) {
    return { status: "Unavailable" };
}
function isRecord(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
function isHostlessWhepPath(value) {
    return (value.startsWith("/") &&
        !value.startsWith("//") &&
        !/[\s?#]/u.test(value) &&
        new TextEncoder().encode(value).byteLength <= 256);
}
function isComponentStatus(value, reason) {
    return (isRecord(value) &&
        (value.state === "ready" || (value.state === "unavailable" && value.reason === reason)));
}
function parseComponentStatus(value, reason) {
    if (!isRecord(value) || value.state === "ready") {
        return { state: "ready" };
    }
    return { state: "unavailable", reason };
}
