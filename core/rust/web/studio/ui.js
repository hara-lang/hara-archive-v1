import { HtaKeyword, HtaSymbol } from "../packages/hta/index.js";
import { defaultBootstrap } from "./boot.js";
import { editorFormAt, editorSourceComplete, editorTopLevelForms, isAnonymousDocument, studioDocumentId } from "./editor-state.js";
import { activateStudioDocument } from "./document-runtime.js";
import { createStudioShell } from "../ui/studio-shell.js";

/**
 * Shared, framework-free studio UI. `mountStudio(root, { broker })` builds
 * the whole studio DOM inside `root` (no dependency on surrounding page
 * markup), so the same mounting code serves the mkdocs website page now and
 * the hara-chrome panel later. Styling comes from the `.hara-studio-*`
 * classes (rust/web/studio/studio.css); hosts provide the stylesheet.
 *
 * All hara interaction goes through `broker.eval` — the UI never touches
 * IndexedDB or the worker directly. Panes: file tree, writable editor
 * (explicit save, no autosave), REPL log + input, status strip. Switchers:
 * space (list/switch/new/import-from-GitHub) and kernel
 * (list/switch/new/close; kernels stay alive in the broker when switching).
 */

const PROMPT = "hara ›";
const INSTA_STORAGE_KEY = "hara-studio.insta-enabled.v1";
const INSTA_DELAY_MS = 400;

export function traceField(value, name) {
  if (!(value instanceof Map)) return undefined;
  for (const [key, item] of value) {
    if ((key?.name ?? key) === name) return item;
  }
  return undefined;
}

export function traceName(value) {
  return String(value?.name ?? value ?? "");
}

/** Project ordered trace events into parent-linked operation nodes while
 * preserving macro/error diagnostics as standalone rows. */
export function buildTraceTree(trace) {
  const roots = [];
  const operations = new Map();
  for (const event of traceField(trace, "events") ?? []) {
    const kind = traceName(traceField(event, "kind"));
    const operation = traceField(event, "operation");
    if (kind === "operation-enter" && operation != null) {
      const node = { event, returnEvent: null, children: [] };
      operations.set(String(operation), node);
      const parent = traceField(event, "parent-operation");
      const parentNode = parent == null ? null : operations.get(String(parent));
      (parentNode?.children ?? roots).push(node);
    } else if (kind === "operation-return" && operation != null) {
      const node = operations.get(String(operation));
      if (node) node.returnEvent = event;
      else roots.push({ event, returnEvent: null, children: [] });
    } else {
      const node = { event, returnEvent: null, children: [] };
      const parent = traceField(event, "parent-operation");
      const parentNode = parent == null ? null : operations.get(String(parent));
      (parentNode?.children ?? roots).push(node);
    }
  }
  return roots;
}

/** Render a decoded HTA value as a display string (same approach as
 *  extensions/hara-chrome/src/resp-client.js `renderHta`, plus symbols). */
export function renderValue(value) {
  if (value === null || value === undefined) return "nil";
  if (value instanceof HtaKeyword) return `:${value.name}`;
  if (value instanceof HtaSymbol) return value.name;
  if (value instanceof Map) {
    return `{${[...value].map(([k, v]) => `${renderValue(k)} ${renderValue(v)}`).join(", ")}}`;
  }
  if (value instanceof Set) return `#{${[...value].map(renderValue).join(" ")}}`;
  if (Array.isArray(value)) return `[${value.map(renderValue).join(" ")}]`;
  if (typeof value === "string") return value;
  return String(value);
}

/** Normalize a user-supplied file path to the attached filesystem shape ("/a/b.hal").
 *  Returns null for empty, root-only, or parent-escaping paths. */
export function normalizePath(input) {
  if (typeof input !== "string") return null;
  let path = input.trim();
  if (!path) return null;
  if (!path.startsWith("/")) path = `/${path}`;
  path = path.replace(/\/{2,}/g, "/");
  if (path.length > 1 && path.endsWith("/")) path = path.slice(0, -1);
  if (path === "/") return null;
  if (path.split("/").includes("..")) return null;
  return path;
}

/** New source files default to HAL while manifest and explicitly-typed files
 * keep their extension. */
export function normalizeNewFilePath(input) {
  const path = normalizePath(input);
  if (!path) return null;
  const leaf = path.slice(path.lastIndexOf("/") + 1);
  return leaf.includes(".") ? path : `${path}.hal`;
}

/** Build a nested tree (directories first, alphabetical) from a flat list
 *  of file paths. Nodes: { name, path, directory, children? }. */
export function buildTree(paths) {
  const root = { name: "/", path: "/", directory: true, children: [] };
  const directories = new Map([["/", root]]);
  for (const raw of paths ?? []) {
    const path = normalizePath(raw);
    if (!path) continue;
    const segments = path.slice(1).split("/");
    let current = root;
    let currentPath = "";
    for (let index = 0; index < segments.length; index++) {
      currentPath += `/${segments[index]}`;
      if (index === segments.length - 1) {
        current.children.push({ name: segments[index], path: currentPath, directory: false });
      } else {
        let directory = directories.get(currentPath);
        if (!directory) {
          directory = { name: segments[index], path: currentPath, directory: true, children: [] };
          directories.set(currentPath, directory);
          current.children.push(directory);
        }
        current = directory;
      }
    }
  }
  const sort = (node) => {
    node.children.sort((a, b) =>
      a.directory === b.directory ? a.name.localeCompare(b.name) : a.directory ? -1 : 1
    );
    for (const child of node.children) if (child.directory) sort(child);
  };
  sort(root);
  return root.children;
}

/** Parse "owner/repo[@ref]" into { repo, ref, space }; null when malformed.
 *  The imported space takes the repo's bare name. */
