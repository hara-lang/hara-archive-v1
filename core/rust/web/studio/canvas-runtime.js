const DEFAULT_CAPABILITIES = new Set([
  "canvas/2d",
  "canvas/webgl2",
  "input/keyboard",
  "input/pointer"
]);

export class CanvasRuntime {
  constructor({
    window: windowObject = globalThis.window,
    requestFrame = globalThis.requestAnimationFrame?.bind(globalThis),
    cancelFrame = globalThis.cancelAnimationFrame?.bind(globalThis),
    capabilities = DEFAULT_CAPABILITIES,
    onDiagnostic = () => {}
  } = {}) {
    this.window = windowObject;
    this.requestFrame = requestFrame;
    this.cancelFrame = cancelFrame;
    this.capabilities = new Set(capabilities);
    this.onDiagnostic = onDiagnostic;
    this.canvases = new Map();
    // A workspace can give one physical surface several semantic names (for
    // example canvas/background and canvas/visualizer). Ownership must still
    // be exclusive at the browser surface, otherwise two live generations
    // could silently draw over one another.
    this.surfaces = new Map();
    this.events = [];
    this.listeners = [];
    this.visible = true;
    this.sequence = 0;
    this.installInput();
  }

  register(canvasId, canvas) {
    if (this.canvases.has(canvasId)) throw new Error(`CANVAS_EXISTS ${canvasId}`);
    const slot = {
      id: canvasId,
      canvas,
      owner: null,
      candidate: null,
      pending: new Map(),
      firstRender: new Map(),
      lastTime: null,
      webgl: null,
      stateful: null,
      live: null,
      liveToken: null,
      lastFrame: null
    };
    this.canvases.set(canvasId, slot);
    let slots = this.surfaces.get(canvas);
    if (!slots) this.surfaces.set(canvas, slots = new Set());
    slots.add(slot);
    return () => this.unregister(canvasId);
  }

  unregister(canvasId) {
    const slot = this.canvases.get(canvasId);
    if (!slot) return false;
    this.cancelSlot(slot, "canvas closed");
    this.disposeWebGl(slot);
    this.canvases.delete(canvasId);
    const slots = this.surfaces.get(slot.canvas);
    slots?.delete(slot);
    if (slots?.size === 0) this.surfaces.delete(slot.canvas);
    return true;
  }

  claim(nodeId, canvasId) {
    const slot = this.requireCanvas(canvasId);
    this.cancelSurfaceOwners(slot, nodeId, "canvas surface ownership replaced");
    if (slot.owner && slot.owner !== nodeId) this.cancelOwner(slot, slot.owner, "canvas ownership replaced");
    slot.owner = nodeId;
    slot.stateful = null;
    slot.lastTime = null;
    return true;
  }

  stage(nodeId, canvasId) {
    const slot = this.requireCanvas(canvasId);
    this.cancelSurfaceCandidates(slot, nodeId, "canvas surface candidate replaced");
    if (slot.candidate && slot.candidate !== nodeId) {
      this.cancelOwner(slot, slot.candidate, "candidate generation replaced");
    }
    slot.candidate = nodeId;
    return true;
  }

  commit(nodeId, canvasId) {
    const slot = this.requireCanvas(canvasId);
    if (slot.candidate !== nodeId) throw structuredError("canvas/not-candidate", `${nodeId} is not staged`);
    this.cancelSurfaceOwners(slot, nodeId, "canvas surface generation replaced");
    if (slot.owner && slot.owner !== nodeId) this.cancelOwner(slot, slot.owner, "canvas generation replaced");
    slot.owner = nodeId;
    slot.candidate = null;
    slot.stateful = null;
    slot.lastTime = null;
    return true;
  }

  discard(nodeId, canvasId) {
    const slot = this.requireCanvas(canvasId);
    if (slot.candidate !== nodeId) return false;
    this.cancelOwner(slot, nodeId, "candidate generation discarded");
    slot.candidate = null;
    return true;
  }

  release(nodeId, canvasId = null) {
    let released = false;
    for (const slot of this.canvases.values()) {
      if (canvasId !== null && slot.id !== canvasId) continue;
      if (slot.candidate === nodeId) {
        this.cancelOwner(slot, nodeId, "canvas candidate released");
        slot.candidate = null;
        released = true;
      }
      if (slot.owner !== nodeId) continue;
      this.cancelOwner(slot, nodeId, "canvas generation released");
      slot.owner = null;
      released = true;
    }
    return released;
  }

