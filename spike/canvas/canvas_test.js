// Canvas spike (spec v5 §32, item 6): can an in-house WebGL2 renderer draw the Designer's graph and
// Psychology's 3D view fast enough, in Chrome and in WebKitGTK (Tauri's webview on Linux)?
//
// Scenes, each measured for a few seconds after a warm-up:
//   graph:    N nodes (rounded rectangles with text labels) and ~1.2·N edges, all on screen at once, while the
//             camera pans and zooms and every node's colour changes each frame (live highlighting).
//   flow:     Psychology's 3D data-flow view: layers × tokens cubes whose values are re-uploaded every frame.
//   combined: a 1,000-node graph and the 26 × 512 flow view side by side, the spec's target.
//
// Everything is instanced: each scene is 1-3 draw calls per frame, whatever N is.
// Query string: ?measure=3000 (ms per config), or ?preview=graph|flow|combined&count=N for a static look.

const WARMUP_MILLISECONDS = 500;
const DEFAULT_MEASURE_MILLISECONDS = 3000;
const IDLE_REFRESH_PROBE_MILLISECONDS = 1000;
const GRAPH_NODE_COUNTS = [1000, 2000, 5000, 10000, 20000, 50000];
const FLOW_SHAPES = [[26, 512], [26, 2048], [26, 8192]]; // [layers, tokens]
const COMBINED_SCENE = { nodes: 1000, layers: 26, tokens: 512 };
const RESULTS_URL = "/results";
// Headroom: every config above runs at the display's refresh cap, so the combined scene is also drawn
// N times per frame, doubling N until the frame rate drops below the cap.
const HEADROOM_REPEATS = [1, 2, 4, 8, 16, 32, 64];
const HEADROOM_MEASURE_MILLISECONDS = 1500;
const AT_CAP_FRACTION = 0.95;

const NODES_PER_COLUMN = 40;
const NODE_WIDTH = 160;
const NODE_HEIGHT = 40;
const COLUMN_GAP = 70;
const ROW_GAP = 24;
const SKIP_EDGE_EVERY = 5; // every 5th node also connects to a node in the next column, like a residual path
const EDGE_WIDTH_PIXELS = 1.5;
const NODE_KINDS = ["embedding", "attention", "feedforward", "rms_norm", "residual", "lora", "sae_lens", "head"];

const ATLAS_SIZE = 1024;
const ATLAS_FONT_PIXELS = 28;
const FIRST_GLYPH_CODE = 32;
const LAST_GLYPH_CODE = 126;
const LABEL_PADDING = 8;
const GLYPH_WORLD_HEIGHT = 18;

const FLOW_CELL_SPACING = 1.0;
const FLOW_LAYER_SPACING = 3.0;
const BACKGROUND = [0.063, 0.075, 0.094, 1.0];

const canvas = document.getElementById("surface");
const statusLine = document.getElementById("status");

// ----------------------------------------------------------------------------------------------
// WebGL plumbing
// ----------------------------------------------------------------------------------------------

function compileShader(gl, type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    throw new Error("shader compile failed: " + gl.getShaderInfoLog(shader));
  }
  return shader;
}

function createProgram(gl, vertexSource, fragmentSource) {
  const program = gl.createProgram();
  gl.attachShader(program, compileShader(gl, gl.VERTEX_SHADER, vertexSource));
  gl.attachShader(program, compileShader(gl, gl.FRAGMENT_SHADER, fragmentSource));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error("program link failed: " + gl.getProgramInfoLog(program));
  }
  return program;
}

function uniformLocations(gl, program, names) {
  const locations = {};
  for (const name of names) locations[name] = gl.getUniformLocation(program, name);
  return locations;
}

// One vertex buffer attribute. `divisor` 1 means one value per instance instead of per vertex.
function bindAttribute(gl, location, buffer, size, type, stride, offset, divisor, normalized = false) {
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.enableVertexAttribArray(location);
  gl.vertexAttribPointer(location, size, type, normalized, stride, offset);
  gl.vertexAttribDivisor(location, divisor);
}

function createBuffer(gl, data, usage) {
  const buffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, data, usage);
  return buffer;
}

// A unit quad as a 4-vertex triangle strip; instances stretch it into nodes, edges and glyphs.
const UNIT_QUAD = new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]);

