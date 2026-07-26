function obsCam() {
  return {
    // Connection state
    connected: false,
    streaming: false,
    connectionStatus: "Connecting...",
    streamUrl: "/stream.mjpg",

    // Controls state
    controlsReady: false,
    controls: {
      exposure: { value: 200, index: 0, scale: [] },
      gain: { value: 250, index: 0, scale: [] },
    },

    // Telemetry
    telemetry: {
      fps: null,
      lastUpdate: "--",
      frameAge: "--",
      cameraStatus: "Init",
    },

    // Internal state
    eventSource: null,
    prototypeTimer: null,
    prototypeToastTimer: null,
    prototype: {
      variant: "A",
      controlsVisible: true,
      controlsHeld: false,
      activeControl: "exposure",
      colour: false,
      rotation: 0,
      hasControl: false,
      takeoverPending: false,
      menuOpen: false,
      paletteOpen: false,
      stale: false,
      toast: "",
    },

    init() {
      console.log("ObsCam initializing...");
      this.prototype.variant = this.readPrototypeVariant();
      window.addEventListener("keydown", (event) => this.handlePrototypeKey(event));
      this.schedulePrototypeHide();
      this.bootstrap();
    },

    readPrototypeVariant() {
      const candidate = new URLSearchParams(window.location.search)
        .get("variant")
        ?.toUpperCase();
      return ["A", "B", "C"].includes(candidate) ? candidate : "A";
    },

    variantLabel() {
      return {
        A: "A — Bottom tray",
        B: "B — Edge controls",
        C: "C — Command palette",
      }[this.prototype.variant];
    },

    cycleVariant(direction) {
      const variants = ["A", "B", "C"];
      const index = variants.indexOf(this.prototype.variant);
      this.prototype.variant = variants[(index + direction + variants.length) % variants.length];
      this.prototype.menuOpen = false;
      this.prototype.paletteOpen = false;
      const url = new URL(window.location.href);
      url.searchParams.set("variant", this.prototype.variant);
      window.history.replaceState({}, "", url);
      this.showPrototypeControls();
    },

    handlePrototypeKey(event) {
      if (!["ArrowLeft", "ArrowRight"].includes(event.key)) return;
      const target = event.target;
      if (target.matches("input, textarea, [contenteditable]")) return;
      event.preventDefault();
      this.cycleVariant(event.key === "ArrowRight" ? 1 : -1);
    },

    showPrototypeControls() {
      this.prototype.controlsVisible = true;
      this.schedulePrototypeHide();
    },

    holdPrototypeControls() {
      this.prototype.controlsHeld = true;
      clearTimeout(this.prototypeTimer);
    },

    releasePrototypeControls() {
      this.prototype.controlsHeld = false;
      this.schedulePrototypeHide();
    },

    schedulePrototypeHide() {
      clearTimeout(this.prototypeTimer);
      this.prototypeTimer = setTimeout(() => {
        if (!this.prototype.controlsHeld && !this.prototype.takeoverPending && !this.prototype.paletteOpen) {
          this.prototype.controlsVisible = false;
          this.prototype.menuOpen = false;
        }
      }, 4200);
    },

    stepActiveControl(direction) {
      const control = this.controls[this.prototype.activeControl];
      control.index = Math.max(0, Math.min(control.scale.length - 1, Number(control.index) + direction));
      this.updateControlValues();
      this.showPrototypeControls();
    },

    toggleColour() {
      this.prototype.colour = !this.prototype.colour;
      this.showToast(this.prototype.colour ? "Colour presentation" : "B&W performance mode");
    },

    rotateViewer() {
      this.prototype.rotation = (this.prototype.rotation + 90) % 360;
      this.showToast(`Rotated ${this.prototype.rotation}° · complete frame preserved`);
    },

    requestTakeover() {
      if (this.prototype.hasControl) {
        this.showToast("You already have control");
        return;
      }
      this.prototype.takeoverPending = true;
      this.holdPrototypeControls();
    },

    confirmTakeover() {
      this.prototype.takeoverPending = false;
      this.prototype.hasControl = true;
      this.releasePrototypeControls();
      this.showToast("Control transferred to this viewer");
    },

    downloadSnapshot() {
      const link = document.createElement("a");
      link.href = this.streamUrl;
      link.download = `obscam-snapshot-${Date.now()}.jpg`;
      link.click();
      this.showToast("Snapshot download requested");
    },

    showToast(message) {
      clearTimeout(this.prototypeToastTimer);
      this.prototype.toast = message;
      this.prototypeToastTimer = setTimeout(() => { this.prototype.toast = ""; }, 2200);
      this.showPrototypeControls();
    },

    async bootstrap() {
      try {
        const response = await fetch("/api/bootstrap", {
          cache: "no-store",
        });
        const data = await response.json();

        if (response.ok) {
          this.buildControlScales(data.capabilities);
          this.syncSettingsFromServer(data.current_settings);
          this.controlsReady = true;

          const backendState = data.status?.backend?.state || "stopped";
          this.connected = backendState === "running";
          this.connectionStatus = backendState;

          // Start SSE telemetry
          this.startTelemetry();

          // Auto-start streaming only when the backend is already running
          if (backendState === "running") {
            setTimeout(() => this.startStream(), 500);
          }
        } else {
          throw new Error(data.detail || "Bootstrap failed");
        }
      } catch (error) {
        console.error("Bootstrap failed:", error);
        this.connectionStatus = "Bootstrap Error";
        this.connected = false;
      }
    },

    buildControlScales(capabilities) {
      console.log("Building control scales from capabilities:", capabilities);

      // Build exposure scale from capabilities
      const expCap = capabilities.exposure_ms;
      this.controls.exposure.scale = this.buildExposureScale(
        expCap.min,
        expCap.max,
      );

      // Build gain scale from capabilities
      const gainCap = capabilities.gain;
      this.controls.gain.scale = this.buildGainScale(gainCap.min, gainCap.max);
    },

    buildExposureScale(min, max) {
      // Smart exposure scale - more points in observatory-relevant range
      const scale = [];
      const targets = [
        0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 100, 150, 200, 300, 500, 800, 1000,
        2000, 5000, 10000, 30000,
      ];

      for (let target of targets) {
        if (target >= min && target <= max) {
          scale.push(target);
        }
      }

      // Ensure we have min and max
      if (scale[0] !== min) scale.unshift(min);
      if (scale[scale.length - 1] !== max) scale.push(max);

      return scale;
    },

    buildGainScale(min, max) {
      // Camera-specific gain scale
      const scale = [];

      if (max <= 600) {
        // ASI662MC scale (0-600)
        const targets = [
          0, 25, 50, 100, 150, 200, 250, 300, 350, 400, 450, 500, 550, 600,
        ];
        for (let target of targets) {
          if (target >= min && target <= max) {
            scale.push(target);
          }
        }
      } else {
        // Generic log-ish scale for higher gain ranges
        if (min <= 0) {
          scale.push(0);
        }

        const safeMin = Math.max(1, min);
        const steps = 12;
        for (let i = 0; i <= steps; i++) {
          const ratio = i / steps;
          const value = Math.round(safeMin * Math.pow(max / safeMin, ratio));
          if (value >= min && value <= max && scale.at(-1) !== value) {
            scale.push(value);
          }
        }
      }

      return scale;
    },

    syncSettingsFromServer(settings) {
      console.log("Syncing settings from server:", settings);

      // Map server values to slider indices
      this.controls.exposure.index = this.findClosestIndex(
        settings.exposure_ms,
        this.controls.exposure.scale,
      );
      this.controls.gain.index = this.findClosestIndex(
        settings.gain,
        this.controls.gain.scale,
      );

      this.updateControlValues();
    },

    findClosestIndex(target, scale) {
      let closestIndex = 0;
      let minDiff = Math.abs(scale[0] - target);

      for (let i = 1; i < scale.length; i++) {
        const diff = Math.abs(scale[i] - target);
        if (diff < minDiff) {
          minDiff = diff;
          closestIndex = i;
        }
      }

      return closestIndex;
    },

    updateControlValues() {
      // Update displayed values from indices
      this.controls.exposure.value =
        this.controls.exposure.scale[this.controls.exposure.index];
      this.controls.gain.value =
        this.controls.gain.scale[this.controls.gain.index];
    },

    formatExposure(ms) {
      if (ms >= 1000) {
        return (ms / 1000).toFixed(ms % 1000 === 0 ? 0 : 1) + "s";
      }
      return ms + "ms";
    },

    async updateCameraSettings() {
      // GRE-184 is intentionally browser-local; no prototype control mutates the server.
      if (this.prototype) {
        this.showToast("Prototype setting updated locally");
        return;
      }
      try {
        const payload = {
          exposure_ms: this.controls.exposure.value,
          gain: this.controls.gain.value,
        };

        const response = await fetch("/api/settings", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        });

        if (response.ok) {
          const data = await response.json();
          console.log("Settings updated successfully:", data.current_settings);
        } else {
          console.error("Settings update failed:", response.status);
        }
      } catch (error) {
        console.error("Settings update error:", error);
      }
    },

    startTelemetry() {
      if (this.eventSource) {
        this.eventSource.close();
      }

      this.eventSource = new EventSource("/api/telemetry");

      this.eventSource.addEventListener("snapshot", (event) => {
        const data = JSON.parse(event.data);
        this.updateTelemetry(data);
      });

      this.eventSource.addEventListener("frame", (event) => {
        const data = JSON.parse(event.data);
        this.updateTelemetry(data);
        this.telemetry.lastUpdate = new Date().toLocaleTimeString();
      });

      this.eventSource.addEventListener("heartbeat", (event) => {
        console.log("Telemetry heartbeat");
      });

      this.eventSource.onerror = (error) => {
        console.error("SSE error:", error);
        this.connectionStatus = "Telemetry Error";
      };
    },

    updateTelemetry(data) {
      this.telemetry.fps = data.fps;
      this.telemetry.cameraStatus = data.backend_state || (data.has_frame ? "Active" : "Idle");
      this.connectionStatus = data.backend_state || this.connectionStatus;

      // Calculate frame age
      if (data.timestamp) {
        const age = Date.now() / 1000 - data.timestamp;
        if (age < 1) {
          this.telemetry.frameAge = Math.round(age * 1000) + "ms";
        } else if (age < 60) {
          this.telemetry.frameAge = age.toFixed(1) + "s";
        } else {
          const mins = Math.floor(age / 60);
          const secs = Math.floor(age % 60);
          this.telemetry.frameAge = `${mins}m${secs}s`;
        }
      }
    },

    startStream() {
      console.log("Starting MJPEG stream...");
      this.streaming = true;
      this.connectionStatus = "Streaming";
    },

    toggleStream() {
      if (this.streaming) {
        this.streaming = false;
        this.connectionStatus = "Paused";
      } else {
        this.startStream();
      }
    },

    handleStreamError() {
      console.error("MJPEG stream error");
      this.connectionStatus = "Stream Error";
      // Retry after delay
      setTimeout(() => {
        if (this.streaming) {
          this.$refs.mjpegStream.src = this.streamUrl + "?t=" + Date.now();
        }
      }, 2000);
    },

    handleStreamLoad() {
      console.log("MJPEG stream loaded successfully");
      if (this.streaming) {
        this.connectionStatus = "running";
        this.connected = true;
      }
    },
  };
}