  setVisible(visible) {
    this.visible = Boolean(visible);
    if (this.visible) return;
    for (const slot of this.canvases.values()) this.cancelSlot(slot, "workspace hidden");
  }

  nextFrame(nodeId, canvasId) {
    const slot = this.assertAccess(nodeId, canvasId);
    if (!this.visible) return Promise.reject(structuredError("canvas/hidden", "workspace is hidden"));
    return new Promise((resolve, reject) => {
      const token = this.requestFrame((time) => {
        slot.pending.delete(token);
        if (!this.visible || (slot.owner !== nodeId && slot.candidate !== nodeId)) {
          reject(structuredError("canvas/generation-inactive", "canvas generation is no longer active"));
          return;
        }
        this.resize(slot.canvas);
        const now = Math.max(0, Math.trunc(time));
        const delta = slot.lastTime === null ? 0 : Math.max(0, now - slot.lastTime);
        slot.lastTime = now;
        const events = this.events.splice(0);
        resolve(toHta({
          "frame/id": ++this.sequence,
          "frame/time-ms": now,
          "frame/delta-ms": delta,
          "canvas/width": slot.canvas.clientWidth || slot.canvas.width,
          "canvas/height": slot.canvas.clientHeight || slot.canvas.height,
          "canvas/pixel-ratio-milli": Math.round(this.pixelRatio() * 1000),
          "input/events": events
        }));
      });
      slot.pending.set(token, reject);
    });
  }

  render(nodeId, canvasId, value) {
    const slot = this.assertAccess(nodeId, canvasId);
    const frame = plain(value);
    const backend = keyName(frame.type ?? frame["frame/type"] ?? frame["render/type"]);
    try {
      if (backend === "webgl2") this.renderWebGl(slot, frame);
      else if (backend === "canvas-2d") this.renderCanvas2d(slot, frame);
      else throw new Error(`unsupported frame type: ${backend ?? "nil"}`);
      slot.lastFrame = frame;
      this.resolveFirstRender(slot, nodeId);
      return true;
    } catch (error) {
      const fallback = frame.fallback ?? frame["frame/fallback"];
      if (fallback) {
        this.renderCanvas2d(slot, plain(fallback));
        this.onDiagnostic(structuredError("canvas/webgl-fallback", error.message));
        this.resolveFirstRender(slot, nodeId);
        return true;
      }
      const diagnostic = structuredError("canvas/render-failed", error.message);
      this.rejectFirstRender(slot, nodeId, diagnostic);
      this.onDiagnostic(diagnostic);
      throw diagnostic;
    }
  }

  publish(nodeId, canvasId, value) {
    const slot = this.assertAccess(nodeId, canvasId);
    slot.live = { nodeId, frame: plain(value) };
    this.renderLive(slot);
    this.scheduleLive(slot);
    return true;
  }