const SCREEN_FROM_PIXELS = `
  vec4 screenPosition(vec2 pixel, vec2 viewportSize) {
    vec2 clip = pixel / viewportSize * 2.0 - 1.0;
    return vec4(clip.x, -clip.y, 0.0, 1.0);
  }`;

// ----------------------------------------------------------------------------------------------
// Graph scene: nodes, edges and labels
// ----------------------------------------------------------------------------------------------

function layoutGraph(nodeCount) {
  const nodeRectangles = new Float32Array(nodeCount * 4);
  const labels = [];
  for (let index = 0; index < nodeCount; index++) {
    const column = Math.floor(index / NODES_PER_COLUMN);
    const row = index % NODES_PER_COLUMN;
    const x = column * (NODE_WIDTH + COLUMN_GAP);
    const y = row * (NODE_HEIGHT + ROW_GAP);
    nodeRectangles.set([x, y, NODE_WIDTH, NODE_HEIGHT], index * 4);
    labels.push({ x: x + LABEL_PADDING, y: y + (NODE_HEIGHT - GLYPH_WORLD_HEIGHT) / 2, text: `${NODE_KINDS[index % NODE_KINDS.length]} ${index}` });
  }

  const segments = [];
  for (let index = 0; index < nodeCount; index++) {
    const [x, y] = [nodeRectangles[index * 4], nodeRectangles[index * 4 + 1]];
    const sameColumnNext = index + 1 < nodeCount && (index + 1) % NODES_PER_COLUMN !== 0;
    if (sameColumnNext) segments.push(x + NODE_WIDTH / 2, y + NODE_HEIGHT, x + NODE_WIDTH / 2, y + NODE_HEIGHT + ROW_GAP);
    const skipTarget = index + NODES_PER_COLUMN + 3;
    if (index % SKIP_EDGE_EVERY === 0 && skipTarget < nodeCount) {
      const [targetX, targetY] = [nodeRectangles[skipTarget * 4], nodeRectangles[skipTarget * 4 + 1]];
      segments.push(x + NODE_WIDTH, y + NODE_HEIGHT / 2, targetX, targetY + NODE_HEIGHT / 2);
    }
  }

  const columns = Math.ceil(nodeCount / NODES_PER_COLUMN);
  const worldWidth = columns * (NODE_WIDTH + COLUMN_GAP) - COLUMN_GAP;
  const worldHeight = Math.min(nodeCount, NODES_PER_COLUMN) * (NODE_HEIGHT + ROW_GAP) - ROW_GAP;
  return { nodeRectangles, segments: new Float32Array(segments), labels, worldWidth, worldHeight };
}

// White glyphs on transparent, drawn with the browser's own text engine once, then sampled by the GPU.
function buildGlyphAtlas(gl) {
  const atlasCanvas = document.createElement("canvas");
  atlasCanvas.width = atlasCanvas.height = ATLAS_SIZE;
  const context = atlasCanvas.getContext("2d");
  context.font = `${ATLAS_FONT_PIXELS}px monospace`;
  const cellWidth = Math.ceil(context.measureText("M").width) + 2;
  const cellHeight = ATLAS_FONT_PIXELS + 8;
  const columns = Math.floor(ATLAS_SIZE / cellWidth);
  context.fillStyle = "white";
  context.textBaseline = "top";
  for (let code = FIRST_GLYPH_CODE; code <= LAST_GLYPH_CODE; code++) {
    const index = code - FIRST_GLYPH_CODE;
    context.fillText(String.fromCharCode(code), (index % columns) * cellWidth + 1, Math.floor(index / columns) * cellHeight + 4);
  }

  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, atlasCanvas);
  gl.generateMipmap(gl.TEXTURE_2D); // labels shrink a lot when zoomed out
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  return { texture, columns, cellUV: [cellWidth / ATLAS_SIZE, cellHeight / ATLAS_SIZE], aspect: cellWidth / cellHeight };
}

function glyphInstances(labels, atlas) {
  const glyphWidth = GLYPH_WORLD_HEIGHT * atlas.aspect;
  const values = [];
  for (const label of labels) {
    for (let position = 0; position < label.text.length; position++) {
      const code = label.text.charCodeAt(position);
      if (code === FIRST_GLYPH_CODE) continue; // spaces draw nothing
      values.push(label.x + position * glyphWidth, label.y, code - FIRST_GLYPH_CODE);
    }
  }
  return { data: new Float32Array(values), glyphSize: [glyphWidth, GLYPH_WORLD_HEIGHT] };
}

