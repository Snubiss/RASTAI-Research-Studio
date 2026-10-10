"""
Static-shape greedy decoding, optionally replayed as a CUDA graph (spec v5 §17 performance, §9 live panel).

A Python decode loop for a 1B model is limited by kernel-launch overhead, not by the GPU. Capturing one
decode step as a CUDA graph and replaying it removes that overhead. That gives a fair baseline for
measuring what the live panel costs.

The step keeps its state on the GPU: it reads the token and position buffers, writes the chosen token back
into the token buffer and advances the position. A replay therefore needs no CPU-to-GPU copies at all, and
the CPU's only job per token is one copy back of the packet (chosen token + panel readouts).
"""

import torch

from rastai_spike.gemma3 import CAPTURE_LAST_TOKEN_ATTENTION, Gemma3Model, KeyValueCache
from rastai_spike.lenses import LensCollector, naive_packet, packet

LENS_OFF = "off"
LENS_NAIVE = "naive"  # one unembedding per layer, a CPU copy per readout
LENS_BATCHED = "batched"  # the CTO rules: one batched unembedding, top-k on the GPU, one copy per token
LENS_FUSED = "fused"  # batched, and the lens's last row doubles as the model's output logits
LENS_MODES = (LENS_OFF, LENS_NAIVE, LENS_BATCHED, LENS_FUSED)
WARMUP_RUNS_BEFORE_CAPTURE = 2


class StaticDecoder:
    def __init__(self, model: Gemma3Model, max_length: int, lens_mode: str = LENS_OFF, autoencoders=(), use_cuda_graph: bool = False):
        if lens_mode not in LENS_MODES:
            raise ValueError(f"unknown lens mode {lens_mode!r}")
        if use_cuda_graph and lens_mode == LENS_NAIVE:
            raise ValueError("the naive lens copies to the CPU mid-step, so it can't be captured in a CUDA graph")

        weight = model.token_embedding.weight
        self.model = model
        self.device = weight.device
        self.lens_mode = lens_mode
        self.autoencoders = list(autoencoders)
        self.use_cuda_graph = use_cuda_graph

        self.cache = KeyValueCache(model.config, 1, max_length, self.device, weight.dtype)
        self.token_buffer = torch.zeros((1, 1), dtype=torch.long, device=self.device)
        self.position_buffer = torch.zeros((1,), dtype=torch.long, device=self.device)
        self.graph: torch.cuda.CUDAGraph | None = None
        self.graph_output: torch.Tensor | None = None
        self.host_packet: torch.Tensor | None = None

    @torch.inference_mode()
    def prefill(self, prompt_ids: list[int]) -> int:
        """Run the prompt through the dynamic path and return the first generated token."""
        prompt = torch.tensor([prompt_ids], device=self.device)
        logits = self.model(prompt, self.cache, last_token_logits_only=True)
        first_token = logits[0, -1].argmax()
        self.token_buffer.copy_(first_token.view(1, 1))
        self.position_buffer.fill_(len(prompt_ids))
        return int(first_token)

    def step_on_gpu(self) -> torch.Tensor:
        """One decode step with no CPU synchronisation (except in naive mode).
        Returns [chosen token, *panel readouts] as one float32 tensor on the GPU."""
        model = self.model
        if self.lens_mode == LENS_OFF:
            logits = model(self.token_buffer, self.cache, decode_position=self.position_buffer)[0, -1]
            readouts = None
        else:
            collector = LensCollector(model.config.layer_count)
            fused = self.lens_mode == LENS_FUSED
            output = model(
                self.token_buffer,
                self.cache,
                observer=collector,
                attention_capture=CAPTURE_LAST_TOKEN_ATTENTION,
                decode_position=self.position_buffer,
                unembed_output=not fused,
            )
            if self.lens_mode == LENS_NAIVE:
                logits = output[0, -1]
                naive_packet(model, collector, self.autoencoders, logits)
                readouts = None
            else:
                readouts, logits = packet(model, collector, self.autoencoders, None if fused else output[0, -1])

        chosen = logits.argmax()
        self.token_buffer.copy_(chosen.view(1, 1))
        self.position_buffer.add_(1)

        chosen_as_float = chosen.view(1).float()  # token IDs (< 2^24) are exact in float32
        return chosen_as_float if readouts is None else torch.cat([chosen_as_float, readouts])

    def capture(self) -> None:
        """Warm up on a side stream (cuBLAS and friends allocate on first use), then record one step."""
        saved_token, saved_position = self.token_buffer.clone(), self.position_buffer.clone()
        side_stream = torch.cuda.Stream(self.device)
        side_stream.wait_stream(torch.cuda.current_stream(self.device))
        with torch.cuda.stream(side_stream):
            for _ in range(WARMUP_RUNS_BEFORE_CAPTURE):
                self.step_on_gpu()
                # Undo the step's bookkeeping; the cache slot it wrote is rewritten with the same values later.
                self.token_buffer.copy_(saved_token)
                self.position_buffer.copy_(saved_position)
        torch.cuda.current_stream(self.device).wait_stream(side_stream)

        # Capture on a stream of *this* device. torch.cuda.graph's default capture stream is created once,
        # on whichever GPU was current at first use, so with two GPUs it would record on the wrong one.
        capture_stream = torch.cuda.Stream(self.device)
        self.graph = torch.cuda.CUDAGraph()
        with torch.cuda.graph(self.graph, stream=capture_stream):
            self.graph_output = self.step_on_gpu()

    @torch.inference_mode()
    def step(self) -> torch.Tensor:
        """Generate one token. Returns the packet in pinned CPU memory (reused: copy it to keep it)."""
        with torch.cuda.device(self.device):
            if self.use_cuda_graph:
                if self.graph is None:
                    self.capture()
                self.graph.replay()
                device_packet = self.graph_output
            else:
                device_packet = self.step_on_gpu()

            if self.host_packet is None:
                self.host_packet = torch.empty(device_packet.shape, dtype=torch.float32, pin_memory=True)
            self.host_packet.copy_(device_packet, non_blocking=True)
            torch.cuda.current_stream(self.device).synchronize()
        return self.host_packet