  waitForFirstRender(nodeId, canvasId, timeout = 2000) {
    const slot = this.assertAccess(nodeId, canvasId);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        slot.firstRender.delete(nodeId);
        reject(structuredError("canvas/first-frame-timeout", "generation did not render a frame"));
      }, timeout);
      slot.firstRender.set(nodeId, {
        resolve: () => { clearTimeout(timer); resolve(true); },
        reject: (error) => { clearTimeout(timer); reject(error); }
      });
    });
  }

  installInput() {
    if (!this.window?.addEventListener) return;
    const listen = (name, handler, options) => {
      this.window.addEventListener(name, handler, options);
      this.listeners.push(() => this.window.removeEventListener(name, handler, options));
    };
    if (this.capabilities.has("input/keyboard")) {
      listen("keydown", (event) => this.pushEvent({
        type: "key", phase: "down", key: event.key, code: event.code,
        repeat: event.repeat, modifiers: modifiers(event)
      }));
      listen("keyup", (event) => this.pushEvent({
        type: "key", phase: "up", key: event.key, code: event.code,
        repeat: false, modifiers: modifiers(event)
      }));
    }
    if (this.capabilities.has("input/pointer")) {
      let touchStart = null;
      listen("pointerdown", (event) => {
        if (event.pointerType === "touch") touchStart = point(event);
        this.pushEvent({ type: "pointer", phase: "down", ...point(event) });
      });
      listen("pointermove", (event) =>
        this.pushEvent({ type: "pointer", phase: "move", ...point(event) }));
      listen("pointerup", (event) => {
        const end = point(event);
        this.pushEvent({ type: "pointer", phase: "up", ...end });
        if (event.pointerType === "touch" && touchStart) {
          const dx = end.x - touchStart.x;
          const dy = end.y - touchStart.y;
          if (Math.max(Math.abs(dx), Math.abs(dy)) >= 24) {
            this.pushEvent({
              type: "swipe",
              direction: Math.abs(dx) > Math.abs(dy)
                ? (dx > 0 ? "right" : "left")
                : (dy > 0 ? "down" : "up")
            });
          }
          touchStart = null;
        }
      });
    }
  }

  pushEvent(event) {
    if (!this.visible || ![...this.canvases.values()].some((slot) => slot.owner)) return;
    this.events.push(toHta(event));
    if (this.events.length > 128) this.events.shift();
  }

  renderCanvas2d(slot, frame) {
    if (!this.capabilities.has("canvas/2d")) throw new Error("missing :canvas/2d capability");
    const canvas = slot.canvas;
    this.resize(canvas);
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Canvas2D is unavailable");
    const ratio = this.pixelRatio();
    const width = canvas.clientWidth || canvas.width / ratio;
    const height = canvas.clientHeight || canvas.height / ratio;
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.globalAlpha = 1;
    context.shadowBlur = 0;
    context.clearRect(0, 0, width, height);
    context.fillStyle = frame.background ?? "#020408";
    context.fillRect(0, 0, width, height);
    if (frame.stateful) {
      renderStateful2d(slot, context, frame.stateful, width, height);
      return;
    }
    for (const command of frame.commands ?? []) execute2d(context, plain(command), width, height);
  }

  renderWebGl(slot, frame) {
    if (!this.capabilities.has("canvas/webgl2")) throw new Error("missing :canvas/webgl2 capability");
    this.resize(slot.canvas);
    const width = slot.canvas.width;
    const height = slot.canvas.height;
    if (!slot.webgl) {
      const surface = this.window?.document?.createElement?.("canvas");
      if (!surface) throw new Error("WebGL surface cannot be created");
      slot.webgl = { surface, gl: surface.getContext("webgl2"), programs: new Map() };
    }
    const { surface, gl, programs } = slot.webgl;
    if (!gl) throw new Error("WebGL2 is unavailable");
    surface.width = width;
    surface.height = height;
    const vertex = frame.vertex ?? frame["shader/vertex"];
    const fragment = frame.fragment ?? frame["shader/fragment"];
    if (typeof vertex !== "string" || typeof fragment !== "string") {
      throw new Error("WebGL frame requires vertex and fragment shader sources");
    }
    const hash = `${hashText(vertex)}:${hashText(fragment)}`;
    let program = programs.get(hash);
    if (!program) {
      program = linkProgram(gl, vertex, fragment);
      programs.set(hash, program);
    }
    gl.viewport(0, 0, width, height);
    gl.useProgram(program);
    const uniforms = plain(frame.uniforms ?? {});
    for (const [name, value] of Object.entries(uniforms)) {
      const location = gl.getUniformLocation(program, name);
      if (location === null) continue;
      if (Array.isArray(value)) {
        const physicalValue = resolutionUniform(name, value, width, height);
        if (physicalValue.length === 2) gl.uniform2fv(location, physicalValue);
        else if (physicalValue.length === 3) gl.uniform3fv(location, physicalValue);
        else if (physicalValue.length === 4) gl.uniform4fv(location, physicalValue);
      } else {
        gl.uniform1f(location, Number(value));
      }
    }
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    const context = slot.canvas.getContext("2d");
    if (!context) throw new Error("Canvas2D compositor is unavailable");
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, width, height);
    context.drawImage(surface, 0, 0);
  }

  resize(canvas) {
    const ratio = this.pixelRatio();
    const width = Math.max(1, Math.round((canvas.clientWidth || canvas.width || 1) * ratio));
    const height = Math.max(1, Math.round((canvas.clientHeight || canvas.height || 1) * ratio));
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
  }

  pixelRatio() {
    return Math.min(2, Math.max(1, this.window?.devicePixelRatio || 1));
  }

  assertOwner(nodeId, canvasId) {
    const slot = this.requireCanvas(canvasId);
    if (slot.owner !== nodeId) {
      throw structuredError("canvas/not-owner", `${nodeId} does not own ${canvasId}`);
    }
    return slot;
  }

  assertAccess(nodeId, canvasId) {
    const slot = this.requireCanvas(canvasId);
    if (slot.owner !== nodeId && slot.candidate !== nodeId) {
      throw structuredError("canvas/not-owner", `${nodeId} does not own ${canvasId}`);
    }
    return slot;
  }

  requireCanvas(canvasId) {
    const slot = this.canvases.get(canvasId);
    if (!slot) throw structuredError("canvas/not-found", `unknown canvas ${canvasId}`);
    return slot;
  }

  resolveFirstRender(slot, nodeId) {
    slot.firstRender.get(nodeId)?.resolve();
    slot.firstRender.delete(nodeId);
  }

  rejectFirstRender(slot, nodeId, error) {
    slot.firstRender.get(nodeId)?.reject(error);
    slot.firstRender.delete(nodeId);
  }

  cancelOwner(slot, nodeId, reason) {
    for (const [token, reject] of slot.pending) {
      this.cancelFrame(token);
      reject(structuredError("canvas/cancelled", reason));
    }
    slot.pending.clear();
    if (slot.live?.nodeId === nodeId) {
      if (slot.liveToken !== null) this.cancelFrame(slot.liveToken);
      slot.live = null;
      slot.liveToken = null;
    }
    this.rejectFirstRender(slot, nodeId, structuredError("canvas/cancelled", reason));
  }

  cancelSurfaceOwners(slot, nodeId, reason) {
    for (const sibling of this.surfaces.get(slot.canvas) ?? []) {
      if (sibling !== slot && sibling.owner && sibling.owner !== nodeId) {
        this.cancelOwner(sibling, sibling.owner, reason);
        sibling.owner = null;
      }
    }
  }

  cancelSurfaceCandidates(slot, nodeId, reason) {
    for (const sibling of this.surfaces.get(slot.canvas) ?? []) {
      if (sibling !== slot && sibling.candidate && sibling.candidate !== nodeId) {
        this.cancelOwner(sibling, sibling.candidate, reason);
        sibling.candidate = null;
      }
    }
  }

  cancelSlot(slot, reason) {
    if (slot.owner) this.cancelOwner(slot, slot.owner, reason);
    if (slot.candidate && slot.candidate !== slot.owner) this.cancelOwner(slot, slot.candidate, reason);
  }

  scheduleLive(slot) {
    if (!slot.live || slot.liveToken !== null) return;
    slot.liveToken = this.requestFrame(() => {
      slot.liveToken = null;
      if (!slot.live || (slot.owner !== slot.live.nodeId && slot.candidate !== slot.live.nodeId)) return;
      this.renderLive(slot);
      this.scheduleLive(slot);
    });
  }

  renderLive(slot) {
    if (!slot.live) return;
    const frame = slot.live.frame;
    const backend = keyName(frame.type ?? frame["frame/type"] ?? frame["render/type"]);
    if (backend !== "canvas-2d") throw new Error(`unsupported published frame type: ${backend ?? "nil"}`);
    this.renderCanvas2d(slot, frame);
    slot.lastFrame = frame;
    this.resolveFirstRender(slot, slot.live.nodeId);
  }

  disposeWebGl(slot) {
    if (!slot.webgl?.gl) return;
    for (const program of slot.webgl.programs.values()) slot.webgl.gl.deleteProgram(program);
    slot.webgl = null;
  }

  close() {
    for (const remove of this.listeners.splice(0)) remove();
    for (const slot of this.canvases.values()) {
      this.cancelSlot(slot, "canvas runtime closed");
      this.disposeWebGl(slot);
    }
    this.canvases.clear();
    this.surfaces.clear();
    this.events.length = 0;
  }
}