const CAMERA_UNIFORMS = `uniform vec2 cameraOffset; uniform float cameraScale; uniform vec2 viewportSize;`;

const EDGE_VERTEX_SHADER = `#version 300 es
layout(location = 0) in vec2 corner;     // x: 0..1 along the edge, y: 0..1 across it
layout(location = 1) in vec4 segment;    // world start (xy) and end (zw)
${CAMERA_UNIFORMS}
uniform float widthPixels;
${SCREEN_FROM_PIXELS}
void main() {
  vec2 start = (segment.xy - cameraOffset) * cameraScale;
  vec2 end = (segment.zw - cameraOffset) * cameraScale;
  vec2 direction = normalize(end - start + vec2(1e-5, 0.0));
  vec2 across = vec2(-direction.y, direction.x) * (corner.y - 0.5) * widthPixels;
  gl_Position = screenPosition(mix(start, end, corner.x) + across, viewportSize);
}`;

const EDGE_FRAGMENT_SHADER = `#version 300 es
precision mediump float;
out vec4 outColor;
void main() { outColor = vec4(0.42, 0.50, 0.60, 1.0); }`;

const NODE_VERTEX_SHADER = `#version 300 es
layout(location = 0) in vec2 corner;
layout(location = 1) in vec4 rectangle;  // world x, y, width, height
layout(location = 2) in vec4 color;      // changes every frame
${CAMERA_UNIFORMS}
out vec2 localPixel;
out vec2 halfSizePixels;
out vec4 fillColor;
${SCREEN_FROM_PIXELS}
void main() {
  vec2 sizePixels = rectangle.zw * cameraScale;
  localPixel = (corner - 0.5) * sizePixels;
  halfSizePixels = sizePixels * 0.5;
  fillColor = color;
  gl_Position = screenPosition((rectangle.xy - cameraOffset) * cameraScale + corner * sizePixels, viewportSize);
}`;

// Rounded rectangle from a signed distance: anti-aliased edge and a 1-pixel border, no geometry needed.
const NODE_FRAGMENT_SHADER = `#version 300 es
precision mediump float;
in vec2 localPixel;
in vec2 halfSizePixels;
in vec4 fillColor;
out vec4 outColor;
void main() {
  float radius = min(halfSizePixels.x, halfSizePixels.y) * 0.3;
  vec2 q = abs(localPixel) - halfSizePixels + radius;
  float distance = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
  float coverage = clamp(0.5 - distance, 0.0, 1.0);
  float border = clamp(1.0 - abs(distance + 1.0), 0.0, 1.0);
  outColor = vec4(mix(fillColor.rgb, vec3(0.85, 0.90, 0.95), border), coverage);
}`;

const GLYPH_VERTEX_SHADER = `#version 300 es
layout(location = 0) in vec2 corner;
layout(location = 1) in vec3 glyph;      // world x, y, atlas index
${CAMERA_UNIFORMS}
uniform vec2 glyphWorldSize;
uniform vec2 atlasCellUV;
uniform float atlasColumns;
out vec2 atlasUV;
${SCREEN_FROM_PIXELS}
void main() {
  float column = mod(glyph.z, atlasColumns);
  float row = floor(glyph.z / atlasColumns);
  atlasUV = (vec2(column, row) + corner) * atlasCellUV;
  vec2 world = glyph.xy + corner * glyphWorldSize;
  gl_Position = screenPosition((world - cameraOffset) * cameraScale, viewportSize);
}`;

const GLYPH_FRAGMENT_SHADER = `#version 300 es
precision mediump float;
in vec2 atlasUV;
uniform sampler2D atlas;
out vec4 outColor;
void main() { outColor = vec4(0.95, 0.97, 1.0, texture(atlas, atlasUV).a); }`;