export function parseGithubSpec(input) {
  if (typeof input !== "string") return null;
  const match = input.trim().match(/^([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)(?:@([A-Za-z0-9_./-]+))?$/);
  if (!match) return null;
  const repo = match[1];
  return { repo, ref: match[2] ?? "main", space: repo.split("/").pop() };
}

/** Wrap an internal studio form with the requires it needs. Raw wasm
 *  require vectors are UNQUOTED — keep them that way. */
export function studioSource(form) {
  return `(do (require [studio.boot :as boot]) ${form})`;
}

/** Seed content for a newly created file. */
export function defaultFileContent(path) {
  return `;; ${path}\n\n(ns user)\n`;
}

/**
 * Mount the studio into `root`. Options: { broker } — a KernelBroker whose
 * kernels already have the studio.* hal resources registered. Returns a
 * controller: { shell, state, submitRepl, refresh, unmount }.
 */
export function mountStudio(root, {
  broker,
  projects = [],
  runtimeVersion = "development",
  canvasRuntime = null
} = {}) {
  if (!root) throw new Error("mountStudio requires a root element");
  if (!broker) throw new Error("mountStudio requires a broker");
  return new StudioController(root, broker, { projects, runtimeVersion, canvasRuntime });
}

class StudioController {
  constructor(root, broker, { projects, runtimeVersion, canvasRuntime }) {
    this.root = root;
    this.broker = broker;
    this.projects = projects;
    this.runtimeVersion = runtimeVersion;
    this.canvasRuntime = canvasRuntime;
    this.spaces = new Set(projects.map((project) => this.projectSpace(project)));
    this.mounts = new WeakMap();
    this.state = {
      kernel: "ROOT",
      space: null,
      files: [],
      open: null,
      dirty: false,
      busy: 0,
      runtime: "BOOTING",
      failed: false,
      instaEnabled: safeStorageGet(INSTA_STORAGE_KEY) === "true",
      previewGeneration: 0,
      preview: null
    };
    this.buildDom();
    this.instaAction.classList.toggle("is-active", this.state.instaEnabled);
    this.bindEvents();
    this.init();
  }

  // -------------------------------------------------------------- dom build

  buildDom() {
    this.chrome = createStudioShell(this.root);
    const shell = this.chrome.shell;
    const main = this.chrome.main;

    // Runtime selectors remain controller state. Operational details and
    // kernel actions are exposed through the compact health popover.
    this.spaceSelect = el("select", "hara-studio-select");
    this.spaceSelect.setAttribute("data-hara-studio", "space-select");
    this.spaceSelect.setAttribute("aria-label", "Active space");
    this.kernelSelect = el("select", "hara-studio-select");
    this.kernelSelect.setAttribute("data-hara-studio", "kernel-select");
    this.kernelSelect.setAttribute("aria-label", "Active kernel");
    this.newFileAction = this.chrome.fileNewButton;

    // File tree.
    const tree = el("aside", "hara-frame hara-studio-tree");
    tree.setAttribute("data-hara-studio", "file-tree");
    this.treeCount = el("span", "hara-index", "0");
    this.treeSpace = el("span", null, "EXPLORER");
    const treeHead = el("div", "hara-studio-pane-head");
    treeHead.append(this.treeSpace, this.treeCount);
    this.treeBody = el("div", "hara-studio-tree-body");
    tree.append(treeHead, this.treeBody);

    // Editor (writable; explicit save).
    const editorWrap = el("section", "hara-frame hara-studio-editor-wrap");
    this.editorName = el("span", null, "—");
    this.editorName.setAttribute("data-hara-studio", "editor-name");
    this.dirtyFlag = el("span", "hara-studio-dirty", "");
    this.saveAction = action("◆", "Save file to the active project");
    this.saveAction.classList.add("hara-studio-pane-icon");
    this.saveAction.setAttribute("aria-label", "Save file");
    this.runAction = action("▶", "Evaluate the whole file");
    this.runAction.classList.add("hara-studio-pane-icon");
    this.runAction.setAttribute("aria-label", "Run file");
    this.traceAction = action("≡", "Trace the file in the active evaluator session");
    this.traceAction.classList.add("hara-studio-pane-icon");
    this.traceAction.setAttribute("aria-label", "Trace file");
    this.instaAction = action("INSTA", "Toggle live isolated evaluation");
    this.instaAction.classList.add("hara-studio-pane-icon", "hara-studio-insta-toggle");
    this.instaAction.setAttribute("aria-label", "Toggle InstaREPL");
    this.instaAction.setAttribute("aria-pressed", String(this.state.instaEnabled));
    const editorHead = el("div", "hara-studio-pane-head");
    const editorHeadRight = el("span");
    editorHeadRight.append(this.dirtyFlag, this.instaAction, this.traceAction, this.runAction, this.saveAction);
    editorHead.append(this.editorName, editorHeadRight);
    this.editor = el("textarea", "hara-studio-editor");
    this.editor.setAttribute("data-hara-studio", "editor");
    this.editor.setAttribute("spellcheck", "false");
    this.editor.setAttribute("wrap", "off");
    this.editor.setAttribute("aria-label", "File editor");
    this.editor.disabled = true;
    this.tracePanel = el("section", "hara-studio-trace");
    this.tracePanel.setAttribute("data-hara-studio", "form-trace");
    this.tracePanel.hidden = true;
    this.tracePanel.append(el("div", "hara-studio-trace-head", "FORM TRACE"));
    this.traceBody = el("div", "hara-studio-trace-body");
    this.tracePanel.append(this.traceBody);
    this.editorStage = el("div", "hara-studio-editor-stage");
    this.instaGutter = el("aside", "hara-studio-insta-gutter");
    this.instaGutter.setAttribute("data-hara-studio", "insta-gutter");
    this.instaGutter.setAttribute("aria-label", "InstaREPL results");
    this.editorStage.append(this.editor, this.instaGutter);
    editorWrap.append(editorHead, this.editorStage, this.tracePanel);

    // REPL.
    const repl = el("section", "hara-frame hara-studio-repl");
    const replHead = el("div", "hara-studio-pane-head");
    this.replKernel = el("span", "hara-index", "TTY/01 · ROOT");
    replHead.append(el("span", null, "CONSOLE"), this.replKernel);
    this.replLog = el("div", "hara-studio-repl-log");
    this.replLog.setAttribute("data-hara-studio", "repl-log");
    const entry = el("div", "hara-studio-repl-entry");
    this.promptLabel = el("span", "hara-tty-p", PROMPT);
    this.input = el("input", "hara-studio-repl-input");
    this.input.setAttribute("data-hara-studio", "repl-input");
    this.input.setAttribute("type", "text");
    this.input.setAttribute("placeholder", "(your first form)");
    this.input.setAttribute("autocomplete", "off");
    this.input.setAttribute("spellcheck", "false");
    this.input.setAttribute("aria-label", "REPL input");
    entry.append(this.promptLabel, this.input);
    repl.append(replHead, this.replLog, entry);

    this.main = main;
    main.append(tree, editorWrap, repl);
    this.canvasPanel = el("section", "hara-frame hara-studio-canvas-panel");
    this.canvasPanel.hidden = true;
    this.canvas = el("canvas", "hara-studio-canvas");
    this.canvas.setAttribute("data-hara-studio", "canvas");
    this.canvasPanel.append(
      el("div", "hara-studio-pane-head", "LIVE CANVAS · HAL OWNED"),
      this.canvas
    );
    this.ampFrame = el("iframe", "hara-studio-amp");
    this.ampFrame.title = "Hara Amp live workspace";
    this.ampFrame.hidden = true;
    this.ampFrame.setAttribute("loading", "lazy");
    this.canvasPanel.appendChild(this.ampFrame);
    main.appendChild(this.canvasPanel);
    if (this.canvasRuntime) {
      this.canvasRuntime.register("canvas/background", this.canvas);
      this.canvasRuntime.register("canvas/visualizer", this.canvas);
    }

    this.projectChooser = el("section", "hara-studio-chooser");
    this.projectChooser.setAttribute("data-hara-studio", "project-chooser");
    this.projectChooser.hidden = true;
    shell.insertBefore(this.projectChooser, main);
    this.shell = shell;
    this.buildDialog();
  }

  buildDialog() {
    this.dialog = el("div", "hara-studio-dialog");
    this.dialog.hidden = true;
    this.dialog.setAttribute("role", "dialog");
    this.dialog.setAttribute("aria-modal", "true");
    this.dialogTitle = el("h2", null, "");
    this.dialogLabel = el("label", null, "");
    this.dialogInput = el("input", "hara-studio-dialog-input");
    this.dialogInput.setAttribute("data-hara-studio", "dialog-input");
    this.dialogLabel.appendChild(this.dialogInput);
    this.dialogError = el("p", "hara-studio-dialog-error", "");
    this.dialogCancel = action("CANCEL");
    this.dialogAccept = action("CONTINUE");
    const actions = el("div", "hara-studio-dialog-actions");
    actions.append(this.dialogCancel, this.dialogAccept);
    this.dialog.append(this.dialogTitle, this.dialogLabel, this.dialogError, actions);
    this.shell.appendChild(this.dialog);
  }

  bindEvents() {
    this.spaceSelect.addEventListener("change", () => this.switchSpace(this.spaceSelect.value));
    this.kernelSelect.addEventListener("change", () => this.switchKernel(this.kernelSelect.value));
    this.shell.addEventListener("hara:studio-action", (event) => {
      const handlers = {
        "project/select": () => this.showProjectChooser(),
        "File/new": () => this.newFile(),
        "project/import": () => this.importGithub(),
        "console/toggle": () => this.toggleConsole(),
        "view/explorer": () => this.setMobileView("explorer"),
        "view/source": () => this.setMobileView("source"),
        "view/output": () => this.setMobileView("output"),
        "view/console": () => this.setMobileView("console"),
        "kernel/new": () => this.newKernel(),
        "kernel/close": () => this.closeKernel()
      };
      handlers[event.detail?.action]?.();
    });
    this.saveAction.addEventListener("click", () => this.saveFile());
    this.runAction.addEventListener("click", () => this.runFile());
    this.traceAction.addEventListener("click", () => this.traceFile());
    this.instaAction.addEventListener("click", () => this.toggleInsta());
    this.editor.addEventListener("input", () => {
      this.state.dirty = true;
      this.clearTrace();
      this.markInstaStale();
      this.scheduleInsta();
      this.renderEditorHead();
    });
    this.editor.addEventListener("scroll", () => {
      this.instaGutter.scrollTop = this.editor.scrollTop;
    });
    this.editor.addEventListener("keydown", (event) => {
      if ((event.ctrlKey || event.metaKey) && event.key === "s") {
        event.preventDefault();
        this.saveFile();
      } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "e") {
        event.preventDefault();
        this.runForm();
      } else if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
        event.preventDefault();
        this.runFile();
      }
    });
    this.input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        this.submitRepl(this.input.value);
        this.input.value = "";
      }
    });
  }

  toggleConsole() {
    const open = this.main.classList.toggle("is-console-open");
    if (open) this.setMobileView("console", { render: false });
    this.renderStatus();
    if (open) this.input.focus();
  }

  setMobileView(view, { render = true } = {}) {
    for (const name of ["explorer", "source", "output", "console"]) this.main.classList.remove(`mobile-view-${name}`);
    this.main.classList.add(`mobile-view-${view}`);
    this.state.mobileView = view;
    if (render) this.renderStatus();
  }

  async showProjectChooser() {
    if (!(await this.confirmDiscard())) return;
    this.renderProjectChooser(await this.listSpaces());
  }

  // ------------------------------------------------------------------- init

  async init() {
    try {
      this.refreshKernelSelect();
      let spaces = await this.listSpaces();
      if (spaces.length === 0) {
        await this.attachSpace("home");
        await this.broker.eval(this.state.kernel, defaultBootstrap("home"));
        spaces = ["home"];
      }
      if (this.projects.length > 0) {
        this.renderProjectChooser(spaces);
        this.state.runtime = "LIVE";
        this.logNote(";; choose a local browser project to begin");
        this.renderStatus();
        return;
      }
      this.state.space = spaces[0];
      this.renderSpaceSelect(spaces);
      await this.refreshFiles();
      this.state.runtime = "LIVE";
      this.logNote(`;; hara studio — wasm runtime live, kernel ${this.state.kernel}, space ${this.state.space}`);
    } catch (error) {
      this.state.runtime = "ERROR";
      this.state.failed = true;
      this.logError(error);
      this.logNote(";; boot failed — check the console and reload");
    }
    this.renderStatus();
  }

  projectSpace(project) {
    return `project-${project.id}-${this.runtimeVersion}`.replace(/[^A-Za-z0-9_.-]/g, "-");
  }

  renderProjectChooser(spaces = []) {
    this.projectChooser.replaceChildren(
      el("p", "hara-kicker", "CHOOSE A LOCAL PROJECT"),
      el("h1", null, "Make something. Keep it live."),
      el("p", null, "Projects stay in this browser. Pick a complete workspace, edit it, save it, and return later.")
    );
    const cards = el("div", "hara-studio-projects");
    for (const project of this.projects) {
      const space = this.projectSpace(project);
      const recovered = spaces.includes(space);
      const card = el("article", "hara-studio-project");
      card.setAttribute("data-project", project.id);
      card.append(
        el("span", "hara-index", recovered ? "CONTINUE LOCAL PROJECT" : project.category.toUpperCase()),
        el("h2", null, project.title),
        el("p", null, project.description)
      );
      const open = action(recovered ? "CONTINUE" : "OPEN PROJECT");
      open.setAttribute("data-project-open", project.id);
      open.addEventListener("click", () => this.openProject(project, { reset: !recovered }));
      card.appendChild(open);
      if (recovered) {
        const reset = action("RESET");
        reset.setAttribute("data-project-reset", project.id);
        reset.addEventListener("click", async () => {
          if (await this.askConfirm(`Reset ${project.title}?`, "Only this project's local edits will be cleared.")) {
            await this.openProject(project, { reset: true });
          }
        });
        card.appendChild(reset);
      }
      cards.appendChild(card);
    }
    this.projectChooser.appendChild(cards);
    this.projectChooser.hidden = false;
    this.shell.classList.add("is-choosing-project");
    this.renderStatus();
  }

  async openProject(project, { reset = false } = {}) {
    const space = this.projectSpace(project);
    await this.attachSpace(space);
    await this.task(async () => {
      const existing = await this.listFilesRecursive("/");
      if (reset) {
        for (const path of existing ?? []) {
          await this.deletePath(String(path));
        }
        for (const [path, content] of Object.entries(project.files)) {
          await this.writeText(`/${path}`, content);
        }
      }
    });
    this.state.space = space;
    this.activeProject = project;
    this.chrome.setProject(project.title);
    this.projectChooser.hidden = true;
    this.shell.classList.remove("is-choosing-project");
    this.renderSpaceSelect(await this.listSpaces());
    await this.refreshFiles();
    const preferred = project.main ? `/${project.main}` : this.state.files.find((path) => path.endsWith(".hal"));
    if (preferred && this.state.files.includes(preferred)) await this.openFile(preferred);
    const ownsCanvas = project.capabilities.some((value) => value === "canvas/2d" || value === "audio/playback");
    this.main.classList.toggle("has-canvas", ownsCanvas);
    this.canvasPanel.hidden = !ownsCanvas;
    this.canvas.hidden = project.category === "audio";
    this.ampFrame.hidden = project.category !== "audio";
    if (project.category === "audio" && !this.ampFrame.src) {
      this.ampFrame.src = new URL("../../examples/music/hara-amp.html", import.meta.url).href;
    }
    if (ownsCanvas && project.category === "visual") await this.runFile();
    this.renderStatus();
    this.logNote(`;; project ${project.title} · manifests loaded · recovery local`);
  }

  askInput(title, label, value = "") {
    return this.showDialog({ title, label, value, confirm: false });
  }

  askConfirm(title, label) {
    return this.showDialog({ title, label, confirm: true });
  }

  showDialog({ title, label, value = "", confirm }) {
    this.dialogTitle.textContent = title;
    this.dialogLabel.firstChild.nodeValue = label;
    this.dialogInput.value = value;
    this.dialogInput.hidden = confirm;
    this.dialogError.textContent = "";
    this.dialog.hidden = false;
    if (!confirm) queueMicrotask(() => this.dialogInput.focus());
    return new Promise((resolve) => {
      const finish = (result) => {
        this.dialog.hidden = true;
        this.dialogCancel.onclick = null;
        this.dialogAccept.onclick = null;
        resolve(result);
      };
      this.dialogCancel.onclick = () => finish(confirm ? false : null);
      this.dialogAccept.onclick = () => finish(confirm ? true : this.dialogInput.value);
    });
  }

  // ------------------------------------------------------------------ evals

  // Eval a studio-lib form in the active kernel (requires wrapped in).
  evalStudio(form) {
    return this.broker.eval(this.state.kernel, studioSource(form));
  }

  readText(path) {
    return this.evalStudio(
      `(str/decode-utf8 (deref (File/read ${JSON.stringify(path)})))`
    );
  }

  async writeText(path, content) {
    const parent = path.slice(0, path.lastIndexOf("/")) || "/";
    if (parent !== "/") {
      await this.evalStudio(`(deref (File/mkdir ${JSON.stringify(parent)}))`);
    }
    return this.evalStudio(
      `(deref (File/write ${JSON.stringify(path)} (str/encode-utf8 ${JSON.stringify(content)})))`
    );
  }

  deletePath(path) {
    return this.evalStudio(`(deref (File/delete ${JSON.stringify(path)}))`);
  }

  async listFilesRecursive(path = "/") {
    const children = await this.evalStudio(`(deref (File/list ${JSON.stringify(path)}))`);
    const files = [];
    for (const child of children ?? []) {
      try {
        files.push(...await this.listFilesRecursive(String(child)));
      } catch {
        files.push(String(child));
      }
    }
    return files;
  }

  // Run an async operation, tracking busy/error state. Errors are logged to
  // the REPL and flip the status strip to ERROR until the next success.
  async task(operation) {
    this.state.busy += 1;
    this.renderStatus();
    try {
      const value = await operation();
      this.state.failed = false;
      return value;
    } catch (error) {
      this.state.failed = true;
      this.logError(error);
      return undefined;
    } finally {
      this.state.busy -= 1;
      this.renderStatus();
    }
  }

  // ------------------------------------------------------------------ repl

  async submitRepl(source) {
    source = (source ?? "").trim();
    if (!source) return;
    const form = el("div");
    form.append(el("span", "hara-tty-p", PROMPT), text(` ${source}`));
    this.replLog.appendChild(form);
    await this.task(async () => {
      const value = await this.broker.eval(this.state.kernel, source);
      this.logValue(value);
    });
    this.replLog.scrollTop = this.replLog.scrollHeight;
  }

  logValue(value) {
    this.replLog.appendChild(el("div", "hara-tty-v", `=> ${renderValue(value)}`));
    this.replLog.scrollTop = this.replLog.scrollHeight;
  }

  logNote(message) {
    this.replLog.appendChild(el("div", "hara-tty-o", message));
    this.replLog.scrollTop = this.replLog.scrollHeight;
  }

  logError(error) {
    this.replLog.appendChild(el("div", "hara-tty-e", `!! ${error?.message ?? error}`));
    this.replLog.scrollTop = this.replLog.scrollHeight;
  }

  // ----------------------------------------------------------------- spaces

  renderSpaceSelect(spaces) {
    this.spaceSelect.replaceChildren();
    for (const name of spaces) {
      const option = el("option", null, name);
      option.value = name;
      if (name === this.state.space) option.selected = true;
      this.spaceSelect.appendChild(option);
    }
    this.treeSpace.textContent = this.state.space ?? "—";
  }

  async listSpaces() {
    return [...this.spaces].sort();
  }

  async attachSpace(name, kernelName = this.state.kernel) {
    const kernel = await this.broker.require(kernelName);
    let mounts = this.mounts.get(kernel.context);
    if (!mounts) this.mounts.set(kernel.context, mounts = new Map());
    let mount = mounts.get(name);
    if (!mount) {
      mount = await kernel.context.createFilesystem({ provider: "indexeddb", key: name });
      mounts.set(name, mount);
    }
    await kernel.context.session().attachFilesystem(mount);
    this.spaces.add(name);
    return mount;
  }

  async switchSpace(name) {
    if (!name || name === this.state.space) return;
    if (!(await this.confirmDiscard())) {
      this.spaceSelect.value = this.state.space ?? "";
      return;
    }
    await this.attachSpace(name);
    await this.clearInsta();
    this.state.space = name;
    this.clearEditor();
    this.renderSpaceSelect(await this.task(() => this.listSpaces()) ?? [name]);
    await this.refreshFiles();
  }

  async newSpace() {
    const name = await this.askInput("New space", "Space name", "");
    if (!name || !name.trim()) return;
    const trimmed = name.trim();
    if (trimmed.includes("/")) {
      this.logError(new Error("space names cannot contain '/'"));
      return;
    }
    const booted = await this.task(async () => {
      await this.attachSpace(trimmed);
      return this.evalStudio(`(boot/boot! ${JSON.stringify(trimmed)})`);
    });
    if (booted === undefined) return;
    this.state.space = trimmed;
    this.clearEditor();
    this.renderSpaceSelect(await this.task(() => this.listSpaces()) ?? [trimmed]);
    await this.refreshFiles();
    this.logNote(`;; space ${trimmed} ready`);
  }

  // GitHub project discovery and fetching are host responsibilities. File
  // bytes still cross the host-mounted native File boundary.
  async importGithub() {
    const spec = await this.askInput("Import from GitHub", "owner/repo[@ref]", "");
    if (!spec || !spec.trim()) return;
    const parsed = parseGithubSpec(spec);
    if (!parsed) {
      this.logError(new Error(`invalid GitHub spec: ${spec.trim()} (expected owner/repo[@ref])`));
      return;
    }
    this.logNote(`;; importing ${parsed.repo}@${parsed.ref} into space ${parsed.space} …`);
    const summary = await this.task(async () => {
      await this.attachSpace(parsed.space);
      const listingResponse = await fetch(`https://data.jsdelivr.com/v1/packages/gh/${parsed.repo}@${parsed.ref}`);
      if (!listingResponse.ok) throw new Error(`GitHub listing failed: ${listingResponse.status}`);
      const listing = await listingResponse.json();
      const paths = [];
      const collect = (entries) => {
        for (const entry of entries ?? []) {
          if (entry.type === "directory") collect(entry.files);
          else paths.push(entry.name);
        }
      };
      collect(listing.files);
      for (const path of paths) {
        const response = await fetch(`https://cdn.jsdelivr.net/gh/${parsed.repo}@${parsed.ref}${path}`);
        if (!response.ok) throw new Error(`GitHub file fetch failed: ${path}`);
        await this.writeText(path, await response.text());
      }
      return new Map([["project", parsed.space], ["repo", parsed.repo], ["ref", parsed.ref], ["imported", paths.length]]);
    });
    if (summary === undefined) return;
    this.state.space = parsed.space;
    this.clearEditor();
    this.renderSpaceSelect(await this.task(() => this.listSpaces()) ?? [parsed.space]);
    await this.refreshFiles();
    this.logValue(summary);
  }

  // ---------------------------------------------------------------- kernels

  refreshKernelSelect() {
    const names = this.broker.list();
    this.kernelSelect.replaceChildren();
    for (const name of names) {
      const option = el("option", null, name);
      option.value = name;
      if (name === this.state.kernel) option.selected = true;
      this.kernelSelect.appendChild(option);
    }
    this.replKernel.textContent = `TTY/01 · ${this.state.kernel}`;
  }

  // Switching kernels never closes them — everything stays alive in the
  // broker, so nothing is lost.
  async switchKernel(name) {
    if (!name || name === this.state.kernel) return;
    await this.clearInsta();
    this.state.kernel = name;
    if (this.state.space) await this.attachSpace(this.state.space, name);
    this.refreshKernelSelect();
    this.renderStatus();
    this.logNote(`;; active kernel ${name}`);
  }

  async newKernel() {
    const name = await this.askInput("New kernel", "Kernel name (A-Za-z0-9_.-)", "");
    if (!name || !name.trim()) return;
    const trimmed = name.trim();
    const fallback = defaultBootstrap(this.state.space ?? "home");
    const custom = await this.askInput("Kernel bootstrap", "Source (empty uses the active space)", "");
    const bootstrap = custom && custom.trim() ? custom.trim() : fallback;
    const kernel = await this.task(() => this.broker.create(trimmed, { bootstrap }));
    if (kernel === undefined) {
      this.refreshKernelSelect();
      return;
    }
    this.state.kernel = trimmed;
    if (this.state.space) await this.attachSpace(this.state.space, trimmed);
    this.refreshKernelSelect();
    this.renderStatus();
    this.logNote(`;; kernel ${trimmed} booted`);
  }

  async closeKernel() {
    const name = this.state.kernel;
    if (name === "ROOT") {
      this.logError(new Error("ROOT_CANNOT_CLOSE"));
      return;
    }
    if (!(await this.askConfirm(`Close kernel ${name}?`, "Its in-memory state will be lost."))) return;
    // broker.close resolves undefined on success too, so confirm the close
    // by checking the kernel is actually gone before switching back.
    await this.task(() => this.broker.close(name));
    if (this.broker.list().includes(name)) return;
    this.state.kernel = "ROOT";
    this.refreshKernelSelect();
    this.renderStatus();
    this.logNote(`;; kernel ${name} closed`);
  }

  // ------------------------------------------------------------------ files

  async refreshFiles() {
    const files = await this.task(() => this.listFilesRecursive("/"));
    this.state.files = (Array.isArray(files) ? files.map(String) : []).sort();
    this.renderTree();
    this.renderStatus();
  }

  renderTree() {
    this.treeBody.replaceChildren();
    if (this.state.files.length === 0) {
      this.treeBody.appendChild(el("div", "hara-studio-tree-group", "EMPTY — NEW FILE TO START"));
      return;
    }
    const renderNodes = (nodes) => {
      for (const node of nodes) {
        if (node.directory) {
          this.treeBody.appendChild(el("div", "hara-studio-tree-group", node.path.slice(1).toUpperCase()));
          renderNodes(node.children);
        } else {
          const row = el("div", "hara-studio-file");
          row.setAttribute("data-file", node.path);
          if (node.path === this.state.open) row.classList.add("is-active");
          row.append(el("span", "hara-index", "◇"), text(node.name));
          row.addEventListener("click", () => this.openFile(node.path));
          this.treeBody.appendChild(row);
        }
      }
    };
    renderNodes(buildTree(this.state.files));
  }

  async openFile(path) {
    if (path === this.state.open && !this.state.dirty) return;
    if (!(await this.confirmDiscard())) return;
    const content = await this.task(() => this.readText(path));
    if (content === undefined) return;
    await this.clearInsta();
    this.state.open = path;
    this.state.dirty = false;
    this.editor.value = content === null ? "" : String(content);
    this.editor.disabled = false;
    this.clearTrace();
    this.renderEditorHead();
    this.renderTree();
    this.scheduleInsta({ immediate: true });
  }

  async saveFile() {
    if (!this.state.open) return;
    const path = this.state.open;
    const content = this.editor.value;
    const ok = await this.task(() => this.writeText(path, content));
    if (ok === undefined) return;
    this.state.dirty = false;
    this.renderEditorHead();
    this.logNote(`;; saved ${path}`);
  }

  async runFile() {
    if (!this.state.open) return;
    const source = this.editor.value;
    this.cancelInstaTimer();
    this.state.previewGeneration += 1;
    await this.task(async () => {
      if (isAnonymousDocument(source)) {
        const documentId = this.activeDocumentId();
        const nodeId = `node/${this.activeProject?.id ?? "document"}`;
        const canvasId = this.activeProject?.category === "audio"
          ? "canvas/visualizer"
          : "canvas/background";
        const ownsCanvas = this.activeProject?.capabilities?.some(
          (value) => value === "canvas/2d" || value === "audio/playback"
        ) ?? false;
        const result = await activateStudioDocument({
          broker: this.broker,
          kernel: this.state.kernel,
          documentId,
          source,
          nodeId,
          canvasRuntime: this.canvasRuntime,
          canvasId,
          requireFirstFrame: ownsCanvas,
          onTaskError: (error) => this.logError(error)
        });
        this.logNote(`;; activated ${this.state.open} generation ${result.generation}`);
        return result;
      }
      const value = await this.broker.eval(this.state.kernel, source);
      this.logValue(value);
      return value;
    });
  }

  async traceFile() {
    if (!this.state.open) return;
    const source = this.editor.value;
    if (isAnonymousDocument(source)) {
      this.showTraceMessage("Live ns+ documents use generated runtime forms and cannot be source-traced yet.");
      return;
    }
    if (!source.trim()) return this.showTraceMessage("No source to trace.");
    this.cancelInstaTimer();
    this.state.previewGeneration += 1;
    const trace = await this.task(() =>
      this.broker.traceEval(this.state.kernel, "ROOT", source)
    );
    if (trace) this.renderTrace(trace);
    else this.showTraceMessage("Structured tracing is unavailable in this runtime build.");
  }

  clearTrace() {
    this.tracePanel.hidden = true;
    this.traceBody.replaceChildren();
  }

  showTraceMessage(message) {
    this.tracePanel.hidden = false;
    this.traceBody.replaceChildren(el("div", "hara-studio-trace-note", message));
  }

  renderTrace(trace, sourceRange = null) {
    const status = traceName(traceField(trace, "status")) || "unknown";
    const events = traceField(trace, "events") ?? [];
    const result = traceField(trace, "result");
    const summary = el("div", `hara-studio-trace-summary is-${status}`);
    summary.append(
      el("strong", null, status.toUpperCase()),
      el("span", null, `${events.length} EVENTS`),
      el("span", "hara-studio-trace-result",
        traceField(result, "display") ?? traceField(trace, "error") ?? "nil")
    );
    const filters = el("div", "hara-studio-trace-filters");
    const tree = el("div", "hara-studio-trace-tree");
    const details = el("pre", "hara-studio-trace-details", "Select an event to inspect it.");
    let activeKinds = new Set(["operation", "macro", "error"]);
    const showEvent = (event) => {
      details.textContent = formatTraceEvent(event);
      const start = traceField(event, "source-start");
      const end = traceField(event, "source-end");
      if (Number.isInteger(start) && Number.isInteger(end)) {
        this.editor.focus();
        this.editor.setSelectionRange(start, end);
      } else if (sourceRange) {
        this.editor.focus();
        this.editor.setSelectionRange(sourceRange.start, sourceRange.end);
      }
    };
    const renderTree = () => {
      tree.replaceChildren();
      const renderNode = (node, parent, depth = 0) => {
        const kind = traceName(traceField(node.event, "kind"));
        const group = traceKindGroup(kind);
        if (kind !== "evaluation-start" && activeKinds.has(group)) {
          const row = action(traceEventLabel(node), "Inspect trace event");
          row.classList.add("hara-studio-trace-node", `is-${group}`);
          row.style.setProperty("--trace-depth", String(depth));
          row.addEventListener("click", () => showEvent(node.event));
          parent.appendChild(row);
        }
        for (const child of node.children) renderNode(child, parent, depth + 1);
      };
      for (const node of buildTraceTree(trace)) renderNode(node, tree);
      if (!tree.childNodes.length) tree.append(el("div", "hara-studio-trace-note", "No events match these filters."));
    };
    for (const [name, label] of [["operation", "CALLS"], ["macro", "MACROS"], ["error", "ERRORS"]]) {
      const button = action(label);
      button.classList.add("hara-studio-trace-filter", "is-active");
      button.setAttribute("aria-pressed", "true");
      button.addEventListener("click", () => {
        if (activeKinds.has(name)) activeKinds.delete(name); else activeKinds.add(name);
        button.classList.toggle("is-active", activeKinds.has(name));
        button.setAttribute("aria-pressed", String(activeKinds.has(name)));
        renderTree();
      });
      filters.appendChild(button);
    }
    const raw = action("RAW TRACE");
    raw.classList.add("hara-studio-trace-raw");
    raw.addEventListener("click", () => this.exportTrace(trace));
    filters.appendChild(raw);
    renderTree();
    const layout = el("div", "hara-studio-trace-layout");
    layout.append(tree, details);
    this.tracePanel.hidden = false;
    this.traceBody.replaceChildren(summary, filters, layout);
  }

  async exportTrace(trace) {
    const json = JSON.stringify(traceToPlain(trace), null, 2);
    try {
      await navigator.clipboard.writeText(json);
      this.logNote(";; trace JSON copied");
    } catch {
      const blob = new Blob([json], { type: "application/json" });
      const link = document.createElement("a");
      link.href = URL.createObjectURL(blob);
      link.download = `${traceField(trace, "trace-id") ?? "hara-trace"}.json`;
      link.click();
      URL.revokeObjectURL(link.href);
    }
  }

  toggleInsta() {
    this.state.instaEnabled = !this.state.instaEnabled;
    safeStorageSet(INSTA_STORAGE_KEY, String(this.state.instaEnabled));
    this.instaAction.setAttribute("aria-pressed", String(this.state.instaEnabled));
    this.instaAction.classList.toggle("is-active", this.state.instaEnabled);
    if (this.state.instaEnabled) this.scheduleInsta({ immediate: true });
    else this.clearInsta();
  }

  instaAllowed() {
    if (!this.state.open || isAnonymousDocument(this.editor.value)) return false;
    return !(this.activeProject?.capabilities ?? []).some(
      (value) => value === "canvas/2d" || value === "audio/playback"
    );
  }

  scheduleInsta({ immediate = false } = {}) {
    this.cancelInstaTimer();
    if (!this.state.instaEnabled || !this.instaAllowed()) {
      if (this.state.instaEnabled && this.state.open) {
        this.renderInstaMessage("INSTA unavailable for live capability documents");
      }
      return;
    }
    if (!editorSourceComplete(this.editor.value)) {
      this.renderInstaMessage("Waiting for a complete form…");
      return;
    }
    this.instaTimer = setTimeout(() => this.runInsta(), immediate ? 0 : INSTA_DELAY_MS);
  }

  cancelInstaTimer() {
    if (this.instaTimer) clearTimeout(this.instaTimer);
    this.instaTimer = null;
  }

  async runInsta() {
    if (!this.state.instaEnabled || !this.instaAllowed()) return;
    const forms = editorTopLevelForms(this.editor.value);
    const requestGeneration = ++this.state.previewGeneration;
    const previous = this.state.preview;
    this.renderInstaPending(forms);
    try {
      const preview = await this.broker.previewDocument(
        this.state.kernel,
        this.activeDocumentId(),
        forms,
        { bootstrap: defaultBootstrap(this.state.space ?? "home") }
      );
      if (requestGeneration !== this.state.previewGeneration || !this.state.instaEnabled) {
        await this.broker.disposePreview(preview.generationId);
        return;
      }
      this.state.preview = preview;
      if (previous) await this.broker.disposePreview(previous.generationId);
      this.renderInstaRows(preview);
    } catch (error) {
      if (requestGeneration === this.state.previewGeneration) {
        this.renderInstaMessage(String(error?.message ?? error));
      }
    }
  }

  renderInstaPending(forms) {
    this.instaGutter.replaceChildren();
    for (const form of forms) this.instaGutter.appendChild(this.instaRow(form, "pending", "…"));
  }

  renderInstaRows(preview) {
    this.instaGutter.replaceChildren();
    for (const row of preview.rows) {
      const label = row.status === "ok"
        ? `${row.valueType ? `${row.valueType} · ` : ""}${row.value ?? "nil"}`
        : row.status === "error" ? row.error : "not evaluated";
      const item = this.instaRow(row, row.status, label);
      if (row.traceId) item.addEventListener("click", () => {
        const trace = this.broker.getPreviewTrace(preview.generationId, row.traceId);
        this.renderTrace(trace, row);
      });
      this.instaGutter.appendChild(item);
    }
  }

  instaRow(form, status, label) {
    const row = action(label, "Open this form's trace");
    row.classList.add("hara-studio-insta-result", `is-${status}`);
    row.style.top = `${sourceLine(this.editor.value, form.start) * editorLineHeight(this.editor)}px`;
    return row;
  }

  markInstaStale() {
    for (const row of this.instaGutter.querySelectorAll(".hara-studio-insta-result")) {
      row.classList.add("is-stale");
    }
  }

  renderInstaMessage(message) {
    this.instaGutter.replaceChildren(el("div", "hara-studio-insta-message", message));
  }

  async clearInsta() {
    this.cancelInstaTimer();
    this.state.previewGeneration += 1;
    const preview = this.state.preview;
    this.state.preview = null;
    this.instaGutter.replaceChildren();
    if (preview) await this.broker.disposePreview(preview.generationId);
  }

  activeDocumentId() {
    return studioDocumentId({
      projectId: this.activeProject?.id ?? "document",
      space: this.state.space,
      path: this.state.open
    });
  }

  async runForm() {
    if (!this.state.open) return;
    const form = editorFormAt(
      this.editor.value,
      this.editor.selectionStart,
      this.editor.selectionEnd
    );
    if (!form?.source.trim()) return;
    await this.task(async () => {
      const documentId = this.activeDocumentId();
      const value = this.broker.hasDocument(this.state.kernel, documentId)
        ? await this.broker.evalForm(this.state.kernel, documentId, form.source)
        : await this.broker.eval(this.state.kernel, form.source);
      this.logValue(value);
      return value;
    });
  }

  async newFile() {
    if (!this.state.space) return;
    const input = await this.askInput(`New file in ${this.state.space}`, "File path", "/scratch.hal");
    if (!input) return;
    const path = normalizeNewFilePath(input);
    if (!path) {
      this.logError(new Error(`invalid file path: ${input.trim()}`));
      return;
    }
    const ok = await this.task(() => this.writeText(path, defaultFileContent(path)));
    if (ok === undefined) return;
    // The file is created either way; only switch to it when any unsaved
    // edits in the currently open file may go (same guard as File/space
    // switching).
    if (!(await this.confirmDiscard())) {
      await this.refreshFiles();
      return;
    }
    this.state.dirty = false;
    await this.refreshFiles();
    await this.openFile(path);
  }

  clearEditor() {
    this.clearInsta();
    this.state.open = null;
    this.state.dirty = false;
    this.editor.value = "";
    this.editor.disabled = true;
    this.clearTrace();
    this.renderEditorHead();
  }

  async confirmDiscard() {
    if (!this.state.dirty) return true;
    return this.askConfirm("Discard unsaved changes?", "The editor contents have not been saved.");
  }

  // ---------------------------------------------------------------- render

  renderEditorHead() {
    this.editorName.textContent = this.state.open ?? "—";
    this.dirtyFlag.textContent = this.state.dirty ? "● " : "";
  }

  renderStatus() {
    const state = this.state.busy > 0 ? "Busy" : this.state.failed ? "Error" : "Idle";
    this.chrome.update({
      project: this.activeProject?.title ?? "Choose project",
      runtime: this.state.runtime,
      kernel: this.state.kernel,
      space: this.state.space ?? "—",
      files: this.state.files.length,
      state,
      consoleOpen: this.main.classList.contains("is-console-open"),
      outputAvailable: !this.canvasPanel.hidden,
      mobileView: this.state.mobileView ?? "source"
    });
    this.treeCount.textContent = String(this.state.files.length);
  }

  // ----------------------------------------------------------------- handle

  async refresh() {
    await this.refreshFiles();
    this.refreshKernelSelect();
    this.renderStatus();
  }

  unmount() {
    this.clearInsta();
    this.chrome.destroy();
  }
}