function renderStateful2d(slot, context, stateful, width, height) {
  const kind = keyName(stateful.kind);
  if (kind === "ants") return renderAntsState(slot, context, stateful, width, height);
  if (kind === "boids") return renderBoidsState(slot, context, stateful, width, height);
  if (kind !== "tron") throw new Error(`unsupported stateful canvas: ${kind ?? "nil"}`);
  const reset = stateful.init || !slot.stateful || slot.stateful.kind !== "tron";
  if (reset) {
    slot.stateful = {
      kind: "tron",
      trails: (stateful.trails ?? []).map((trail) => trail.map(([x, y]) => [Number(x), Number(y)])),
      positions: [],
      actualPositions: [],
      starts: [],
      targets: [],
      velocities: [],
      lastTime: canvasNow(),
      lastEventTime: canvasNow(),
      transitionStartedAt: canvasNow(),
      transitionDuration: 0,
      lastEvent: null
    };
  }
  if (reset || slot.stateful.lastEvent !== stateful) {
    const now = canvasNow();
    const interval = Math.max(1, now - slot.stateful.lastEventTime);
    const trails = slot.stateful.trails;
    for (const [cycle, x, y] of stateful.append ?? []) {
      const trail = trails[Number(cycle)] ?? (trails[Number(cycle)] = []);
      trail.push([Number(x), Number(y)]);
      trimTronTrail(trail, width, height);
    }
    for (const [cycle, x, y] of stateful.reset ?? []) {
      trails[Number(cycle)] = [[Number(x), Number(y)]];
    }
    for (let cycle = 0; cycle < 4; cycle += 1) {
      const x = Number((stateful.heads ?? [])[cycle * 2]), y = Number((stateful.heads ?? [])[cycle * 2 + 1]);
      const previous = slot.stateful.actualPositions[cycle] ?? [x, y];
      slot.stateful.starts[cycle] = reset ? [x, y] : [...(slot.stateful.positions[cycle] ?? [x, y])];
      slot.stateful.targets[cycle] = [x, y];
      if (reset) slot.stateful.positions[cycle] = [x, y];
      slot.stateful.actualPositions[cycle] = [x, y];
      const [vx = 0, vy = 0] = (stateful.velocities ?? [])[cycle] ?? [];
      slot.stateful.velocities[cycle] = reset ? [Number(vx) * .06, Number(vy) * .06] : [(x - previous[0]) / interval, (y - previous[1]) / interval];
    }
    slot.stateful.lastEvent = stateful;
    slot.stateful.lastTime = now;
    slot.stateful.lastEventTime = now;
    slot.stateful.transitionStartedAt = now;
    slot.stateful.transitionDuration = reset ? 0 : Math.min(500, interval);
  } else {
    const now = canvasNow();
    const progress = slot.stateful.transitionDuration === 0 ? 1 : Math.min(1, Math.max(0, now - slot.stateful.transitionStartedAt) / slot.stateful.transitionDuration);
    slot.stateful.lastTime = now;
    for (let cycle = 0; cycle < 4; cycle += 1) {
      const position = slot.stateful.positions[cycle], start = slot.stateful.starts[cycle], target = slot.stateful.targets[cycle];
      position[0] = start[0] + (target[0] - start[0]) * progress;
      position[1] = start[1] + (target[1] - start[1]) * progress;
      const trail = slot.stateful.trails[cycle] ?? (slot.stateful.trails[cycle] = []);
      appendTronTrailPoint(trail, position[0], position[1]); trimTronTrail(trail, width, height);
    }
  }
  const trails = slot.stateful.trails;
  const colors = ["#41f5e4", "#ff2e88", "#9c7bff", "#f5d742"];
  for (let cycle = 0; cycle < 4; cycle += 1) {
    const points = trails[cycle] ?? [];
    const [x, y] = slot.stateful.positions[cycle] ?? [NaN, NaN];
    drawTronTrail(context, points, colors[cycle], x, y);
    if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
    context.fillStyle = "#efffff";
    context.globalAlpha = 1;
    context.beginPath();
    context.arc(x, y, 5, 0, Math.PI * 2);
    context.fill();
  }
}