function createGraphScene(gl, nodeCount) {
  const graph = layoutGraph(nodeCount);
  const atlas = buildGlyphAtlas(gl);
  const glyphs = glyphInstances(graph.labels, atlas);
  const quad = createBuffer(gl, UNIT_QUAD, gl.STATIC_DRAW);
  const nodeColors = new Uint8Array(nodeCount * 4);
  const nodeColorBuffer = createBuffer(gl, nodeColors, gl.DYNAMIC_DRAW);

  const programs = {
    edge: createProgram(gl, EDGE_VERTEX_SHADER, EDGE_FRAGMENT_SHADER),
    node: createProgram(gl, NODE_VERTEX_SHADER, NODE_FRAGMENT_SHADER),
    glyph: createProgram(gl, GLYPH_VERTEX_SHADER, GLYPH_FRAGMENT_SHADER),
  };
  const cameraNames = ["cameraOffset", "cameraScale", "viewportSize"];
  const uniforms = {
    edge: uniformLocations(gl, programs.edge, [...cameraNames, "widthPixels"]),
    node: uniformLocations(gl, programs.node, cameraNames),
    glyph: uniformLocations(gl, programs.glyph, [...cameraNames, "glyphWorldSize", "atlasCellUV", "atlasColumns", "atlas"]),
  };

  const vertexArrays = { edge: gl.createVertexArray(), node: gl.createVertexArray(), glyph: gl.createVertexArray() };
  gl.bindVertexArray(vertexArrays.edge);
  bindAttribute(gl, 0, quad, 2, gl.FLOAT, 0, 0, 0);
  bindAttribute(gl, 1, createBuffer(gl, graph.segments, gl.STATIC_DRAW), 4, gl.FLOAT, 0, 0, 1);
  gl.bindVertexArray(vertexArrays.node);
  bindAttribute(gl, 0, quad, 2, gl.FLOAT, 0, 0, 0);
  bindAttribute(gl, 1, createBuffer(gl, graph.nodeRectangles, gl.STATIC_DRAW), 4, gl.FLOAT, 0, 0, 1);
  bindAttribute(gl, 2, nodeColorBuffer, 4, gl.UNSIGNED_BYTE, 0, 0, 1, true);
  gl.bindVertexArray(vertexArrays.glyph);
  bindAttribute(gl, 0, quad, 2, gl.FLOAT, 0, 0, 0);
  bindAttribute(gl, 1, createBuffer(gl, glyphs.data, gl.STATIC_DRAW), 3, gl.FLOAT, 0, 0, 1);
  gl.bindVertexArray(null);

  const edgeCount = graph.segments.length / 4;
  const glyphCount = glyphs.data.length / 3;

  function updateNodeColors(timeSeconds) {
    // A highlight wave sweeping through the graph: every node's colour changes every frame.
    for (let index = 0; index < nodeCount; index++) {
      const wave = 0.5 + 0.5 * Math.sin(timeSeconds * 3.0 - index * 0.02);
      nodeColors[index * 4] = 40 + wave * 80;
      nodeColors[index * 4 + 1] = 70 + wave * 60;
      nodeColors[index * 4 + 2] = 110 + (1 - wave) * 60;
      nodeColors[index * 4 + 3] = 255;
    }
    gl.bindBuffer(gl.ARRAY_BUFFER, nodeColorBuffer);
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, nodeColors);
  }

  function camera(timeSeconds, viewport) {
    // Keep the whole graph on screen (every node visible: the worst case), while panning and zooming.
    const fitScale = 0.92 * Math.min(viewport.width / graph.worldWidth, viewport.height / graph.worldHeight);
    const scale = fitScale * (1.0 + 0.08 * Math.sin(timeSeconds * 0.9));
    const centerX = graph.worldWidth / 2 + graph.worldWidth * 0.02 * Math.sin(timeSeconds * 0.7);
    const centerY = graph.worldHeight / 2 + graph.worldHeight * 0.02 * Math.cos(timeSeconds * 0.6);
    return { scale, offset: [centerX - viewport.width / 2 / scale, centerY - viewport.height / 2 / scale] };
  }

  function setCamera(names, view, viewport) {
    gl.uniform2f(names.cameraOffset, view.offset[0], view.offset[1]);
    gl.uniform1f(names.cameraScale, view.scale);
    gl.uniform2f(names.viewportSize, viewport.width, viewport.height);
  }

  function draw(timeSeconds, viewport) {
    updateNodeColors(timeSeconds);
    const view = camera(timeSeconds, viewport);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.disable(gl.DEPTH_TEST);

    gl.useProgram(programs.edge);
    setCamera(uniforms.edge, view, viewport);
    gl.uniform1f(uniforms.edge.widthPixels, EDGE_WIDTH_PIXELS);
    gl.bindVertexArray(vertexArrays.edge);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, edgeCount);

    gl.useProgram(programs.node);
    setCamera(uniforms.node, view, viewport);
    gl.bindVertexArray(vertexArrays.node);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, nodeCount);

    gl.useProgram(programs.glyph);
    setCamera(uniforms.glyph, view, viewport);
    gl.uniform2f(uniforms.glyph.glyphWorldSize, glyphs.glyphSize[0], glyphs.glyphSize[1]);
    gl.uniform2f(uniforms.glyph.atlasCellUV, atlas.cellUV[0], atlas.cellUV[1]);
    gl.uniform1f(uniforms.glyph.atlasColumns, atlas.columns);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, atlas.texture);
    gl.uniform1i(uniforms.glyph.atlas, 0);
    gl.bindVertexArray(vertexArrays.glyph);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, glyphCount);
    gl.bindVertexArray(null);
  }

  return { draw, counts: { nodes: nodeCount, edges: edgeCount, glyphs: glyphCount } };
}