// ------------------------------------------------------------------ helpers

function el(tag, className, content) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (content !== undefined) node.textContent = content;
  return node;
}

function text(content) {
  return document.createTextNode(content);
}

function label(content) {
  return el("span", "hara-studio-step hara-studio-label", content);
}

function stepAction(content, title) {
  const node = el("button", "hara-studio-step hara-studio-action", content);
  node.type = "button";
  if (title) node.setAttribute("title", title);
  return node;
}

function action(content, title) {
  const node = el("button", "hara-studio-action", content);
  node.type = "button";
  if (title) node.setAttribute("title", title);
  return node;
}

function strip(name, valueNode) {
  const span = el("span");
  span.append(text(`${name} `), valueNode);
  return span;
}

function safeStorageGet(key) {
  try { return globalThis.localStorage?.getItem(key) ?? null; } catch { return null; }
}

function safeStorageSet(key, value) {
  try { globalThis.localStorage?.setItem(key, value); } catch {}
}

function sourceLine(source, offset) {
  return source.slice(0, Math.max(0, offset)).split("\n").length - 1;
}

function editorLineHeight(editor) {
  const value = Number.parseFloat(globalThis.getComputedStyle?.(editor)?.lineHeight);
  return Number.isFinite(value) ? value : 18;
}

function traceKindGroup(kind) {
  if (kind.startsWith("operation-")) return "operation";
  if (kind === "macro-expand") return "macro";
  return "error";
}

function traceEventLabel(node) {
  const event = node.event;
  const kind = traceName(traceField(event, "kind"));
  const fn = traceField(event, "function");
  if (kind === "operation-enter") {
    const returned = node.returnEvent
      ? traceField((traceField(node.returnEvent, "values") ?? [])[0], "display")
      : null;
    return `${fn ?? "<anonymous>"}${returned == null ? "" : ` → ${returned}`}`;
  }
  if (kind === "macro-expand") return `MACRO ${fn ?? ""}`;
  return traceField(event, "message") ?? kind.replaceAll("-", " ").toUpperCase();
}

function formatTraceEvent(event) {
  return JSON.stringify(traceToPlain(event), null, 2);
}

function traceToPlain(value) {
  if (value instanceof Map) {
    return Object.fromEntries([...value].map(([key, item]) => [
      String(key?.name ?? key), traceToPlain(item)
    ]));
  }
  if (Array.isArray(value)) return value.map(traceToPlain);
  if (value instanceof Set) return [...value].map(traceToPlain);
  if (value?.name && value.constructor?.name?.startsWith("Hta")) return value.name;
  return value;
}
