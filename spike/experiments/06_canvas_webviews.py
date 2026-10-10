"""
Experiment 06: the in-house WebGL2 canvas in Chrome and in WebKitGTK (spec v5 §32, spike item 6).

Serves canvas/canvas_test.html on a loopback port, opens it in each browser, and collects the frame-time
report the page posts back when it's done. The page sweeps the Designer graph (1k–50k nodes, all on screen,
every node's colour changing each frame), Psychology's 3D flow view (26 × 512 up to 26 × 8192 cubes, values
re-uploaded each frame), and both together. The spec's target: 1,000 visible nodes at 60 fps in both engines.

Browsers:
  - chrome: Chrome for Testing, pinned in vendor/;
  - webkitgtk: the same WebKitGTK webview Tauri uses on Linux, opened by webview-harness (built on wry/tao,
    Tauri's own webview crates), once as-is and once with WEBKIT_DISABLE_DMABUF_RENDERER=1, the common
    workaround for NVIDIA on Wayland.

Windows open on screen while this runs (about a minute each). Don't cover them: browsers slow down
animation in hidden windows.

Run:  uv run python experiments/06_canvas_webviews.py
"""

import json
import os
import shutil
import subprocess
import threading
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from rastai_spike.reporting import environment_fingerprint, write_results

EXPERIMENT_NAME = "06_canvas_webviews"
SPIKE_FOLDER = Path(__file__).resolve().parents[1]
CANVAS_FOLDER = SPIKE_FOLDER / "canvas"
SCRATCH_FOLDER = SPIKE_FOLDER / ".scratch" / EXPERIMENT_NAME
CHROME_BINARY = SPIKE_FOLDER / "vendor" / "chrome-for-testing-155.0.8059.39" / "chrome-linux64" / "chrome"
WEBVIEW_HARNESS_BINARY = SPIKE_FOLDER / "webview-harness" / "target" / "release" / "webview-harness"
HOST = "127.0.0.1"
PORT = 18090
WINDOW_WIDTH = 1600
WINDOW_HEIGHT = 1000
MEASURE_MILLISECONDS = 3000
BROWSER_TIMEOUT_SECONDS = 300
TARGET_NODES = 1000
TARGET_FPS = 60


# ----------------------------------------------------------------------------------------------
# A tiny server: static files, plus POST /results from the page
# ----------------------------------------------------------------------------------------------

class CanvasRequestHandler(SimpleHTTPRequestHandler):
    def __init__(self, *arguments, inbox: dict, **keyword_arguments):
        self.inbox = inbox
        super().__init__(*arguments, **keyword_arguments)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        self.inbox["report"] = json.loads(self.rfile.read(length))
        self.inbox["arrived"].set()
        self.send_response(204)
        self.end_headers()

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")  # always the current page, never a cached one
        super().end_headers()

    def log_message(self, *arguments):
        pass