// ----------------------------------------------------------------------------------------------
// Flow scene: Psychology's 3D layers × tokens view
// ----------------------------------------------------------------------------------------------

function cubeVertices() {
  // 6 faces × 2 triangles × 3 vertices; each vertex is position (xyz) then normal (xyz).
  const faces = [
    [[1, 0, 0], [[1, -1, -1], [1, 1, -1], [1, 1, 1], [1, -1, 1]]],
    [[-1, 0, 0], [[-1, -1, 1], [-1, 1, 1], [-1, 1, -1], [-1, -1, -1]]],
    [[0, 1, 0], [[-1, 1, -1], [-1, 1, 1], [1, 1, 1], [1, 1, -1]]],
    [[0, -1, 0], [[-1, -1, 1], [-1, -1, -1], [1, -1, -1], [1, -1, 1]]],
    [[0, 0, 1], [[-1, -1, 1], [1, -1, 1], [1, 1, 1], [-1, 1, 1]]],
    [[0, 0, -1], [[1, -1, -1], [-1, -1, -1], [-1, 1, -1], [1, 1, -1]]],
  ];
  const values = [];
  for (const [normal, corners] of faces) {
    for (const cornerIndex of [0, 1, 2, 0, 2, 3]) values.push(...corners[cornerIndex].map((v) => v * 0.5), ...normal);
  }
  return new Float32Array(values);
}

function perspective(fieldOfViewRadians, aspect, near, far) {
  const f = 1 / Math.tan(fieldOfViewRadians / 2);
  const depth = 1 / (near - far);
  return [f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) * depth, -1, 0, 0, 2 * far * near * depth, 0];
}

function lookAt(eye, target, up) {
  const subtract = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
  const normalize = (v) => { const length = Math.hypot(...v); return v.map((x) => x / length); };
  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const forward = normalize(subtract(eye, target));
  const right = normalize(cross(up, forward));
  const trueUp = cross(forward, right);
  return [right[0], trueUp[0], forward[0], 0, right[1], trueUp[1], forward[1], 0, right[2], trueUp[2], forward[2], 0,
    -dot(right, eye), -dot(trueUp, eye), -dot(forward, eye), 1];
}

function multiply(a, b) {
  // Column-major 4×4: result = a · b
  const result = new Array(16).fill(0);
  for (let column = 0; column < 4; column++) {
    for (let row = 0; row < 4; row++) {
      for (let k = 0; k < 4; k++) result[column * 4 + row] += a[k * 4 + row] * b[column * 4 + k];
    }
  }
  return result;
}

const FLOW_VERTEX_SHADER = `#version 300 es
layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 cell;        // layer, token
layout(location = 3) in float activation; // changes every frame
uniform mat4 viewProjection;
uniform vec2 gridSize;                    // layers, tokens
uniform float cellSpacing;
uniform float layerSpacing;
out vec3 shadedColor;
void main() {
  float height = 0.15 + abs(activation) * 2.5;
  vec3 scale = vec3(cellSpacing * 0.8, height, cellSpacing * 0.8);
  vec3 base = vec3((cell.y - gridSize.y * 0.5) * cellSpacing, height * 0.5, (cell.x - gridSize.x * 0.5) * layerSpacing);
  gl_Position = viewProjection * vec4(base + position * scale, 1.0);
  vec3 cold = vec3(0.20, 0.45, 0.95);
  vec3 warm = vec3(0.95, 0.35, 0.20);
  vec3 color = mix(vec3(0.25), activation < 0.0 ? cold : warm, clamp(abs(activation), 0.0, 1.0));
  float light = 0.35 + 0.65 * max(dot(normal, normalize(vec3(0.4, 0.8, 0.3))), 0.0);
  shadedColor = color * light;
}`;