function renderAntsState(slot, context, stateful, width, height) {
  if (!slot.stateful || slot.stateful.kind !== "ants") {
    slot.stateful = { kind: "ants", musk: new Map(), lastTime: canvasNow() };
  }
  const now = canvasNow();
  const fade = Math.pow(.5, Math.max(0, now - slot.stateful.lastTime) / 7000);
  slot.stateful.lastTime = now;
  for (const [key, amount] of slot.stateful.musk) {
    const next = amount * fade;
    if (next < 8) slot.stateful.musk.delete(key);
    else slot.stateful.musk.set(key, next);
  }
  for (const [x, y, amount] of stateful.musk ?? []) {
    const key = `${Number(x)},${Number(y)}`;
    slot.stateful.musk.set(key, Math.max(Number(amount), slot.stateful.musk.get(key) ?? 0));
  }
  const size = Math.max(width, height) / 80;
  const left = (width - size * 80) / 2, top = (height - size * 80) / 2;
  context.fillStyle = "rgba(16,45,92,.42)";
  context.fillRect(left + 20 * size, top + 20 * size, 7 * size, 7 * size);
  for (const [x, y, amount] of stateful.food ?? []) {
    if (Number(amount) <= 0) continue;
    context.fillStyle = "#ff2e88"; context.globalAlpha = Number(amount) > 30 ? 1 : .82;
    context.beginPath(); context.arc(left + (Number(x) + .5) * size, top + (Number(y) + .5) * size, Math.max(2, size / 2), 0, Math.PI * 2); context.fill();
  }
  for (const [key, amount] of slot.stateful.musk) {
    const [x, y] = key.split(",").map(Number);
    const strength = Math.min(1, amount / 1200);
    if (strength <= 0) continue;
    context.fillStyle = "#31ff8d"; context.globalAlpha = .04 + strength * .18;
    context.beginPath(); context.arc(left + (Number(x) + .5) * size, top + (Number(y) + .5) * size, Math.max(3, size * (1.2 + strength * 2.4)), 0, Math.PI * 2); context.fill();
  }
  for (const [x, y, direction, carrying] of stateful.ants ?? []) {
    const cx = left + (Number(x) + .5) * size, cy = top + (Number(y) + .5) * size;
    const color = carrying ? "#ffe93d" : "#eaffff";
    context.fillStyle = color; context.globalAlpha = 1; context.beginPath(); context.arc(cx, cy, Math.max(2, size / 2), 0, Math.PI * 2); context.fill();
    const delta = [[0,-1],[1,-1],[1,0],[1,1],[0,1],[-1,1],[-1,0],[-1,-1]][Number(direction)] ?? [0,0];
    context.strokeStyle = carrying ? "#ff8a3d" : "#41f5e4"; context.lineWidth = Math.max(1, size / 5); context.globalAlpha = .92;
    context.beginPath(); context.moveTo(cx, cy); context.lineTo(cx + delta[0] * size / 2, cy + delta[1] * size / 2); context.stroke();
  }
  context.globalAlpha = 1;
}

