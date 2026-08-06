# ObsCam

ObsCam provides a truthful, low-latency observatory camera view while preserving
the distinction between sensor acquisition and what a browser has presented.

## Language

**Trustworthy frame**:
A presented frame whose source identity and capture facts are exactly correlated
within the current runtime, stream, and browser connection.
_Avoid_: Best-effort frame, assumed-current frame

**Live feed**:
A browser presentation whose decoded media is advancing within the expected
delivery contract. Exact source identity and capture facts are independent
diagnostic evidence and must remain explicitly unknown until proven. The sensor
may simultaneously be exposing the next frame.
_Avoid_: Capturing, real-time feed

**Waiting for first image**:
The initial condition in which an authoritative exposure is progressing normally
but no decoded browser presentation exists yet.
_Avoid_: Capturing, unavailable

**Stale feed**:
A retained trustworthy frame for which a newer presentation is overdue or current
freshness can no longer be proven.
_Avoid_: Frozen feed, disconnected feed

**Exposure activity**:
The independently reported acquisition of the next sensor frame, including its
authoritative start and requested duration. It is not a feed state.
_Avoid_: Capturing state

**Requested settings**:
The newest complete exposure, gain, and treatment tuple intentionally submitted by
the controller but not yet proven visible.
_Avoid_: Current settings, selected settings

**Draft settings**:
A browser-local exposure, gain, and treatment tuple edited by the controller but not
submitted. Apply promotes the complete draft to Requested settings in one command;
hiding the interface does not.
_Avoid_: Pending settings, requested settings, staged server settings

**Visible settings**:
The complete settings tuple exactly correlated with the trustworthy frame currently
presented by the browser.
_Avoid_: Applied settings, selected settings

**ISO**:
The operator-facing name for the camera sensitivity detent stored internally as
vendor gain. It is a familiar control scale, not a calibrated photographic ISO.
_Avoid_: Gain, sensor gain

**Viewer**:
A browser session that can observe the feed, inspect status, and save a browser-local
snapshot without holding camera-control authority.
_Avoid_: Read-only controller

**Controller**:
The viewer holding the current renewable control lease and therefore permitted to
submit camera-setting mutations.
_Avoid_: Owner, admin