const FLOW_FRAGMENT_SHADER = `#version 300 es
precision mediump float;
in vec3 shadedColor;
out vec4 outColor;
void main() { outColor = vec4(shadedColor, 1.0); }`;

function createFlowScene(gl, layerCount, tokenCount) {
  const instanceCount = layerCount * tokenCount;
  const cells = new Float32Array(instanceCount * 2);
  for (let layer = 0; layer < layerCount; layer++) {
    for (let token = 0; token < tokenCount; token++) cells.set([layer, token], (layer * tokenCount + token) * 2);
  }
  const activations = new Float32Array(instanceCount);
  const activationBuffer = createBuffer(gl, activations, gl.DYNAMIC_DRAW);

  const program = createProgram(gl, FLOW_VERTEX_SHADER, FLOW_FRAGMENT_SHADER);
  const uniforms = uniformLocations(gl, program, ["viewProjection", "gridSize", "cellSpacing", "layerSpacing"]);
  const vertexArray = gl.createVertexArray();
  const vertexStride = 6 * 4;
  gl.bindVertexArray(vertexArray);
  const cube = createBuffer(gl, cubeVertices(), gl.STATIC_DRAW);
  bindAttribute(gl, 0, cube, 3, gl.FLOAT, vertexStride, 0, 0);
  bindAttribute(gl, 1, cube, 3, gl.FLOAT, vertexStride, 12, 0);
  bindAttribute(gl, 2, createBuffer(gl, cells, gl.STATIC_DRAW), 2, gl.FLOAT, 0, 0, 1);
  bindAttribute(gl, 3, activationBuffer, 1, gl.FLOAT, 0, 0, 1);
  gl.bindVertexArray(null);

  const worldWidth = tokenCount * FLOW_CELL_SPACING;
  const worldDepth = layerCount * FLOW_LAYER_SPACING;

  function updateActivations(timeSeconds) {
    // Stand-in for live per-token readouts streaming in: every value changes every frame.
    for (let layer = 0; layer < layerCount; layer++) {
      const layerPhase = layer * 0.35 + timeSeconds * 2.0;
      for (let token = 0; token < tokenCount; token++) {
        activations[layer * tokenCount + token] = Math.sin(layerPhase + token * 0.07) * Math.cos(token * 0.013 - timeSeconds);
      }
    }
    gl.bindBuffer(gl.ARRAY_BUFFER, activationBuffer);
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, activations);
  }

  function draw(timeSeconds, viewport) {
    updateActivations(timeSeconds);
    // Frame the whole grid (every cube on screen), swinging gently around it rather than orbiting,
    // so the long tokens axis never leaves the view.
    const fieldOfView = Math.PI / 4;
    const horizontalFit = (worldWidth * 0.5) / Math.tan(fieldOfView / 2) / Math.max(viewport.width / viewport.height, 1e-3);
    const distance = Math.max(horizontalFit, worldDepth) * 1.15 + 10;
    const angle = 0.35 * Math.sin(timeSeconds * 0.4);
    const eye = [Math.sin(angle) * distance, distance * 0.6, Math.cos(angle) * distance];
    const projection = perspective(Math.PI / 4, viewport.width / viewport.height, 1, distance * 4);
    const viewProjection = multiply(projection, lookAt(eye, [0, 0, 0], [0, 1, 0]));

    gl.disable(gl.BLEND);
    gl.enable(gl.DEPTH_TEST);
    gl.useProgram(program);
    gl.uniformMatrix4fv(uniforms.viewProjection, false, viewProjection);
    gl.uniform2f(uniforms.gridSize, layerCount, tokenCount);
    gl.uniform1f(uniforms.cellSpacing, FLOW_CELL_SPACING);
    gl.uniform1f(uniforms.layerSpacing, FLOW_LAYER_SPACING);
    gl.bindVertexArray(vertexArray);
    gl.drawArraysInstanced(gl.TRIANGLES, 0, 36, instanceCount);
    gl.bindVertexArray(null);
  }

  return { draw, counts: { cubes: instanceCount, triangles: instanceCount * 12 } };
}

// ----------------------------------------------------------------------------------------------
// Frame loop and measurement
// ----------------------------------------------------------------------------------------------

function resizeCanvas() {
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.round(window.innerWidth * ratio);
  canvas.height = Math.round(window.innerHeight * ratio);
}