function renderBoidsState(slot, context, stateful, width, height) {
  const boids = stateful.boids ?? [];
  const reset = stateful.init || !slot.stateful || slot.stateful.kind !== "boids" || slot.stateful.tails.length !== boids.length;
  if (reset) {
    slot.stateful = {
      kind: "boids",
      tails: boids.map(([x, y]) => [[Number(x), Number(y)]]),
      positions: boids.map(([x, y]) => [Number(x), Number(y)]),
      actualPositions: boids.map(([x, y]) => [Number(x), Number(y)]),
      velocities: boids.map(() => [0, 0]),
      lastTime: canvasNow(),
      lastEventTime: canvasNow(),
      lastEvent: null
    };
  }
  if (reset || slot.stateful.lastEvent !== stateful) {
    const now = canvasNow();
    const interval = Math.max(1, now - slot.stateful.lastEventTime);
    for (let index = 0; index < boids.length; index += 1) {
      const [x, y, vx = 0, vy = 0] = boids[index];
      const tail = slot.stateful.tails[index];
      if (!reset) tail.push([Number(x), Number(y)]);
      while (tail.length > 18) tail.shift();
      const previous = slot.stateful.actualPositions[index] ?? [Number(x), Number(y)];
      slot.stateful.positions[index] = [Number(x), Number(y)];
      slot.stateful.actualPositions[index] = [Number(x), Number(y)];
      slot.stateful.velocities[index] = reset ? [Number(vx) * .06, Number(vy) * .06] : [(Number(x) - previous[0]) / interval, (Number(y) - previous[1]) / interval];
    }
    slot.stateful.lastEvent = stateful;
    slot.stateful.lastTime = now;
    slot.stateful.lastEventTime = now;
  } else {
    const now = canvasNow();
    const elapsed = Math.min(50, Math.max(0, now - slot.stateful.lastTime));
    slot.stateful.lastTime = now;
    for (let index = 0; index < boids.length; index += 1) {
      const position = slot.stateful.positions[index];
      const [vx, vy] = slot.stateful.velocities[index];
      position[0] += vx * elapsed;
      position[1] += vy * elapsed;
      const tail = slot.stateful.tails[index];
      tail.push([position[0], position[1]]);
      while (tail.length > 18) tail.shift();
    }
  }
  for (let index = 0; index < boids.length; index += 1) {
    const [x, y] = slot.stateful.positions[index];
    const color = index % 3 === 0 ? "#9c7bff" : "#41f5e4";
    drawBoidTail(context, slot.stateful.tails[index], color);
    context.fillStyle = "#efffff"; context.globalAlpha = .94; context.beginPath(); context.arc(Number(x), Number(y), 3, 0, Math.PI * 2); context.fill();
  }
  context.globalAlpha = 1;
}

function trimTronTrail(trail, width, height) {
  const limit = Math.max(width, height);
  let distance = 0;
  for (let index = trail.length - 1; index > 0; index -= 1) {
    const [x, y] = trail[index], [previousX, previousY] = trail[index - 1];
    distance += Math.hypot(x - previousX, y - previousY);
    if (distance <= limit) continue;
    trail.splice(0, index);
    return;
  }
}

