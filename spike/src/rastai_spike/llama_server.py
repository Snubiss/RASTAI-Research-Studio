"""
A minimal adapter for llama.cpp's llama-server, the Fast runtime (spec v5 §8, §17 and §22).

Rastai manages its own llama-server process: it starts it on a loopback port, waits until it is healthy,
talks to it over HTTP, and stops it. The gateway owns tokenization, so prompts are sent as token IDs,
never as text. This module also exposes llama-server's own tokenizer, so the two can be checked against each other.
"""

import json
import os
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path

SPIKE_FOLDER = Path(__file__).resolve().parents[2]
LLAMA_CPP_FOLDER = SPIKE_FOLDER / "vendor" / "llama.cpp-b11537"
SERVER_BINARY = LLAMA_CPP_FOLDER / "llama-server"
BENCH_BINARY = LLAMA_CPP_FOLDER / "llama-bench"
HOST = "127.0.0.1"  # loopback only (spec §21): never listen on the network
STARTUP_TIMEOUT_SECONDS = 120
HEALTH_POLL_SECONDS = 0.5
REQUEST_TIMEOUT_SECONDS = 300
ALL_LAYERS_ON_GPU = "999"


def library_environment() -> dict:
    """The release bundles its CUDA runtime libraries next to the binaries."""
    environment = dict(os.environ)
    environment["LD_LIBRARY_PATH"] = f"{LLAMA_CPP_FOLDER}:{environment.get('LD_LIBRARY_PATH', '')}"
    return environment


class LlamaServer:
    """Use as a context manager: the server process is always stopped on exit."""

    def __init__(self, model_path: Path, device: str, port: int, context_size: int = 4096):
        self.model_path = model_path
        self.device = device  # llama.cpp's name, e.g. "CUDA0"
        self.port = port
        self.context_size = context_size
        self.process: subprocess.Popen | None = None

    def __enter__(self):
        command = [
            str(SERVER_BINARY),
            "--model", str(self.model_path),
            "--host", HOST,
            "--port", str(self.port),
            "--device", self.device,
            "--n-gpu-layers", ALL_LAYERS_ON_GPU,
            "--ctx-size", str(self.context_size),
            "--parallel", "1",
        ]
        self.process = subprocess.Popen(command, env=library_environment(), stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        self.wait_until_healthy()
        return self

    def __exit__(self, *exception_info):
        if self.process is not None:
            self.process.terminate()
            try:
                self.process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.process.kill()

    def wait_until_healthy(self) -> None:
        deadline = time.monotonic() + STARTUP_TIMEOUT_SECONDS
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                error_output = self.process.stderr.read().decode(errors="replace")[-2000:]
                raise RuntimeError(f"llama-server exited during startup:\n{error_output}")
            try:
                if self.get("/health").get("status") == "ok":
                    return
            except (urllib.error.URLError, ConnectionError, json.JSONDecodeError):
                pass
            time.sleep(HEALTH_POLL_SECONDS)
        raise TimeoutError(f"llama-server was not healthy after {STARTUP_TIMEOUT_SECONDS} s")

    # ---- HTTP -------------------------------------------------------------------------------

    def url(self, path: str) -> str:
        return f"http://{HOST}:{self.port}{path}"

    def get(self, path: str) -> dict:
        with urllib.request.urlopen(self.url(path), timeout=REQUEST_TIMEOUT_SECONDS) as response:
            return json.loads(response.read())

    def post(self, path: str, body: dict) -> dict:
        request = urllib.request.Request(
            self.url(path), data=json.dumps(body).encode(), headers={"Content-Type": "application/json"}
        )
        with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:
            return json.loads(response.read())

    # ---- What the gateway uses -----------------------------------------------------------------

    def tokenize(self, text: str, add_special: bool, parse_special: bool = True) -> list[int]:
        body = {"content": text, "add_special": add_special, "parse_special": parse_special}
        return self.post("/tokenize", body)["tokens"]

    def detokenize(self, token_ids: list[int]) -> str:
        return self.post("/detokenize", {"tokens": token_ids})["content"]

    def complete_greedy(self, prompt_ids: list[int], max_new_tokens: int, ignore_end_of_turn: bool = False) -> dict:
        """Greedy generation from token IDs. Returns the generated IDs and llama-server's own timings."""
        body = {
            "prompt": prompt_ids,
            "n_predict": max_new_tokens,
            "temperature": 0.0,  # greedy: always the most likely token
            "top_k": 1,
            "return_tokens": True,
            "cache_prompt": False,
            "ignore_eos": ignore_end_of_turn,
        }
        response = self.post("/completion", body)
        return {"generated_ids": response["tokens"], "timings": response.get("timings", {})}