function nextFrame() {
  return new Promise((resolve) => requestAnimationFrame(resolve));
}

function percentile(sortedValues, fraction) {
  if (sortedValues.length === 0) return null;
  return sortedValues[Math.min(sortedValues.length - 1, Math.floor(fraction * sortedValues.length))];
}

// Read the frame back (same callback as the draw, before compositing clears it) and count the pixels that
// differ from the background. Zero means nothing was drawn.
function drawnPixelFraction(gl) {
  const pixels = new Uint8Array(canvas.width * canvas.height * 4);
  gl.readPixels(0, 0, canvas.width, canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
  const background = BACKGROUND.map((channel) => Math.round(channel * 255));
  let drawn = 0;
  for (let offset = 0; offset < pixels.length; offset += 4) {
    const differs = Math.abs(pixels[offset] - background[0]) > 3 || Math.abs(pixels[offset + 1] - background[1]) > 3 || Math.abs(pixels[offset + 2] - background[2]) > 3;
    if (differs) drawn++;
  }
  return drawn / (canvas.width * canvas.height);
}

function summarize(intervals, callbackTimes, elapsedMilliseconds) {
  const sortedIntervals = [...intervals].sort((a, b) => a - b);
  const sortedCallbacks = [...callbackTimes].sort((a, b) => a - b);
  return {
    frames: intervals.length,
    fps: (intervals.length / elapsedMilliseconds) * 1000,
    frame_ms_p50: percentile(sortedIntervals, 0.5),
    frame_ms_p95: percentile(sortedIntervals, 0.95),
    frame_ms_p99: percentile(sortedIntervals, 0.99),
    frame_ms_max: sortedIntervals[sortedIntervals.length - 1],
    script_ms_p50: percentile(sortedCallbacks, 0.5),
    script_ms_p95: percentile(sortedCallbacks, 0.95),
  };
}

// Runs drawFrame on every animation frame: a warm-up, then `measureMilliseconds` of measured frames.
async function measureScene(gl, drawFrame, measureMilliseconds) {
  const intervals = [];
  const callbackTimes = [];
  const start = await nextFrame();
  let previous = start;
  let measureStart = null;
  while (true) {
    const timestamp = await nextFrame();
    const callbackStart = performance.now();
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(...BACKGROUND);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    drawFrame(timestamp / 1000);
    const callbackTime = performance.now() - callbackStart;

    if (measureStart === null && timestamp - start >= WARMUP_MILLISECONDS) measureStart = timestamp;
    if (measureStart !== null) {
      intervals.push(timestamp - previous);
      callbackTimes.push(callbackTime);
      if (timestamp - measureStart >= measureMilliseconds) {
        return { ...summarize(intervals, callbackTimes, timestamp - measureStart), drawn_pixel_fraction: drawnPixelFraction(gl) };
      }
    }
    previous = timestamp;
  }
}

async function probeRefreshRate() {
  const intervals = [];
  let previous = await nextFrame();
  const start = previous;
  while (previous - start < IDLE_REFRESH_PROBE_MILLISECONDS) {
    const timestamp = await nextFrame();
    intervals.push(timestamp - previous);
    previous = timestamp;
  }
  intervals.sort((a, b) => a - b);
  return 1000 / percentile(intervals, 0.5);
}

function fullViewport() {
  return { width: canvas.width, height: canvas.height };
}

function drawSideBySide(gl, graphScene, flowScene, timeSeconds) {
  const half = Math.floor(canvas.width / 2);
  gl.enable(gl.SCISSOR_TEST);
  gl.viewport(0, 0, half, canvas.height);
  gl.scissor(0, 0, half, canvas.height);
  graphScene.draw(timeSeconds, { width: half, height: canvas.height });
  gl.viewport(half, 0, canvas.width - half, canvas.height);
  gl.scissor(half, 0, canvas.width - half, canvas.height);
  flowScene.draw(timeSeconds, { width: canvas.width - half, height: canvas.height });
  gl.disable(gl.SCISSOR_TEST);
}

function rendererInfo(gl) {
  const debugInfo = gl.getExtension("WEBGL_debug_renderer_info");
  return {
    vendor: debugInfo ? gl.getParameter(debugInfo.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR),
    renderer: debugInfo ? gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
    version: gl.getParameter(gl.VERSION),
  };
}

function scenePlan() {
  const plan = GRAPH_NODE_COUNTS.map((count) => ({ scene: "graph", label: `graph ${count} nodes`, nodes: count }));
  plan.push(...FLOW_SHAPES.map(([layers, tokens]) => ({ scene: "flow", label: `flow ${layers}×${tokens}`, layers, tokens })));
  plan.push({ scene: "combined", label: `combined ${COMBINED_SCENE.nodes} nodes + flow ${COMBINED_SCENE.layers}×${COMBINED_SCENE.tokens}`, ...COMBINED_SCENE });
  return plan;
}

function buildScene(gl, step) {
  if (step.scene === "graph") {
    const graph = createGraphScene(gl, step.nodes);
    return { draw: (time) => graph.draw(time, fullViewport()), counts: graph.counts };
  }
  if (step.scene === "flow") {
    const flow = createFlowScene(gl, step.layers, step.tokens);
    return { draw: (time) => flow.draw(time, fullViewport()), counts: flow.counts };
  }
  const graph = createGraphScene(gl, step.nodes);
  const flow = createFlowScene(gl, step.layers, step.tokens);
  return { draw: (time) => drawSideBySide(gl, graph, flow, time), counts: { ...graph.counts, ...flow.counts } };
}

async function measureHeadroom(gl, refreshRate) {
  const scene = buildScene(gl, { scene: "combined", ...COMBINED_SCENE });
  const rows = [];
  for (const repeats of HEADROOM_REPEATS) {
    statusLine.textContent = `headroom: combined scene drawn ${repeats}× per frame`;
    const drawRepeated = (time) => { for (let copy = 0; copy < repeats; copy++) scene.draw(time); };
    const summary = await measureScene(gl, drawRepeated, HEADROOM_MEASURE_MILLISECONDS);
    rows.push({ repeats, fps: summary.fps, frame_ms_p95: summary.frame_ms_p95 });
    if (summary.fps < refreshRate * AT_CAP_FRACTION) break;
  }
  const atCap = rows.filter((row) => row.fps >= refreshRate * AT_CAP_FRACTION).map((row) => row.repeats);
  return { rows, largest_repeat_at_cap: atCap.length ? Math.max(...atCap) : 0 };
}

async function postResults(report) {
  await fetch(RESULTS_URL, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(report) });
}