function appendTronTrailPoint(trail, x, y) {
  const previous = trail.at(-1);
  if (!previous || Math.hypot(x - previous[0], y - previous[1]) >= 3) trail.push([x, y]);
}

function drawTronTrail(context, points, color, headX, headY) {
  if (points.length === 0 || !Number.isFinite(headX) || !Number.isFinite(headY)) return;
  for (const [lineWidth, alpha] of [[13, .14], [3, .92]]) {
    context.strokeStyle = color;
    context.lineWidth = lineWidth;
    context.globalAlpha = alpha;
    context.beginPath();
    context.moveTo(points[0][0], points[0][1]);
    for (let index = 1; index < points.length; index += 1) {
      context.lineTo(points[index][0], points[index][1]);
    }
    if (Number.isFinite(headX) && Number.isFinite(headY)) context.lineTo(headX, headY);
    context.stroke();
  }
  context.globalAlpha = 1;
}

function drawBoidTail(context, points, color) {
  if (points.length < 2) return;
  for (const [lineWidth, alpha] of [[7, .12], [2, .68]]) {
    context.strokeStyle = color; context.lineWidth = lineWidth; context.globalAlpha = alpha;
    context.beginPath(); context.moveTo(points[0][0], points[0][1]);
    for (let index = 1; index < points.length; index += 1) {
      const point = points[index], previous = points[index - 1];
      const wobble = Math.sin(index * 1.7) * Math.min(4, lineWidth + 1);
      const dx = point[0] - previous[0], dy = point[1] - previous[1];
      const length = Math.hypot(dx, dy) || 1;
      context.lineTo(point[0] - dy / length * wobble, point[1] + dx / length * wobble);
    }
    context.stroke();
  }
}

function canvasNow() {
  return globalThis.performance?.now?.() ?? Date.now();
}

export function resolutionUniform(name, value, width, height) {
  if (name !== "u_resolution" && name !== "iResolution") return value;
  if (value.length === 2) return [width, height];
  if (value.length === 3) return [width, height, value[2]];
  return value;
}

function execute2d(context, command, width, height) {
  if (!Array.isArray(command) || command.length === 0) return;
  const name = keyName(command[0]);
  if (name === "grid") {
    const spacing = Number(command[1] ?? 48);
    context.strokeStyle = command[2] ?? "rgba(65,245,228,.08)";
    context.lineWidth = Number(command[3] ?? 1);
    context.beginPath();
    for (let x = 0; x <= width; x += spacing) { context.moveTo(x, 0); context.lineTo(x, height); }
    for (let y = 0; y <= height; y += spacing) { context.moveTo(0, y); context.lineTo(width, y); }
    context.stroke();
  } else if (name === "mist") {
    const x = Number(command[1]);
    const y = Number(command[2]);
    const radius = Math.max(1, Number(command[3] ?? 48));
    const color = command[4] ?? "#41f5e4";
    const alpha = Number(command[5] ?? .12);
    const glow = context.createRadialGradient(x, y, 0, x, y, radius);
    glow.addColorStop(0, colorWithAlpha(color, alpha));
    glow.addColorStop(1, colorWithAlpha(color, 0));
    context.fillStyle = glow;
    context.beginPath();
    context.arc(x, y, radius, 0, Math.PI * 2);
    context.fill();
  } else if (name === "line") {
    context.strokeStyle = command[5] ?? "#41f5e4";
    context.lineWidth = Number(command[6] ?? 2);
    context.globalAlpha = Number(command[7] ?? 1);
    context.beginPath();
    context.moveTo(Number(command[1]), Number(command[2]));
    context.lineTo(Number(command[3]), Number(command[4]));
    context.stroke();
    context.globalAlpha = 1;
  } else if (name === "polyline") {
    const points = command[1] ?? [];
    if (points.length < 2) return;
    context.strokeStyle = command[2] ?? "#41f5e4";
    context.lineWidth = Number(command[3] ?? 2);
    context.globalAlpha = Number(command[4] ?? 1);
    context.beginPath();
    context.moveTo(Number(points[0][0]), Number(points[0][1]));
    for (const point of points.slice(1)) context.lineTo(Number(point[0]), Number(point[1]));
    context.stroke();
    context.globalAlpha = 1;
  } else if (name === "rect") {
    context.fillStyle = command[5] ?? "#41f5e4";
    context.globalAlpha = Number(command[6] ?? 1);
    context.fillRect(Number(command[1]), Number(command[2]), Number(command[3]), Number(command[4]));
    context.globalAlpha = 1;
  } else if (name === "circle") {
    context.fillStyle = command[4] ?? "#41f5e4";
    context.globalAlpha = Number(command[5] ?? 1);
    context.beginPath();
    context.arc(Number(command[1]), Number(command[2]), Number(command[3]), 0, Math.PI * 2);
    context.fill();
    context.globalAlpha = 1;
  } else if (name === "mist") {
    const x = Number(command[1]);
    const y = Number(command[2]);
    const radius = Math.max(1, Number(command[3] ?? 24));
    const color = command[4] ?? "#31ff8d";
    const alpha = Number(command[5] ?? 0.2);
    const gradient = context.createRadialGradient(x, y, 0, x, y, radius);
    gradient.addColorStop(0, color);
    gradient.addColorStop(0.28, color);
    gradient.addColorStop(1, "rgba(0,0,0,0)");
    context.save();
    context.globalCompositeOperation = "lighter";
    context.globalAlpha = alpha;
    context.fillStyle = gradient;
    context.fillRect(x - radius, y - radius, radius * 2, radius * 2);
    context.restore();
  } else if (name === "text") {
    context.fillStyle = command[4] ?? "#eaffff";
    context.font = `${Number(command[5] ?? 12)}px ui-monospace, monospace`;
    context.fillText(String(command[1]), Number(command[2]), Number(command[3]));
  }
}