def start_server(inbox: dict) -> ThreadingHTTPServer:
    handler = partial(CanvasRequestHandler, inbox=inbox, directory=str(CANVAS_FOLDER))
    server = ThreadingHTTPServer((HOST, PORT), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


# ----------------------------------------------------------------------------------------------
# Browsers
# ----------------------------------------------------------------------------------------------

def page_url() -> str:
    return f"http://{HOST}:{PORT}/canvas_test.html?measure={MEASURE_MILLISECONDS}"


def chrome_command() -> list[str]:
    profile = SCRATCH_FOLDER / "chrome-profile"
    shutil.rmtree(profile, ignore_errors=True)
    return [
        str(CHROME_BINARY),
        f"--user-data-dir={profile}",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-timer-throttling",
        "--disable-renderer-backgrounding",
        "--disable-backgrounding-occluded-windows",
        f"--window-size={WINDOW_WIDTH},{WINDOW_HEIGHT}",
        f"--app={page_url()}",
    ]


def webkitgtk_command() -> list[str]:
    return [str(WEBVIEW_HARNESS_BINARY), page_url(), str(WINDOW_WIDTH), str(WINDOW_HEIGHT)]


BROWSERS = {  # name: (binary that must exist, command builder, extra environment)
    "chrome": (CHROME_BINARY, chrome_command, {}),
    "webkitgtk": (WEBVIEW_HARNESS_BINARY, webkitgtk_command, {}),
    "webkitgtk, DMABUF renderer off": (WEBVIEW_HARNESS_BINARY, webkitgtk_command, {"WEBKIT_DISABLE_DMABUF_RENDERER": "1"}),
}


def run_browser(name: str, inbox: dict) -> dict:
    binary, build_command, extra_environment = BROWSERS[name]
    if not binary.exists():
        return {"not_run": f"{binary.relative_to(SPIKE_FOLDER)} is missing"}

    print(f"[{EXPERIMENT_NAME}] {name}", flush=True)
    inbox["arrived"].clear()
    inbox.pop("report", None)
    process = subprocess.Popen(build_command(), env=dict(os.environ, **extra_environment),
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        if not inbox["arrived"].wait(BROWSER_TIMEOUT_SECONDS):
            return {"error": f"no report within {BROWSER_TIMEOUT_SECONDS} s (window hidden, blank, or crashed)"}
        return inbox["report"]
    finally:
        process.terminate()
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()


# ----------------------------------------------------------------------------------------------
# Report
# ----------------------------------------------------------------------------------------------

def target_met(report: dict) -> bool | None:
    for row in report.get("results", []):
        if row["scene"] == "graph" and row["nodes"] == TARGET_NODES:
            return row["fps"] >= TARGET_FPS * 0.97  # rounding tolerance on a 60 Hz cadence
    return None


def largest_graph_at_target(report: dict) -> int | None:
    passing = [row["nodes"] for row in report.get("results", []) if row["scene"] == "graph" and row["fps"] >= TARGET_FPS * 0.97]
    return max(passing) if passing else None


def build_markdown(results: dict) -> str:
    lines = [
        "# Experiment 06: in-house WebGL2 canvas in Chrome and WebKitGTK",
        "",
        f"Window {WINDOW_WIDTH}×{WINDOW_HEIGHT}, {MEASURE_MILLISECONDS} ms per configuration after a warm-up. "
        f"Target: {TARGET_NODES:,} visible nodes at {TARGET_FPS} fps. Frame rates are capped by the display's refresh rate.",
        "",
    ]
    for name, report in results["browsers"].items():
        lines += [f"## {name}", ""]
        if "not_run" in report or "error" in report:
            lines += [f"**Not measured:** {report.get('not_run') or report.get('error')}", ""]
            continue
        lines += [
            f"- {report['renderer']} ({report['vendor']}); {report['version']}",
            f"- Refresh {report['refresh_rate_hz']:.0f} Hz, canvas {report['canvas_size'][0]}×{report['canvas_size'][1]}, device pixel ratio {report['device_pixel_ratio']}",
            f"- **Target met: {target_met(report)}**; largest graph still at {TARGET_FPS} fps: {largest_graph_at_target(report)} nodes",
            f"- Headroom: the combined scene ({TARGET_NODES:,} nodes + 26×512 flow) still runs at the refresh cap when drawn "
            f"**{report['headroom']['largest_repeat_at_cap']}×** per frame "
            f"({', '.join(f'{row['repeats']}×: {row['fps']:.0f} fps' for row in report['headroom']['rows'])})",
            "",
            "| Scene | Instances | fps | Frame ms p50 / p95 / p99 | Script ms p50 / p95 | Pixels drawn |",
            "|---|---|---|---|---|---|",
        ]
        for row in report["results"]:
            instances = ", ".join(f"{row[key]:,} {key}" for key in ("nodes", "edges", "glyphs", "cubes") if key in row)
            lines.append(
                f"| {row['label']} | {instances} | {row['fps']:.0f} | {row['frame_ms_p50']:.1f} / {row['frame_ms_p95']:.1f} / {row['frame_ms_p99']:.1f} | "
                f"{row['script_ms_p50']:.2f} / {row['script_ms_p95']:.2f} | {row['drawn_pixel_fraction']:.1%} |"
            )
        lines.append("")
    lines += ["## Environment", "", "```", *[f"{key}: {value}" for key, value in results["environment"].items()], "```", ""]
    return "\n".join(lines)


def main() -> None:
    SCRATCH_FOLDER.mkdir(parents=True, exist_ok=True)
    inbox = {"arrived": threading.Event()}
    server = start_server(inbox)
    try:
        results = {"browsers": {name: run_browser(name, inbox) for name in BROWSERS}}
    finally:
        server.shutdown()
        shutil.rmtree(SCRATCH_FOLDER, ignore_errors=True)
    results["environment"] = environment_fingerprint()

    markdown = build_markdown(results)
    json_path, markdown_path = write_results(EXPERIMENT_NAME, results, markdown)
    print(markdown)
    print(f"Wrote {json_path} and {markdown_path}")


if __name__ == "__main__":
    main()