async function runPreview(gl, parameters) {
  const scene = parameters.get("preview");
  const count = Number(parameters.get("count") || 1000);
  const step = scene === "graph" ? { scene, nodes: count }
    : scene === "flow" ? { scene, layers: 26, tokens: count }
    : { scene: "combined", ...COMBINED_SCENE };
  const built = buildScene(gl, step);
  statusLine.textContent = `preview: ${scene} ${JSON.stringify(built.counts)}`;
  for (;;) {
    const timestamp = await nextFrame();
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(...BACKGROUND);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    built.draw(timestamp / 1000);
  }
}

async function runMeasurements(gl, parameters) {
  const measureMilliseconds = Number(parameters.get("measure") || DEFAULT_MEASURE_MILLISECONDS);
  const report = {
    user_agent: navigator.userAgent,
    device_pixel_ratio: window.devicePixelRatio,
    canvas_size: [canvas.width, canvas.height],
    ...rendererInfo(gl),
    refresh_rate_hz: await probeRefreshRate(),
    measure_milliseconds: measureMilliseconds,
    results: [],
  };
  for (const step of scenePlan()) {
    statusLine.textContent = `measuring: ${step.label}`;
    const scene = buildScene(gl, step);
    const summary = await measureScene(gl, scene.draw, measureMilliseconds);
    report.results.push({ ...step, ...scene.counts, ...summary });
  }
  report.headroom = await measureHeadroom(gl, report.refresh_rate_hz);
  statusLine.textContent = "done";
  await postResults(report);
}

async function main() {
  resizeCanvas();
  const parameters = new URLSearchParams(window.location.search);
  const gl = canvas.getContext("webgl2", { antialias: true, alpha: false, powerPreference: "high-performance" });
  if (!gl) {
    await postResults({ error: "WebGL2 is not available", user_agent: navigator.userAgent });
    statusLine.textContent = "WebGL2 is not available";
    return;
  }
  if (parameters.has("preview")) return runPreview(gl, parameters);
  return runMeasurements(gl, parameters);
}

main().catch(async (error) => {
  statusLine.textContent = "error: " + error.message;
  try { await postResults({ error: String(error && error.stack || error), user_agent: navigator.userAgent }); } catch (_) {}
});