function linkProgram(gl, vertexSource, fragmentSource) {
  const compile = (type, source, label) => {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      const log = gl.getShaderInfoLog(shader);
      gl.deleteShader(shader);
      throw new Error(`${label} shader: ${log}`);
    }
    return shader;
  };
  const vertex = compile(gl.VERTEX_SHADER, vertexSource, "vertex");
  const fragment = compile(gl.FRAGMENT_SHADER, fragmentSource, "fragment");
  const program = gl.createProgram();
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  gl.deleteShader(vertex);
  gl.deleteShader(fragment);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(program);
    gl.deleteProgram(program);
    throw new Error(`program link: ${log}`);
  }
  return program;
}

function plain(value) {
  if (value === null || typeof value !== "object") return value;
  if (value instanceof Map) {
    return Object.fromEntries([...value].map(([key, entry]) => [keyName(key), plain(entry)]));
  }
  if (Array.isArray(value)) return value.map(plain);
  const type = value.constructor?.name;
  if (["HtaObject", "HtaOrderedMap", "HtaSortedMap", "HtaTrie", "HtaPriorityMap"].includes(type)) {
    return Object.fromEntries(value.entries.map(([key, entry]) => [keyName(key), plain(entry)]));
  }
  if (["HtaArray", "HtaTuple", "HtaCons", "HtaQueue", "HtaDeque", "HtaOrderedSet", "HtaSortedSet"].includes(type)) {
    return value.values.map(plain);
  }
  return value;
}

function toHta(value) {
  if (Array.isArray(value)) return value.map(toHta);
  if (value && typeof value === "object") {
    return new Map(Object.entries(value).map(([key, entry]) => [key, toHta(entry)]));
  }
  return value;
}

function keyName(value) {
  return value?.constructor?.name === "HtaKeyword" ? value.name : String(value);
}

function colorWithAlpha(color, alpha) {
  if (typeof color !== "string" || !color.startsWith("#")) return color;
  const hex = color.slice(1);
  const normalized = hex.length === 3
    ? hex.split("").map((part) => part + part).join("")
    : hex;
  if (!/^[0-9a-f]{6}$/i.test(normalized)) return color;
  const red = parseInt(normalized.slice(0, 2), 16);
  const green = parseInt(normalized.slice(2, 4), 16);
  const blue = parseInt(normalized.slice(4, 6), 16);
  return `rgba(${red}, ${green}, ${blue}, ${Math.max(0, Math.min(1, alpha))})`;
}

function point(event) {
  return {
    x: Math.round(event.clientX ?? 0),
    y: Math.round(event.clientY ?? 0),
    button: event.button ?? 0,
    pointer: event.pointerType ?? "mouse"
  };
}

function modifiers(event) {
  return [event.ctrlKey && "ctrl", event.altKey && "alt", event.shiftKey && "shift", event.metaKey && "meta"]
    .filter(Boolean);
}

function structuredError(code, message) {
  const error = new Error(message);
  error.code = code;
  error.origin = "studio.canvas";
  error.retryable = false;
  return error;
}

function hashText(text) {
  let hash = 2166136261;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(16);
}
